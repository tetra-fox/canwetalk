//! the whole effect: the input's pitch is detected, smear works on the input, a voice for each
//! held note resynthesizes it at that note's pitch, or one voice at the input's own pitch
//! while none is held, and detune and stereo work on their sum
//!
//! frequencies are in cycles per two samples, so 1 is the Nyquist frequency and a period of
//! P samples is 2 / P

use std::f64::consts::TAU;

use crate::effects::{Amounts, Detune, Smear, Stereo};
use crate::envelope::Adsr;
use crate::grains::Shape;
use crate::marks::{Cycle, Marks};
use crate::notes::{Notes, Play};
use crate::pitch::{Detector, Range};
use crate::ring::Ring;
use crate::voice::Expression;

/// seconds of input the voices can read back over. the longest grains are cut to fit
const HISTORY_SECONDS: f32 = 4.5;
/// the range the detected pitch is kept in, periods of 6400 down to 4 samples
const INPUT_PITCH: std::ops::RangeInclusive<f64> = 3.125e-4..=0.5;

/// a slow rise and fall of every voice's pitch
#[derive(Clone, Copy, Debug)]
pub(crate) struct Vibrato {
  /// semitones the pitch moves, either way
  pub(crate) depth: f64,
  /// cycles the vibrato moves on each sample
  pub(crate) step: f64,
  /// where in its cycle the vibrato is, when something outside sets that. otherwise it runs
  /// on by `step`
  pub(crate) phase: Option<f64>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Settings {
  pub(crate) shape: Shape,
  pub(crate) vibrato: Vibrato,
  pub(crate) play: Play,
  /// how notes rise and fall in the gate modes
  pub(crate) envelope: Adsr,
  pub(crate) expression: Expression,
  pub(crate) effects: Amounts,
  pub(crate) detect: Range,
  /// grains start where the input's cycles do, found from its waveform. otherwise where a
  /// count by the detected pitch puts them, which wanders off the cycles as the pitch moves
  pub(crate) anchor: bool,
}

pub(crate) struct Shifter {
  sample_rate: f64,
  detector: Detector,
  /// the last detected pitch, held while the input is unvoiced
  input_pitch: f64,
  smear: Smear,
  /// what the voices read, the input after smear
  input: [Ring; 2],
  /// where the cycles of `input` start
  marks: Marks,
  pub(crate) notes: Notes,
  /// where in its cycle the vibrato is, 0 to 1
  vibrato_phase: f64,
  detune: Detune,
  stereo: Stereo,
}

/// the pitch assumed until one is detected, the lowest of the detector's range
fn lowest_pitch(detector: &Detector, sample_rate: f32) -> f64 {
  let lowest = 2.0 / f64::from(detector.range().longest_period(sample_rate));
  // at the highest sample rates the bass range goes under what the input pitch is kept to
  lowest.clamp(*INPUT_PITCH.start(), *INPUT_PITCH.end())
}

impl Shifter {
  /// `range` is the detection range the host has set, which the pitch assumed until one is
  /// detected goes by
  pub(crate) fn new(sample_rate: f32, range: Range) -> Self {
    let detector = Detector::new(sample_rate, range);
    let input_pitch = lowest_pitch(&detector, sample_rate);
    Self {
      sample_rate: f64::from(sample_rate),
      detector,
      input_pitch,
      smear: Smear::new(sample_rate),
      input: std::array::from_fn(|_| Ring::new((sample_rate * HISTORY_SECONDS) as usize)),
      marks: Marks::new(2.0 / INPUT_PITCH.start()),
      notes: Notes::new(f64::from(sample_rate), input_pitch),
      vibrato_phase: 0.0,
      detune: Detune::new(sample_rate),
      stereo: Stereo::new(sample_rate),
    }
  }

  /// back to how `new` makes it with the range the detector has. does not allocate
  pub(crate) fn reset(&mut self) {
    // every field is named so that one added later cannot be left out
    let Self {
      sample_rate,
      detector,
      input_pitch,
      smear,
      input,
      marks,
      notes,
      vibrato_phase,
      detune,
      stereo,
    } = self;
    detector.reset();
    *input_pitch = lowest_pitch(detector, *sample_rate as f32);
    smear.reset();
    for line in input {
      line.clear();
    }
    marks.reset();
    notes.reset(*input_pitch);
    *vibrato_phase = 0.0;
    detune.reset();
    stereo.reset();
  }

  /// the most samples the output can go on for after the input stops: what smear holds,
  /// then what the voices read back over, then the delays of detune and stereo
  pub(crate) fn tail(&self) -> usize {
    self.smear.longest_delay()
      + self.input[0].len()
      + self.detune.longest_delay()
      + self.stereo.longest_delay()
  }

  /// the effect alone, with no latency against the input. the mode and latch of `settings`
  /// are given to `notes` beforehand, ahead of the sample's notes
  pub(crate) fn process(&mut self, frame: [f32; 2], settings: &Settings) -> [f32; 2] {
    if settings.detect != self.detector.range() {
      self.detector.set_range(settings.detect);
      self.input_pitch = lowest_pitch(&self.detector, self.sample_rate as f32);
    }
    if let Some(period) = self.detector.push(frame[0] + frame[1]) {
      self.input_pitch = (2.0 / f64::from(period)).clamp(*INPUT_PITCH.start(), *INPUT_PITCH.end());
    }

    let Amounts {
      detune,
      smear,
      stereo,
    } = settings.effects;
    let frame = if smear > 0.0 {
      self.smear.process(frame, 0.5 * self.input_pitch, smear)
    } else {
      self.smear.rest();
      frame
    };
    for (line, sample) in self.input.iter_mut().zip(frame) {
      line.push(sample);
    }

    self.vibrato_phase = match settings.vibrato.phase {
      Some(phase) => phase.rem_euclid(1.0),
      None => (self.vibrato_phase + settings.vibrato.step).fract(),
    };
    let vibrato = 2f64.powf(settings.vibrato.depth * (TAU * self.vibrato_phase).sin() / 12.0);

    // where the waveform does not repeat there are no cycles to mark, at the start of a word
    // or in a consonant, and the grains count the period off as they do unanchored
    let unmarked = Cycle {
      pitch: self.input_pitch,
      phase: None,
    };
    let cycle = match settings
      .anchor
      .then(|| self.marks.tick(&self.input, 2.0 / self.input_pitch))
    {
      Some(Some((period, phase))) => Cycle {
        pitch: (2.0 / period).clamp(*INPUT_PITCH.start(), *INPUT_PITCH.end()),
        phase: Some(phase),
      },
      _ => unmarked,
    };
    let out = self.notes.tick(cycle, vibrato, settings, &self.input);
    let out = self.detune.process(out, detune * detune, self.sample_rate);
    self.stereo.process(out, stereo, self.sample_rate)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::grains::UNITY_DRIFT;
  use crate::notes::{Mode, Note};
  use crate::signals::{FORMANT_HZ, VOWEL_HZ, vowel_sampled_at};

  const RATE: f32 = 48_000.0;
  const NEUTRAL: Settings = Settings {
    shape: Shape {
      pitch: 0.0,
      formant: 0.0,
      formant_follow: 0.0,
      smooth: 0.0,
    },
    vibrato: Vibrato {
      depth: 0.0,
      step: 0.0,
      phase: None,
    },
    play: Play {
      mode: Mode::Poly,
      latch: false,
      glide: 0.0,
      glide_notes_only: false,
    },
    envelope: Adsr {
      attack: 0.01,
      decay: 0.05,
      sustain: 0.5,
      release: 0.1,
    },
    expression: Expression {
      pressure_level: 0.0,
      slide_formant: 0.0,
    },
    effects: Amounts {
      detune: 0.0,
      smear: 0.0,
      stereo: 0.0,
    },
    detect: Range::Mid,
    anchor: false,
  };

  fn vowel_for(seconds: usize) -> Vec<f32> {
    vowel_at(seconds * RATE as usize, |_| VOWEL_HZ)
  }

  /// a vowel with the pitch `hz` gives for each sample
  fn vowel_at(samples: usize, hz: impl Fn(usize) -> f32) -> Vec<f32> {
    vowel_sampled_at(RATE, samples, hz)
  }

  fn vowel() -> Vec<f32> {
    vowel_for(1)
  }

  fn run(shifter: &mut Shifter, input: &[f32], settings: &Settings) -> Vec<f32> {
    input
      .iter()
      .map(|&x| {
        shifter.notes.set_play(&settings.play);
        shifter.process([x, x], settings)[0]
      })
      .collect()
  }

  /// every reading of the pitch the detector makes after the first `skip` samples
  fn pitch_readings(signal: &[f32], skip: usize) -> Vec<f32> {
    let mut detector = Detector::new(RATE, Range::Mid);
    // the detector takes the sum of two channels
    let readings = signal.iter().enumerate().filter_map(|(index, &x)| {
      detector
        .push(2.0 * x)
        .filter(|_| index >= skip)
        .map(|period| RATE / period)
    });
    readings.collect()
  }

  /// the middle one of the readings over the last quarter second. a single reading moves by
  /// half a percent with where the grains happen to fall
  fn pitch_hz(signal: &[f32]) -> f32 {
    let mut readings = pitch_readings(signal, signal.len() - RATE as usize / 4);
    readings.sort_by(f32::total_cmp);
    *readings
      .get(readings.len() / 2)
      .expect("the output has a pitch")
  }

  /// power at `hz` in the last 4096 samples, a window short enough that a small error in `hz`
  /// stays well inside the lobe
  fn power_at(signal: &[f32], hz: f32) -> f32 {
    let tail = &signal[signal.len() - 4096..];
    let (re, im) = tail
      .iter()
      .enumerate()
      .fold((0.0, 0.0), |(re, im), (i, &x)| {
        let turn = std::f32::consts::TAU;
        let hann = 0.5 - 0.5 * (turn * i as f32 / tail.len() as f32).cos();
        let angle = turn * (hz * i as f32 / RATE).fract();
        (re + hann * x * angle.cos(), im + hann * x * angle.sin())
      });
    re * re + im * im
  }

  /// frequency of the strongest harmonic of `fundamental` below 4 kHz. grains start at no
  /// particular point of the input period, which decides how a resonance between two
  /// harmonics is shared between them, so it stands for the resonance to within a harmonic
  fn strongest_harmonic_hz(signal: &[f32], fundamental: f32) -> f32 {
    (1..(4000.0 / fundamental) as usize)
      .map(|harmonic| harmonic as f32 * fundamental)
      .max_by(|&a, &b| power_at(signal, a).total_cmp(&power_at(signal, b)))
      .expect("the fundamental is below 2 kHz")
  }

  fn assert_near(actual: f32, expected: f32, tolerance: f32, what: &str) {
    assert!(
      (actual - expected).abs() <= tolerance,
      "{what}: {actual}, expected {expected}"
    );
  }

  /// at the input pitch the grains creep onto the input periods, UNITY_DRIFT octaves per
  /// period, and then give way to the input. that is at most one period's worth of creeping
  #[test]
  fn neutral_settings_settle_to_the_input_itself() {
    let periods = 1.0 / (2f32.powf(UNITY_DRIFT as f32) - 1.0);
    let input = vowel_for((periods / VOWEL_HZ).ceil() as usize + 2);
    for anchor in [false, true] {
      let settings = Settings { anchor, ..NEUTRAL };
      let output = run(&mut Shifter::new(RATE, NEUTRAL.detect), &input, &settings);
      let settled = input.len() - RATE as usize;
      let error = output[settled..]
        .iter()
        .zip(&input[settled..])
        .fold(0f32, |worst, (a, b)| worst.max((a - b).abs()));
      assert!(error < 1e-3, "error {error} with anchor {anchor}");
    }
  }

  /// a new shifter counts the input's period and its own off from the same sample, so its
  /// first grain is lined up and there is only the input to hear once the voice has faded in
  #[test]
  fn a_new_shifter_passes_the_input_on_at_every_sample_rate() {
    for rate in [44_100.0, 48_000.0, 88_200.0, 96_000.0, 192_000.0] {
      let input = vowel_sampled_at(rate, rate as usize / 2, |_| VOWEL_HZ);
      for anchor in [false, true] {
        let settings = Settings { anchor, ..NEUTRAL };
        let output = run(&mut Shifter::new(rate, settings.detect), &input, &settings);
        let settled = rate as usize / 100;
        let error = output[settled..]
          .iter()
          .zip(&input[settled..])
          .fold(0f32, |worst, (a, b)| worst.max((a - b).abs()));
        assert!(
          error < 1e-3,
          "error {error} at {rate} Hz with anchor {anchor}"
        );
      }
    }
  }

  #[test]
  fn pitch_moves_the_fundamental_and_keeps_the_formant() {
    let settings = Settings {
      shape: Shape {
        pitch: 7.0 / 12.0,
        ..NEUTRAL.shape
      },
      ..NEUTRAL
    };
    for anchor in [false, true] {
      let settings = Settings { anchor, ..settings };
      let output = run(&mut Shifter::new(RATE, NEUTRAL.detect), &vowel(), &settings);
      let expected = VOWEL_HZ * 2f32.powf(7.0 / 12.0);
      let fundamental = pitch_hz(&output);
      assert_near(fundamental, expected, 0.005 * expected, "fundamental");
      let formant = strongest_harmonic_hz(&output, fundamental);
      assert_near(formant, FORMANT_HZ, fundamental, "formant");
    }
  }

  #[test]
  fn formant_moves_the_resonance_and_keeps_the_fundamental() {
    let settings = Settings {
      shape: Shape {
        formant: 1.0,
        ..NEUTRAL.shape
      },
      ..NEUTRAL
    };
    for anchor in [false, true] {
      let settings = Settings { anchor, ..settings };
      let output = run(&mut Shifter::new(RATE, NEUTRAL.detect), &vowel(), &settings);
      let fundamental = pitch_hz(&output);
      assert_near(fundamental, VOWEL_HZ, 0.005 * VOWEL_HZ, "fundamental");
      let formant = strongest_harmonic_hz(&output, fundamental);
      assert_near(formant, 2.0 * FORMANT_HZ, fundamental, "formant");
    }
  }

  /// a note an octave above the input, with the formants following by a half and by all of
  /// it, puts the resonance half an octave and an octave up
  #[test]
  fn formant_follows_the_note_by_the_set_share() {
    for follow in [0.5, 1.0] {
      let settings = Settings {
        shape: Shape {
          formant_follow: f64::from(follow),
          ..NEUTRAL.shape
        },
        ..NEUTRAL
      };
      let mut shifter = shifter_in(&settings);
      shifter.notes.on(key(57));
      let output = run(&mut shifter, &vowel(), &settings);
      let formant = strongest_harmonic_hz(&output, A3);
      assert_near(formant, FORMANT_HZ * 2f32.powf(follow), A3, "formant");
    }
  }

  #[test]
  fn latch_holds_notes_until_the_next_key_after_all_are_up() {
    let latched = Settings {
      play: Play {
        latch: true,
        ..NEUTRAL.play
      },
      ..NEUTRAL
    };
    let mut shifter = shifter_in(&latched);
    shifter.notes.on(key(57));
    shifter.notes.off(key(57));
    let output = run(&mut shifter, &vowel(), &latched);
    assert_near(
      pitch_hz(&output),
      A3,
      0.005 * A3,
      "fundamental after the key came up",
    );

    shifter.notes.on(key(64));
    shifter.notes.off(key(64));
    let output = run(&mut shifter, &vowel(), &latched);
    assert_near(
      pitch_hz(&output),
      E4,
      0.005 * E4,
      "fundamental after the next key",
    );
    assert!(
      power_at(&output, E4) > 100.0 * power_at(&output, A3),
      "the first note still sounds"
    );

    let output = run(&mut shifter, &vowel(), &NEUTRAL);
    assert_near(
      pitch_hz(&output),
      VOWEL_HZ,
      0.55,
      "fundamental with the latch off",
    );
  }

  #[test]
  fn latch_adds_keys_pressed_while_one_is_down() {
    let latched = Settings {
      play: Play {
        latch: true,
        ..NEUTRAL.play
      },
      ..NEUTRAL
    };
    let mut shifter = shifter_in(&latched);
    shifter.notes.on(key(57));
    shifter.notes.on(key(64));
    shifter.notes.off(key(57));
    shifter.notes.off(key(64));
    let output = run(&mut shifter, &vowel(), &latched);
    let absent = power_at(&output, C4);
    assert!(power_at(&output, A3) > 100.0 * absent, "A3 is missing");
    assert!(power_at(&output, E4) > 100.0 * absent, "E4 is missing");
  }

  /// gated, there is silence without a note and after one's release has run out
  #[test]
  fn gate_sounds_only_notes() {
    let gated = Settings {
      play: Play {
        mode: Mode::PolyGate,
        ..NEUTRAL.play
      },
      ..NEUTRAL
    };
    let loudest = |signal: &[f32]| signal.iter().fold(0f32, |peak, x| peak.max(x.abs()));
    let mut shifter = shifter_in(&gated);
    assert_eq!(loudest(&run(&mut shifter, &vowel(), &gated)), 0.0);

    shifter.notes.on(key(57));
    let output = run(&mut shifter, &vowel(), &gated);
    assert_near(
      pitch_hz(&output),
      A3,
      0.005 * A3,
      "fundamental of the held note",
    );

    shifter.notes.off(key(57));
    let output = run(&mut shifter, &vowel(), &gated);
    let released = (gated.envelope.release * f64::from(RATE)) as usize;
    assert!(
      loudest(&output[..released / 2]) > 0.01,
      "the release was cut short"
    );
    assert_eq!(loudest(&output[released + 1..]), 0.0);
  }

  /// a note that starts from silence starts with its first grain, and none of the input as it
  /// is comes through ahead of that
  #[test]
  fn a_note_from_silence_starts_without_the_plain_input() {
    let gated = Settings {
      play: Play {
        mode: Mode::PolyGate,
        ..NEUTRAL.play
      },
      envelope: Adsr {
        attack: 0.0,
        ..NEUTRAL.envelope
      },
      ..NEUTRAL
    };
    let level =
      |signal: &[f32]| (signal.iter().map(|x| x * x).sum::<f32>() / signal.len() as f32).sqrt();
    let input = vowel();
    let (before, after) = input.split_at(input.len() / 2);
    let mut shifter = shifter_in(&gated);
    run(&mut shifter, before, &gated);
    shifter.notes.on(key(69));
    let output = run(&mut shifter, after, &gated);
    // a grain takes an input period to rise, and a millisecond is a ninth of that
    let start = RATE as usize / 1000;
    let (heard, plain) = (level(&output[..start]), level(after));
    assert!(
      heard < 0.1 * plain,
      "the first millisecond is at {heard}, the input at {plain}"
    );
  }

  /// a note let go fades at its own pitch, not the input's
  #[test]
  fn released_note_keeps_its_pitch() {
    let gated = Settings {
      play: Play {
        mode: Mode::PolyGate,
        ..NEUTRAL.play
      },
      envelope: Adsr {
        release: 2.0,
        ..NEUTRAL.envelope
      },
      ..NEUTRAL
    };
    let mut shifter = shifter_in(&gated);
    shifter.notes.on(key(64));
    run(&mut shifter, &vowel(), &gated);
    shifter.notes.off(key(64));
    let fading = &run(&mut shifter, &vowel(), &gated)[..8192];
    assert!(
      power_at(fading, E4) > 100.0 * power_at(fading, VOWEL_HZ),
      "the release went to the input pitch"
    );
  }

  /// held at the top and at the bottom of its cycle, the vibrato moves a note by its depth
  #[test]
  fn vibrato_moves_the_pitch_by_its_depth() {
    for (phase, semitones) in [(0.25, 2.0), (0.75, -2.0)] {
      let settings = Settings {
        vibrato: Vibrato {
          depth: 2.0,
          phase: Some(phase),
          ..NEUTRAL.vibrato
        },
        ..NEUTRAL
      };
      let mut shifter = shifter_in(&settings);
      shifter.notes.on(key(57));
      let output = run(&mut shifter, &vowel(), &settings);
      let expected = A3 * 2f32.powf(semitones / 12.0);
      assert_near(pitch_hz(&output), expected, 0.005 * expected, "fundamental");
    }
  }

  /// left to run, the vibrato goes through its cycle at the step it is given
  #[test]
  fn vibrato_runs_at_its_rate() {
    let settings = Settings {
      vibrato: Vibrato {
        depth: 1.0,
        step: 4.0 / f64::from(RATE),
        ..NEUTRAL.vibrato
      },
      ..NEUTRAL
    };
    let mut shifter = shifter_in(&settings);
    shifter.notes.on(key(57));
    run(&mut shifter, &vowel(), &settings);
    // the phase wraps at one, so just short of a whole cycle reads as nearly one
    let from_whole = shifter.vibrato_phase.min(1.0 - shifter.vibrato_phase);
    assert!(
      from_whole < 1e-3,
      "{from_whole} of a cycle from four whole ones"
    );
  }

  #[test]
  fn pressure_sets_the_level_of_its_own_note() {
    let settings = Settings {
      expression: Expression {
        pressure_level: 1.0,
        ..NEUTRAL.expression
      },
      ..NEUTRAL
    };
    let loudest = |signal: &[f32]| signal.iter().fold(0f32, |peak, x| peak.max(x.abs()));
    let mut shifter = shifter_in(&settings);
    shifter.notes.on(note(3, 57));
    shifter.notes.channel_pressure(5, 1.0);
    assert_eq!(loudest(&run(&mut shifter, &vowel(), &settings)), 0.0);

    shifter.notes.channel_pressure(3, 1.0);
    let full = loudest(&run(&mut shifter, &vowel(), &settings));
    shifter.notes.channel_pressure(3, 0.5);
    let half = loudest(&run(&mut shifter, &vowel(), &settings)[RATE as usize / 2..]);
    assert_near(half, 0.5 * full, 0.05 * full, "level at half pressure");
  }

  /// slid to the top of its travel, a note's formants are up by the whole of the setting
  #[test]
  fn slide_moves_the_formants_of_its_own_note() {
    let settings = Settings {
      expression: Expression {
        slide_formant: 1.0,
        ..NEUTRAL.expression
      },
      ..NEUTRAL
    };
    let mut shifter = shifter_in(&settings);
    let note = note(3, 57);
    shifter.notes.on(note);
    let output = run(&mut shifter, &vowel(), &settings);
    assert_near(
      strongest_harmonic_hz(&output, A3),
      FORMANT_HZ,
      A3,
      "formant at rest",
    );

    shifter.notes.note_slide(note, 1.0);
    let output = run(&mut shifter, &vowel(), &settings);
    assert_near(
      strongest_harmonic_hz(&output, A3),
      2.0 * FORMANT_HZ,
      A3,
      "formant slid up",
    );
  }

  /// after a reset the shifter gives what a new one gives, whatever it was doing before
  #[test]
  fn reset_is_a_fresh_start() {
    let settings = Settings {
      vibrato: Vibrato {
        depth: 0.5,
        step: 5.0 / f64::from(RATE),
        phase: None,
      },
      play: Play {
        glide: 0.6,
        ..NEUTRAL.play
      },
      effects: Amounts {
        detune: 0.3,
        smear: 0.2,
        stereo: 0.5,
      },
      // the pitch the voices glide from at the start goes by the range
      detect: Range::Bass,
      ..NEUTRAL
    };
    let play = |shifter: &mut Shifter| {
      shifter.notes.set_play(&settings.play);
      shifter.notes.on(key(57));
      run(shifter, &vowel(), &settings)
    };

    let mut used = shifter_in(&settings);
    used.notes.bend(0, 2.0);
    used.notes.on(note(3, 81));
    used.notes.channel_pressure(3, 0.7);
    used.notes.channel_slide(3, 0.9);
    run(&mut used, &vowel(), &settings);
    used.reset();

    assert!(
      play(&mut used) == play(&mut Shifter::new(RATE, settings.detect)),
      "a reset shifter differs from a new one"
    );
  }

  /// an effect that is turned off and on again goes on from what it is given then. with
  /// silence coming in, nothing it held when it was last on is heard
  #[test]
  fn an_effect_turned_on_again_holds_nothing_from_before() {
    let off = NEUTRAL.effects;
    let quiet = vec![0.0; 2 * RATE as usize];
    for effects in [
      Amounts { smear: 1.0, ..off },
      Amounts { detune: 1.0, ..off },
      Amounts { stereo: 1.0, ..off },
    ] {
      let on = Settings { effects, ..NEUTRAL };
      let mut shifter = Shifter::new(RATE, NEUTRAL.detect);
      run(&mut shifter, &vowel(), &on);
      run(&mut shifter, &quiet, &NEUTRAL);
      let output = run(&mut shifter, &quiet, &on);
      let loudest = output.iter().fold(0f32, |peak, x| peak.max(x.abs()));
      assert!(loudest < 1e-6, "a peak of {loudest} with {effects:?}");
    }
  }

  /// smear at full on an input with no pitch holds a stretch for the longest it can, here
  /// one taken up a quarter second before the input stops. once the tail has passed there
  /// is nothing left of it
  #[test]
  fn nothing_sounds_once_the_tail_has_passed() {
    let rate = 44_100.0;
    let settings = Settings {
      effects: Amounts {
        smear: 1.0,
        ..NEUTRAL.effects
      },
      detect: Range::Bass,
      ..NEUTRAL
    };
    let mut state = 0x2545_f491_u32;
    let noise = (0..(6.2 * rate) as usize).map(|_| {
      state ^= state << 13;
      state ^= state >> 17;
      state ^= state << 5;
      state as f32 / u32::MAX as f32 - 0.5
    });
    let mut shifter = Shifter::new(rate, settings.detect);
    run(&mut shifter, &noise.collect::<Vec<_>>(), &settings);
    let tail = vec![0.0; shifter.tail()];
    run(&mut shifter, &tail, &settings);
    let after = run(&mut shifter, &vec![0.0; rate as usize], &settings);
    let loudest = after.iter().fold(0f32, |peak, x| peak.max(x.abs()));
    assert!(loudest < 1e-6, "a peak of {loudest} after the tail");
  }

  fn note(channel: u8, key: u8) -> Note {
    Note::new(channel, key).expect("a channel under 16 and a key under 128")
  }

  /// a voice sliding fast under a held note. anchored, the note's pitch is off by a tenth of
  /// a semitone on average, where grains that only count the input's periods off are off by
  /// a third
  #[test]
  fn anchored_note_holds_its_pitch_under_a_sliding_voice() {
    // up and down two semitones either way, five times a second
    let slide = |index: usize| {
      let turn = (5.0 * index as f32 / RATE).fract();
      let semitones = 2.0 * (4.0 * (turn - 0.5).abs() - 1.0);
      VOWEL_HZ * 2.0 * 2f32.powf(semitones / 12.0)
    };
    let input = vowel_at(3 * RATE as usize, slide);
    let settings = Settings {
      anchor: true,
      ..NEUTRAL
    };
    let mut shifter = shifter_in(&settings);
    shifter.notes.on(key(57));
    let output = run(&mut shifter, &input, &settings);
    let readings = pitch_readings(&output, RATE as usize);
    let off = readings
      .iter()
      .map(|hz| (1200.0 * (hz / A3).log2()).abs())
      .sum::<f32>()
      / readings.len() as f32;
    assert!(off < 15.0, "{off} cents off on average");
  }

  /// the lowest note bent down two octaves, with the formants all the way up and the grains
  /// stretched to keep overlapping, makes the longest grains there are. every one still has
  /// to read inside what the input lines hold
  #[test]
  fn the_longest_grains_stay_inside_the_input_lines() {
    for anchor in [false, true] {
      let settings = Settings {
        shape: Shape {
          pitch: -2.0,
          formant: 4.0,
          smooth: 1.0,
          ..NEUTRAL.shape
        },
        anchor,
        ..NEUTRAL
      };
      let mut shifter = shifter_in(&settings);
      shifter.notes.on(key(0));
      run(&mut shifter, &vowel_for(8), &settings);
    }
  }

  /// at the highest sample rate the lowest pitch of the bass range has a longer period than
  /// the input pitch is allowed
  #[test]
  fn the_pitch_assumed_before_any_is_found_is_one_the_input_can_have() {
    let settings = Settings {
      detect: Range::Bass,
      ..NEUTRAL
    };
    let mut shifter = Shifter::new(192_000.0, Range::Mid);
    shifter.process([0.0; 2], &settings);
    assert!(
      INPUT_PITCH.contains(&shifter.input_pitch),
      "{} is out of range",
      shifter.input_pitch
    );
  }

  /// a host can send a bend, tuning, pressure or slide that is no number. the note goes on
  /// sounding as it was
  #[test]
  fn values_that_are_no_numbers_do_not_reach_a_note() {
    let settings = Settings {
      expression: Expression {
        pressure_level: 1.0,
        slide_formant: 1.0,
      },
      ..NEUTRAL
    };
    let mut shifter = shifter_in(&settings);
    let played = note(3, 57);
    shifter.notes.on(played);
    shifter.notes.channel_pressure(3, 1.0);
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
      shifter.notes.bend(3, bad);
      shifter.notes.bend(0, bad);
      shifter.notes.tune(played, bad);
      shifter.notes.channel_pressure(3, bad);
      shifter.notes.note_pressure(played, bad);
      shifter.notes.channel_slide(3, bad);
      shifter.notes.note_slide(played, bad);
    }
    let output = run(&mut shifter, &vowel(), &settings);
    assert!(
      output.iter().all(|sample| sample.is_finite()),
      "the output is no number"
    );
    assert_near(pitch_hz(&output), A3, 0.005 * A3, "fundamental");
  }

  fn key(key: u8) -> Note {
    note(0, key)
  }

  /// a bend on a note's own channel moves that note, and no note on another channel
  #[test]
  fn a_channel_bends_its_own_note() {
    let mut shifter = shifter_in(&NEUTRAL);
    let note = note(3, 57);
    shifter.notes.on(note);
    shifter.notes.bend(5, 7.0);
    let output = run(&mut shifter, &vowel(), &NEUTRAL);
    assert_near(
      pitch_hz(&output),
      A3,
      0.005 * A3,
      "fundamental with another channel bent",
    );

    shifter.notes.bend(3, 2.0);
    let output = run(&mut shifter, &vowel(), &NEUTRAL);
    let bent = A3 * 2f32.powf(2.0 / 12.0);
    assert_near(
      pitch_hz(&output),
      bent,
      0.005 * bent,
      "fundamental with its channel bent",
    );

    shifter.notes.bend(0, -2.0);
    let output = run(&mut shifter, &vowel(), &NEUTRAL);
    assert_near(
      pitch_hz(&output),
      A3,
      0.005 * A3,
      "fundamental with the first channel bent back",
    );
  }

  /// a shifter that takes notes in the mode of `settings`
  fn shifter_in(settings: &Settings) -> Shifter {
    let mut shifter = Shifter::new(RATE, settings.detect);
    shifter.notes.set_play(&settings.play);
    shifter.process([0.0; 2], settings);
    shifter
  }

  #[test]
  fn held_note_sets_the_pitch_until_released() {
    let mut shifter = shifter_in(&NEUTRAL);
    shifter.notes.on(key(57));
    let output = run(&mut shifter, &vowel(), &NEUTRAL);
    assert_near(pitch_hz(&output), 220.0, 1.1, "fundamental");
    shifter.notes.off(key(57));
    let output = run(&mut shifter, &vowel(), &NEUTRAL);
    assert_near(
      pitch_hz(&output),
      VOWEL_HZ,
      0.55,
      "fundamental after release",
    );
  }

  const A3: f32 = 220.0;
  const C4: f32 = 261.63;
  const E4: f32 = 329.63;

  #[test]
  fn poly_sounds_held_notes_together() {
    let mut shifter = shifter_in(&NEUTRAL);
    shifter.notes.on(key(57));
    shifter.notes.on(key(64));
    let output = run(&mut shifter, &vowel(), &NEUTRAL);
    let absent = power_at(&output, C4);
    assert!(power_at(&output, A3) > 100.0 * absent, "A3 is missing");
    assert!(power_at(&output, E4) > 100.0 * absent, "E4 is missing");

    shifter.notes.off(key(57));
    let output = run(&mut shifter, &vowel(), &NEUTRAL);
    assert_near(
      pitch_hz(&output),
      E4,
      0.005 * E4,
      "fundamental with one note left",
    );
  }

  #[test]
  fn mono_plays_the_latest_note_and_returns_to_the_one_still_held() {
    let settings = Settings {
      play: Play {
        mode: Mode::Mono,
        ..NEUTRAL.play
      },
      ..NEUTRAL
    };
    let mut shifter = shifter_in(&settings);
    shifter.notes.on(key(57));
    shifter.notes.on(key(64));
    let output = run(&mut shifter, &vowel(), &settings);
    assert_near(
      pitch_hz(&output),
      E4,
      0.005 * E4,
      "fundamental with both held",
    );
    shifter.notes.off(key(64));
    let output = run(&mut shifter, &vowel(), &settings);
    assert_near(
      pitch_hz(&output),
      A3,
      0.005 * A3,
      "fundamental with the first left",
    );
  }

  #[test]
  fn off_ignores_notes() {
    let settings = Settings {
      play: Play {
        mode: Mode::Off,
        ..NEUTRAL.play
      },
      ..NEUTRAL
    };
    let mut shifter = shifter_in(&settings);
    shifter.notes.on(key(57));
    let output = run(&mut shifter, &vowel(), &settings);
    assert_near(pitch_hz(&output), VOWEL_HZ, 0.55, "fundamental");
  }
}
