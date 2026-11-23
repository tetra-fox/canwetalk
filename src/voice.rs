//! one voice of the shifter. its pitch glides to its note's, or to the input's while it has
//! no note, and its level follows an envelope and its note's pressure
//!
//! frequencies are in cycles per two samples, as in `shifter`

use crate::envelope::{Adsr, Envelope};
use crate::grains::{Engine, Shape};
use crate::marks::Cycle;
use crate::notes::Note;
use crate::ring::Ring;
use crate::shifter::{BLOCK, Settings};

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

/// what a note's pressure and slide do to it
#[derive(Clone, Copy, Debug)]
pub(crate) struct Expression {
  /// how much of a note's level its pressure sets, 0 for none of it and 1 for all
  pub(crate) pressure_level: f64,
  /// octaves a note's slide moves its formants at either end of its travel
  pub(crate) slide_formant: f64,
}

pub(crate) struct Voice {
  sample_rate: f64,
  engine: Engine,
  /// the note played. a released note stays through its fade, which is at the note's pitch
  pub(crate) note: Option<Note>,
  /// semitones the host has tuned this note by
  pub(crate) tuning: f64,
  /// semitones the note is off its key, following its channels' bends and its tuning
  bend: f64,
  /// whether the note is held, or with no note the input followed. otherwise the voice fades
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

  /// back to how `new` makes it, without allocating
  pub(crate) fn reset(&mut self, input_pitch: f64) {
    // every field is named so that one added later cannot be left out
    let Self {
      sample_rate: _,
      control_step: _,
      engine,
      note,
      tuning,
      bend,
      sounding,
      started,
      envelope,
      pressure,
      pressure_sent,
      slide,
      slide_sent,
      pitch,
      note_blend,
    } = self;
    engine.reset();
    *note = None;
    (*tuning, *bend) = (0.0, 0.0);
    *sounding = false;
    *started = 0;
    *envelope = Envelope::default();
    (*pressure, *pressure_sent) = (0.0, 0.0);
    (*slide, *slide_sent) = (SLIDE_REST, SLIDE_REST);
    *pitch = input_pitch;
    *note_blend = 0.0;
  }

  /// the note while it is held
  pub(crate) fn held(&self) -> Option<Note> {
    self.note.filter(|_| self.sounding)
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
      self.pitch = note_pitch(f64::from(note.key()) + bend, self.sample_rate);
      self.engine.reset();
    }
    self.note = Some(note);
    self.sounding = true;
    self.envelope.trigger();
  }

  /// goes on with `note`, which was held before the note the voice has and waited for it.
  /// the pressure and slide move to where the note's channel has them
  pub(crate) fn resume(&mut self, note: Note, pressure: f64, slide: f64) {
    self.note = Some(note);
    self.tuning = 0.0;
    self.pressure_sent = pressure;
    self.slide_sent = slide;
  }

  /// adds the voice to the samples of `out`, of which `input` already holds every one's
  /// input. each has its own input cycle, settings and ratio the vibrato moves the pitch by.
  /// `bend` is the semitones the note's channels are bent by
  pub(crate) fn tick(
    &mut self,
    cycles: &[Cycle],
    bend: f64,
    vibratos: &[f64],
    settings: &[Settings],
    input: &[Ring; 2],
    out: &mut [[f32; 2]],
  ) {
    let (mut targets, mut gains) = ([0.0; BLOCK], [0.0; BLOCK]);
    let mut shapes = [Shape::default(); BLOCK];
    // a voice that fades to silence is left where it is from that sample on
    let mut sounded = 0;
    for ((cycle, vibrato), settings) in cycles.iter().zip(vibratos).zip(settings) {
      let Some(moved) = self.moved(*cycle, bend, *vibrato, settings) else {
        break;
      };
      (targets[sounded], shapes[sounded], gains[sounded]) = moved;
      sounded += 1;
    }
    let mut voiced = [[0.0; 2]; BLOCK];
    self.engine.tick(
      &cycles[..sounded],
      &targets[..sounded],
      &shapes[..sounded],
      input,
      out.len() - sounded,
      &mut voiced[..sounded],
    );
    for ((out, voiced), gain) in out.iter_mut().zip(voiced).zip(gains).take(sounded) {
      for (out, sample) in out.iter_mut().zip(voiced) {
        *out += gain * sample;
      }
    }
  }

  /// moves the voice on by one sample and gives the pitch and shape its grains have there
  /// and its level, or nothing once it is silent
  fn moved(
    &mut self,
    cycle: Cycle,
    bend: f64,
    vibrato: f64,
    settings: &Settings,
  ) -> Option<(f64, Shape, f32)> {
    if !self.sounding && self.envelope.is_silent() {
      return None;
    }
    let goal = match self.note {
      Some(note) => {
        self.bend += self.control_step * (bend + self.tuning - self.bend);
        self.pressure += self.control_step * (self.pressure_sent - self.pressure);
        self.slide += self.control_step * (self.slide_sent - self.slide);
        note_pitch(f64::from(note.key()) + self.bend, self.sample_rate)
      }
      None => cycle.pitch,
    };
    let step = 1.0 / (settings.play.glide.powi(3) * self.sample_rate + 1.0);
    self.pitch += step * (goal - self.pitch);
    let target = if settings.play.glide_notes_only {
      let held = f64::from(u8::from(self.note.is_some()));
      self.note_blend += step * (held - self.note_blend);
      if self.note_blend < 1e-8 {
        self.note_blend = 0.0;
      }
      (1.0 - self.note_blend) * cycle.pitch + self.note_blend * self.pitch
    } else {
      self.pitch
    };
    // times one with no vibrato, which leaves a voice at the input pitch exactly there
    let target = target * vibrato;
    let adsr = if settings.play.mode.is_gated() {
      &settings.envelope
    } else {
      &FADE
    };
    // a voice with no note has no pressure or slide to go by
    let (pressed, slid) = match self.note {
      Some(_) => (
        1.0 - settings.expression.pressure_level * (1.0 - self.pressure),
        settings.expression.slide_formant * (self.slide - SLIDE_REST) / SLIDE_REST,
      ),
      None => (1.0, 0.0),
    };
    let level = self.envelope.tick(self.sounding, adsr, self.sample_rate);
    let shape = Shape {
      formant: settings.shape.formant + slid,
      ..settings.shape
    };
    Some((target, shape, (pressed * level) as f32))
  }
}

/// pitch of a key, which may lie between two
fn note_pitch(key: f64, sample_rate: f64) -> f64 {
  2.0 * A4_HZ * 2f64.powf((key - A4_NOTE) / 12.0) / sample_rate
}
