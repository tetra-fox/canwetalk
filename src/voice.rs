//! one voice of the shifter. its pitch glides to its note's, or to the input's while it has
//! no note, and its level follows an envelope and its note's pressure
//!
//! frequencies are in cycles per two samples, as in `shifter`

use crate::envelope::{Adsr, Envelope};
use crate::grains::{Engine, Shape};
use crate::notes::Note;
use crate::ring::Ring;
use crate::shifter::Settings;

/// seconds a voice takes to fade when it starts or stops
const FADE_SECONDS: f64 = 0.005;
/// the envelope of a voice outside the gate modes, short fades that keep it from clicking
const FADE: Adsr = Adsr {
  attack: FADE_SECONDS,
  decay: 0.0,
  sustain: 1.0,
  release: FADE_SECONDS,
};
/// where a slide sits untouched, half way along its travel
pub(crate) const SLIDE_REST: f64 = 0.5;
const A4_HZ: f64 = 440.0;
const A4_NOTE: f64 = 69.0;

pub(crate) struct Voice {
  sample_rate: f64,
  engine: Engine,
  pub(crate) note: Option<Note>,
  /// semitones the host has tuned this note by
  pub(crate) tuning: f64,
  /// semitones the note is off its key, following its channels' bends and its tuning
  bend: f64,
  pub(crate) sounding: bool,
  /// the count of notes when this one came, which `notes` keeps
  pub(crate) started: u64,
  pub(crate) envelope: Envelope,
  /// the note's pressure, 0 to 1, and where it was last sent
  pressure: f64,
  pub(crate) pressure_sent: f64,
  /// the note's slide, 0 to 1 with the middle at rest, and where it was last sent
  slide: f64,
  pub(crate) slide_sent: f64,
  /// how much of the way the bend, pressure and slide move to where they were sent each
  /// sample. they arrive in steps, which would be heard
  control_step: f64,
  /// the gliding pitch
  pitch: f64,
  /// how far the pitch has moved from the input's to the note's under `glide_notes_only`
  note_blend: f64,
}

impl Voice {
  pub(crate) fn new(sample_rate: f64, input_pitch: f64) -> Self {
    Self {
      sample_rate,
      engine: Engine::new(),
      note: None,
      tuning: 0.0,
      bend: 0.0,
      sounding: false,
      started: 0,
      envelope: Envelope::default(),
      pressure: 0.0,
      pressure_sent: 0.0,
      slide: SLIDE_REST,
      slide_sent: SLIDE_REST,
      control_step: 1.0 - (-1.0 / (FADE_SECONDS * sample_rate)).exp(),
      pitch: input_pitch,
      note_blend: 0.0,
    }
  }

  /// silences the voice. its note and the pitch it glides from stay
  pub(crate) fn reset(&mut self) {
    self.engine.reset();
    self.envelope = Envelope::default();
  }

  /// takes up `note` with the bend, pressure and slide of its channel. they are where the
  /// note starts, not somewhere it moves to
  pub(crate) fn start(&mut self, note: Note, bend: f64, pressure: f64, slide: f64) {
    self.tuning = 0.0;
    self.bend = bend;
    (self.pressure, self.pressure_sent) = (pressure, pressure);
    (self.slide, self.slide_sent) = (slide, slide);
    if !self.sounding && self.envelope.is_silent() {
      // a voice coming from silence has no pitch to glide from
      self.pitch = note_pitch(f64::from(note.key) + bend, self.sample_rate);
      self.engine.reset();
    }
    self.note = Some(note);
    self.sounding = true;
    self.envelope.trigger();
  }

  /// one sample of the voice. `bend` is the semitones its note's channels are bent by and
  /// `vibrato` the ratio the vibrato moves its pitch by
  pub(crate) fn tick(
    &mut self,
    input_pitch: f64,
    bend: f64,
    vibrato: f64,
    settings: &Settings,
    input: &[Ring; 2],
  ) -> [f32; 2] {
    if !self.sounding && self.envelope.is_silent() {
      return [0.0; 2];
    }
    let goal = match self.note {
      Some(note) => {
        self.bend += self.control_step * (bend + self.tuning - self.bend);
        self.pressure += self.control_step * (self.pressure_sent - self.pressure);
        self.slide += self.control_step * (self.slide_sent - self.slide);
        note_pitch(f64::from(note.key) + self.bend, self.sample_rate)
      }
      None => input_pitch,
    };
    let step = 1.0 / (settings.glide.powi(3) * self.sample_rate + 1.0);
    self.pitch += step * (goal - self.pitch);
    let target = if settings.glide_notes_only {
      let held = f64::from(u8::from(self.note.is_some()));
      self.note_blend += step * (held - self.note_blend);
      if self.note_blend < 1e-8 {
        self.note_blend = 0.0;
      }
      (1.0 - self.note_blend) * input_pitch + self.note_blend * self.pitch
    } else {
      self.pitch
    };
    // times one with no vibrato, which leaves a voice at the input pitch exactly there
    let target = target * vibrato;
    let adsr = if settings.gate {
      &settings.envelope
    } else {
      &FADE
    };
    // a voice with no note has no pressure or slide to go by
    let (pressed, slid) = match self.note {
      Some(_) => (
        1.0 - settings.pressure_level * (1.0 - self.pressure),
        settings.slide_formant * (self.slide - SLIDE_REST) / SLIDE_REST,
      ),
      None => (1.0, 0.0),
    };
    let level = self.envelope.tick(self.sounding, adsr, self.sample_rate);
    let gain = (pressed * level) as f32;
    let shape = Shape {
      pitch: settings.pitch,
      formant: settings.formant + slid,
      formant_follow: settings.formant_follow,
      smooth: settings.smooth,
    };
    self
      .engine
      .tick(input_pitch, target, &shape, input)
      .map(|sample| gain * sample)
  }
}

/// pitch of a key, which may lie between two
fn note_pitch(key: f64, sample_rate: f64) -> f64 {
  2.0 * A4_HZ * 2f64.powf((key - A4_NOTE) / 12.0) / sample_rate
}
