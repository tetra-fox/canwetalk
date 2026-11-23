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

const HISTORY_SAMPLES: usize = 200_000;
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
  2.0 / f64::from(detector.range().longest_period(sample_rate))
}

impl Shifter {
  pub(crate) fn new(sample_rate: f32) -> Self {
    let detector = Detector::new(sample_rate, Range::Mid);
    let input_pitch = lowest_pitch(&detector, sample_rate);
    Self {
      sample_rate: f64::from(sample_rate),
      detector,
      input_pitch,
      smear: Smear::new(sample_rate),
      input: [Ring::new(HISTORY_SAMPLES), Ring::new(HISTORY_SAMPLES)],
      marks: Marks::new(2.0 / INPUT_PITCH.start()),
      notes: Notes::new(f64::from(sample_rate), input_pitch),
      vibrato_phase: 0.0,
      detune: Detune::new(sample_rate),
      stereo: Stereo::new(sample_rate),
    }
  }

  /// back to how `new` makes it but for the detector's range, which is the host's setting.
  /// does not allocate
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
    let mut out = self.notes.tick(cycle, vibrato, settings, &self.input);
    if detune > 0.0 {
      out = self.detune.process(out, detune * detune, self.sample_rate);
    }
    if stereo > 0.0 {
      out = self.stereo.process(out, stereo, self.sample_rate);
    }
    out
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::grains::UNITY_DRIFT;
  use crate::notes::{Mode, Note};

  const RATE: f32 = 48_000.0;
  const VOWEL_HZ: f32 = 110.0;
  const FORMANT_HZ: f32 = 800.0;
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

  /// a pulse train through a low-pass, for the tilt of a voice's spectrum, and a resonance
  fn vowel_for(seconds: usize) -> Vec<f32> {
    vowel_at(seconds * RATE as usize, |_| VOWEL_HZ)
  }

  /// the same with the pitch `hz` gives for each sample
  fn vowel_at(samples: usize, hz: impl Fn(usize) -> f32) -> Vec<f32> {
    let radius = (-std::f32::consts::PI * 80.0 / RATE).exp();
    let feedback = 2.0 * radius * (std::f32::consts::TAU * FORMANT_HZ / RATE).cos();
    let tilt = (-std::f32::consts::TAU * VOWEL_HZ / RATE).exp();
    let (mut y1, mut y2, mut phase, mut source) = (0.0, 0.0, 0.0, 0.0);
    (0..samples)
      .map(|index| {
        phase += hz(index) / RATE;
        let pulse = if phase >= 1.0 {
          phase -= 1.0;
          3.0
        } else {
          0.0
        };
        source = pulse + tilt * (source - pulse);
        let y = source + feedback * y1 - radius * radius * y2;
        (y2, y1) = (y1, y);
        y
      })
      .collect()
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
      let output = run(&mut Shifter::new(RATE), &input, &settings);
      let settled = input.len() - RATE as usize;
      let error = output[settled..]
        .iter()
        .zip(&input[settled..])
        .fold(0f32, |worst, (a, b)| worst.max((a - b).abs()));
      assert!(error < 1e-3, "error {error} with anchor {anchor}");
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
      let output = run(&mut Shifter::new(RATE), &vowel(), &settings);
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
      let output = run(&mut Shifter::new(RATE), &vowel(), &settings);
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
      play(&mut used) == play(&mut Shifter::new(RATE)),
      "a reset shifter differs from a new one"
    );
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
    let mut shifter = Shifter::new(RATE);
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
