//! the whole effect: the input's pitch is detected, smear works on the input, a voice for each
//! held note resynthesizes it at that note's pitch, or one voice at the input's own pitch
//! while none is held, and detune and stereo work on their sum
//!
//! frequencies are in cycles per two samples, so 1 is the Nyquist frequency and a period of
//! P samples is 2 / P

use std::f64::consts::TAU;

use crate::effects::{Detune, Smear, Stereo};
use crate::envelope::Adsr;
use crate::notes::{Mode, Notes};
use crate::pitch::{Detector, Range};
use crate::ring::Ring;

const HISTORY_SAMPLES: usize = 200_000;
/// the range the detected pitch is kept in, periods of 6400 down to 4 samples
const INPUT_PITCH: std::ops::RangeInclusive<f64> = 3.125e-4..=0.5;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Settings {
  /// octaves
  pub(crate) pitch: f64,
  /// octaves
  pub(crate) formant: f64,
  /// octaves the formants move further for each octave a voice is from the input's pitch
  pub(crate) formant_follow: f64,
  /// 0 to 1, as are the rest unless they say otherwise
  pub(crate) detune: f64,
  pub(crate) smear: f64,
  pub(crate) stereo: f64,
  /// how far grains stretch to keep overlapping when the pitch is lowered
  pub(crate) smooth: f64,
  /// the pitch settles in about the cube of this in seconds
  pub(crate) glide: f64,
  /// glide between notes only, and follow the input pitch as it is
  pub(crate) glide_notes_only: bool,
  /// semitones the vibrato moves every voice's pitch, either way
  pub(crate) vibrato: f64,
  /// cycles the vibrato moves on each sample
  pub(crate) vibrato_step: f64,
  /// where in its cycle the vibrato is, when something outside sets that. otherwise it runs
  /// on by `vibrato_step`
  pub(crate) vibrato_phase: Option<f64>,
  pub(crate) mode: Mode,
  /// nothing sounds without a note, and notes rise and fall by `envelope`. otherwise a voice
  /// follows the input while no note is held
  pub(crate) gate: bool,
  pub(crate) envelope: Adsr,
  /// how much of a note's level its pressure sets, 0 for none of it and 1 for all
  pub(crate) pressure_level: f64,
  /// octaves a note's slide moves its formants at either end of its travel
  pub(crate) slide_formant: f64,
  /// notes sound on after their keys come up, until the next key pressed with none down
  pub(crate) latch: bool,
  pub(crate) detect: Range,
}

pub(crate) struct Shifter {
  sample_rate: f64,
  detector: Detector,
  /// the last detected pitch, held while the input is unvoiced
  input_pitch: f64,
  smear: Smear,
  /// what the voices read, the input after smear
  input: [Ring; 2],
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
      notes: Notes::new(f64::from(sample_rate), input_pitch),
      vibrato_phase: 0.0,
      detune: Detune::new(sample_rate),
      stereo: Stereo::new(sample_rate),
    }
  }

  /// does not allocate
  pub(crate) fn reset(&mut self) {
    self.detector.reset();
    self.input_pitch = lowest_pitch(&self.detector, self.sample_rate as f32);
    self.smear.reset();
    for line in &mut self.input {
      line.clear();
    }
    self.notes.reset();
    self.detune.reset();
    self.stereo.reset();
  }

  /// the effect alone, with no latency against the input
  pub(crate) fn process(&mut self, frame: [f32; 2], settings: &Settings) -> [f32; 2] {
    self.notes.set_mode(settings.mode, settings.gate);
    self.notes.set_latch(settings.latch);
    if settings.detect != self.detector.range() {
      self.detector.set_range(settings.detect);
      self.input_pitch = lowest_pitch(&self.detector, self.sample_rate as f32);
    }
    if let Some(period) = self.detector.push(frame[0] + frame[1]) {
      self.input_pitch = (2.0 / f64::from(period)).clamp(*INPUT_PITCH.start(), *INPUT_PITCH.end());
    }

    let frame = if settings.smear > 0.0 {
      self
        .smear
        .process(frame, 0.5 * self.input_pitch, settings.smear)
    } else {
      frame
    };
    for (line, sample) in self.input.iter_mut().zip(frame) {
      line.push(sample);
    }

    self.vibrato_phase = match settings.vibrato_phase {
      Some(phase) => phase.rem_euclid(1.0),
      None => (self.vibrato_phase + settings.vibrato_step).fract(),
    };
    let vibrato = 2f64.powf(settings.vibrato * (TAU * self.vibrato_phase).sin() / 12.0);

    let mut out = self
      .notes
      .tick(self.input_pitch, vibrato, settings, &self.input);
    if settings.detune > 0.0 {
      out = self
        .detune
        .process(out, settings.detune * settings.detune, self.sample_rate);
    }
    if settings.stereo > 0.0 {
      out = self.stereo.process(out, settings.stereo, self.sample_rate);
    }
    out
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::grains::UNITY_DRIFT;
  use crate::notes::Note;

  const RATE: f32 = 48_000.0;
  const VOWEL_HZ: f32 = 110.0;
  const FORMANT_HZ: f32 = 800.0;
  const NEUTRAL: Settings = Settings {
    pitch: 0.0,
    formant: 0.0,
    formant_follow: 0.0,
    detune: 0.0,
    smear: 0.0,
    stereo: 0.0,
    smooth: 0.0,
    glide: 0.0,
    glide_notes_only: false,
    pressure_level: 0.0,
    slide_formant: 0.0,
    vibrato: 0.0,
    vibrato_step: 0.0,
    vibrato_phase: None,
    mode: Mode::Poly,
    gate: false,
    envelope: Adsr {
      attack: 0.01,
      decay: 0.05,
      sustain: 0.5,
      release: 0.1,
    },
    latch: false,
    detect: Range::Mid,
  };

  /// a pulse train through a low-pass, for the tilt of a voice's spectrum, and a resonance
  fn vowel_for(seconds: usize) -> Vec<f32> {
    let radius = (-std::f32::consts::PI * 80.0 / RATE).exp();
    let feedback = 2.0 * radius * (std::f32::consts::TAU * FORMANT_HZ / RATE).cos();
    let tilt = (-std::f32::consts::TAU * VOWEL_HZ / RATE).exp();
    let (mut y1, mut y2, mut phase, mut source) = (0.0, 0.0, 0.0, 0.0);
    (0..seconds * RATE as usize)
      .map(|_| {
        phase += VOWEL_HZ / RATE;
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
      .map(|&x| shifter.process([x, x], settings)[0])
      .collect()
  }

  fn pitch_hz(signal: &[f32]) -> f32 {
    let mut detector = Detector::new(RATE, Range::Mid);
    // the detector takes the sum of two channels
    let last = signal.iter().filter_map(|&x| detector.push(2.0 * x)).last();
    RATE / last.expect("the output has a pitch")
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
    let output = run(&mut Shifter::new(RATE), &input, &NEUTRAL);
    let settled = input.len() - RATE as usize;
    let error = output[settled..]
      .iter()
      .zip(&input[settled..])
      .fold(0f32, |worst, (a, b)| worst.max((a - b).abs()));
    assert!(error < 1e-3, "error {error}");
  }

  #[test]
  fn pitch_moves_the_fundamental_and_keeps_the_formant() {
    let settings = Settings {
      pitch: 7.0 / 12.0,
      ..NEUTRAL
    };
    let output = run(&mut Shifter::new(RATE), &vowel(), &settings);
    let expected = VOWEL_HZ * 2f32.powf(7.0 / 12.0);
    let fundamental = pitch_hz(&output);
    assert_near(fundamental, expected, 0.005 * expected, "fundamental");
    let formant = strongest_harmonic_hz(&output, fundamental);
    assert_near(formant, FORMANT_HZ, fundamental, "formant");
  }

  #[test]
  fn formant_moves_the_resonance_and_keeps_the_fundamental() {
    let settings = Settings {
      formant: 1.0,
      ..NEUTRAL
    };
    let output = run(&mut Shifter::new(RATE), &vowel(), &settings);
    let fundamental = pitch_hz(&output);
    assert_near(fundamental, VOWEL_HZ, 0.005 * VOWEL_HZ, "fundamental");
    let formant = strongest_harmonic_hz(&output, fundamental);
    assert_near(formant, 2.0 * FORMANT_HZ, fundamental, "formant");
  }

  /// a note an octave above the input, with the formants following by a half and by all of
  /// it, puts the resonance half an octave and an octave up
  #[test]
  fn formant_follows_the_note_by_the_set_share() {
    for follow in [0.5, 1.0] {
      let settings = Settings {
        formant_follow: f64::from(follow),
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
      latch: true,
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
      latch: true,
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
      gate: true,
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

  /// held at the top and at the bottom of its cycle, the vibrato moves a note by its depth
  #[test]
  fn vibrato_moves_the_pitch_by_its_depth() {
    for (phase, semitones) in [(0.25, 2.0), (0.75, -2.0)] {
      let settings = Settings {
        vibrato: 2.0,
        vibrato_phase: Some(phase),
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
      vibrato: 1.0,
      vibrato_step: 4.0 / f64::from(RATE),
      ..NEUTRAL
    };
    let mut shifter = shifter_in(&settings);
    shifter.notes.on(key(57));
    run(&mut shifter, &vowel(), &settings);
    assert_near(
      shifter.vibrato_phase as f32,
      0.0,
      1e-3,
      "phase after four whole cycles",
    );
  }

  #[test]
  fn pressure_sets_the_level_of_its_own_note() {
    let settings = Settings {
      pressure_level: 1.0,
      ..NEUTRAL
    };
    let loudest = |signal: &[f32]| signal.iter().fold(0f32, |peak, x| peak.max(x.abs()));
    let mut shifter = shifter_in(&settings);
    shifter.notes.on(Note {
      channel: 3,
      key: 57,
    });
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
      slide_formant: 1.0,
      ..NEUTRAL
    };
    let mut shifter = shifter_in(&settings);
    let note = Note {
      channel: 3,
      key: 57,
    };
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

  fn key(key: u8) -> Note {
    Note { channel: 0, key }
  }

  /// a bend on a note's own channel moves that note, and no note on another channel
  #[test]
  fn a_channel_bends_its_own_note() {
    let mut shifter = shifter_in(&NEUTRAL);
    let note = Note {
      channel: 3,
      key: 57,
    };
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

  /// a shifter that has seen `settings`, so notes are taken in its mode
  fn shifter_in(settings: &Settings) -> Shifter {
    let mut shifter = Shifter::new(RATE);
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
      mode: Mode::Mono,
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
      mode: Mode::Off,
      ..NEUTRAL
    };
    let mut shifter = shifter_in(&settings);
    shifter.notes.on(key(57));
    let output = run(&mut shifter, &vowel(), &settings);
    assert_near(pitch_hz(&output), VOWEL_HZ, 0.55, "fundamental");
  }
}
