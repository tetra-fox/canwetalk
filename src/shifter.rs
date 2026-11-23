//! the whole effect: the input's pitch is detected, smear works on the input, a voice for each
//! held note resynthesizes it at that note's pitch, or one voice at the input's own pitch
//! while none is held, and detune and stereo work on their sum
//!
//! frequencies are in cycles per two samples, so 1 is the Nyquist frequency and a period of
//! P samples is 2 / P

use crate::effects::{Detune, Smear, Stereo};
use crate::grains::{Engine, Shape};
use crate::pitch::{Detector, Range};
use crate::ring::Ring;

const VOICES: usize = 16;
/// notes that wait for a voice while more are held than there are voices
const MAX_WAITING: usize = 16;
const HISTORY_SAMPLES: usize = 200_000;
/// the range the detected pitch is kept in, periods of 6400 down to 4 samples
const INPUT_PITCH: std::ops::RangeInclusive<f64> = 3.125e-4..=0.5;
/// seconds a voice takes to fade when it starts or stops
const FADE_SECONDS: f64 = 0.005;
const SILENT: f64 = 0.001;
const A4_HZ: f64 = 440.0;
const A4_NOTE: f64 = 69.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
  /// notes are ignored and one voice follows the input pitch
  Off,
  /// one voice plays the latest note
  Mono,
  /// each held note gets a voice, up to `VOICES`
  Poly,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Settings {
  /// octaves
  pub(crate) pitch: f64,
  /// octaves
  pub(crate) formant: f64,
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
  pub(crate) mode: Mode,
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
  voices: [Voice; VOICES],
  /// the voice that follows the input pitch while no note is held
  follower: Option<usize>,
  /// held notes that lost their voice to a later one, the latest last
  waiting: Vec<u8>,
  mode: Mode,
  /// counts notes, to tell which voice has had its note longest
  clock: u64,
  detune: Detune,
  stereo: Stereo,
}

struct Voice {
  engine: Engine,
  key: Option<u8>,
  sounding: bool,
  started: u64,
  level: f64,
  fade_step: f64,
  /// the gliding pitch
  pitch: f64,
  /// how far the pitch has moved from the input's to the note's under `glide_notes_only`
  note_blend: f64,
}

impl Voice {
  fn new(sample_rate: f64, input_pitch: f64) -> Self {
    Self {
      engine: Engine::new(),
      key: None,
      sounding: false,
      started: 0,
      level: 0.0,
      fade_step: 1.0 - (-1.0 / (FADE_SECONDS * sample_rate)).exp(),
      pitch: input_pitch,
      note_blend: 0.0,
    }
  }

  fn tick(
    &mut self,
    input_pitch: f64,
    settings: &Settings,
    sample_rate: f64,
    input: &[Ring; 2],
  ) -> [f32; 2] {
    if !self.sounding && self.level <= SILENT {
      return [0.0; 2];
    }
    let goal = self
      .key
      .map_or(input_pitch, |key| note_pitch(key, sample_rate));
    let step = 1.0 / (settings.glide.powi(3) * sample_rate + 1.0);
    self.pitch += step * (goal - self.pitch);
    let target = if settings.glide_notes_only {
      let held = f64::from(u8::from(self.key.is_some()));
      self.note_blend += step * (held - self.note_blend);
      if self.note_blend < 1e-8 {
        self.note_blend = 0.0;
      }
      (1.0 - self.note_blend) * input_pitch + self.note_blend * self.pitch
    } else {
      self.pitch
    };
    self.level += self.fade_step * (f64::from(u8::from(self.sounding)) - self.level);
    // the fade is applied to what the voice reads and again to what it gives
    let gain = (self.level * self.level) as f32;
    let shape = Shape {
      pitch: settings.pitch,
      formant: settings.formant,
      smooth: settings.smooth,
    };
    self
      .engine
      .tick(input_pitch, target, &shape, input)
      .map(|sample| gain * sample)
  }
}

fn note_pitch(key: u8, sample_rate: f64) -> f64 {
  2.0 * A4_HZ * 2f64.powf((f64::from(key) - A4_NOTE) / 12.0) / sample_rate
}

impl Shifter {
  pub(crate) fn new(sample_rate: f32) -> Self {
    let rate = f64::from(sample_rate);
    let detector = Detector::new(sample_rate, Range::Mid);
    let input_pitch = 2.0 / f64::from(detector.range().longest_period(sample_rate));
    let mut shifter = Self {
      sample_rate: rate,
      detector,
      input_pitch,
      smear: Smear::new(sample_rate),
      input: [Ring::new(HISTORY_SAMPLES), Ring::new(HISTORY_SAMPLES)],
      voices: std::array::from_fn(|_| Voice::new(rate, input_pitch)),
      follower: None,
      waiting: Vec::with_capacity(MAX_WAITING),
      mode: Mode::Off,
      clock: 0,
      detune: Detune::new(sample_rate),
      stereo: Stereo::new(sample_rate),
    };
    shifter.release_all();
    shifter
  }

  /// the pitch assumed until one is detected, the lowest of the range
  fn lowest_pitch(&self) -> f64 {
    2.0
      / f64::from(
        self
          .detector
          .range()
          .longest_period(self.sample_rate as f32),
      )
  }

  /// does not allocate
  pub(crate) fn reset(&mut self) {
    self.detector.reset();
    self.input_pitch = self.lowest_pitch();
    self.smear.reset();
    for line in &mut self.input {
      line.clear();
    }
    for voice in &mut self.voices {
      voice.engine.reset();
      voice.level = 0.0;
    }
    self.detune.reset();
    self.stereo.reset();
    self.release_all();
  }

  /// drops every note and leaves the first voice following the input
  fn release_all(&mut self) {
    self.waiting.clear();
    for voice in &mut self.voices {
      voice.key = None;
      voice.sounding = false;
    }
    self.voices[0].sounding = true;
    self.follower = Some(0);
  }

  pub(crate) fn note_on(&mut self, key: u8) {
    let count = match self.mode {
      Mode::Off => return,
      Mode::Mono => 1,
      Mode::Poly => VOICES,
    };
    self.clock += 1;
    let voices = &mut self.voices[..count];
    // the follower first, then a voice with nothing to play, then the one longest on its note
    let index = (self.follower.take())
      .or_else(|| voices.iter().position(|voice| voice.key == Some(key)))
      .or_else(|| voices.iter().position(|voice| !voice.sounding))
      .unwrap_or_else(|| {
        (0..count)
          .min_by_key(|&index| voices[index].started)
          .expect("there is at least one voice")
      });
    let voice = &mut voices[index];
    if let Some(displaced) = voice.key.filter(|&held| held != key) {
      if self.waiting.len() == MAX_WAITING {
        self.waiting.remove(0);
      }
      self.waiting.push(displaced);
    }
    if !voice.sounding && voice.level <= SILENT {
      // a voice coming from silence has no pitch to glide from
      voice.pitch = note_pitch(key, self.sample_rate);
      voice.engine.reset();
    }
    voice.key = Some(key);
    voice.sounding = true;
    voice.started = self.clock;
  }

  pub(crate) fn note_off(&mut self, key: u8) {
    self.waiting.retain(|&waiting| waiting != key);
    let Some(index) = self.voices.iter().position(|voice| voice.key == Some(key)) else {
      return;
    };
    if let Some(waiting) = self.waiting.pop() {
      self.voices[index].key = Some(waiting);
      return;
    }
    self.voices[index].key = None;
    if self.voices.iter().all(|voice| voice.key.is_none()) {
      // the last voice playing goes back to following the input
      self.follower = Some(index);
    } else {
      self.voices[index].sounding = false;
    }
  }

  /// the effect alone, with no latency against the input
  pub(crate) fn process(&mut self, frame: [f32; 2], settings: &Settings) -> [f32; 2] {
    if settings.mode != self.mode {
      self.mode = settings.mode;
      self.release_all();
    }

    if settings.detect != self.detector.range() {
      self.detector.set_range(settings.detect);
      self.input_pitch = self.lowest_pitch();
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

    let mut out = [0.0; 2];
    for voice in &mut self.voices {
      let voiced = voice.tick(self.input_pitch, settings, self.sample_rate, &self.input);
      for (out, sample) in out.iter_mut().zip(voiced) {
        *out += sample;
      }
    }
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

  const RATE: f32 = 48_000.0;
  const VOWEL_HZ: f32 = 110.0;
  const FORMANT_HZ: f32 = 800.0;
  const NEUTRAL: Settings = Settings {
    pitch: 0.0,
    formant: 0.0,
    detune: 0.0,
    smear: 0.0,
    stereo: 0.0,
    smooth: 0.0,
    glide: 0.0,
    glide_notes_only: false,
    mode: Mode::Poly,
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

  /// a shifter that has seen `settings`, so notes are taken in its mode
  fn shifter_in(settings: &Settings) -> Shifter {
    let mut shifter = Shifter::new(RATE);
    shifter.process([0.0; 2], settings);
    shifter
  }

  #[test]
  fn held_note_sets_the_pitch_until_released() {
    let mut shifter = shifter_in(&NEUTRAL);
    shifter.note_on(57);
    let output = run(&mut shifter, &vowel(), &NEUTRAL);
    assert_near(pitch_hz(&output), 220.0, 1.1, "fundamental");
    shifter.note_off(57);
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
    shifter.note_on(57);
    shifter.note_on(64);
    let output = run(&mut shifter, &vowel(), &NEUTRAL);
    let absent = power_at(&output, C4);
    assert!(power_at(&output, A3) > 100.0 * absent, "A3 is missing");
    assert!(power_at(&output, E4) > 100.0 * absent, "E4 is missing");

    shifter.note_off(57);
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
    shifter.note_on(57);
    shifter.note_on(64);
    let output = run(&mut shifter, &vowel(), &settings);
    assert_near(
      pitch_hz(&output),
      E4,
      0.005 * E4,
      "fundamental with both held",
    );
    shifter.note_off(64);
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
    shifter.note_on(57);
    let output = run(&mut shifter, &vowel(), &settings);
    assert_near(pitch_hz(&output), VOWEL_HZ, 0.55, "fundamental");
  }
}
