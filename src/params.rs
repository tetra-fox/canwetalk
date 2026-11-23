//! the plugin's parameters, grouped as the host shows them, and the shifter's settings they
//! stand for

use nice_plug::prelude::*;

use crate::effects::Amounts;
use crate::envelope::Adsr;
use crate::grains::Shape;
use crate::notes::{Mode, Play};
use crate::pitch::Range;
use crate::voice::Expression;

/// the pitch and formant controls reach this far either way
const SHIFT_SEMITONES: f32 = 24.0;
/// the time a smoothed control takes to reach a new value
const RAMP: SmoothingStyle = SmoothingStyle::Linear(100.0);

#[derive(Params)]
pub(crate) struct CanWeTalkParams {
  #[nested(group = "Shift")]
  pub(crate) shift: ShiftParams,
  #[nested(group = "Effects")]
  pub(crate) effects: EffectParams,
  #[nested(group = "Vibrato")]
  pub(crate) vibrato: VibratoParams,
  #[nested(group = "Notes")]
  pub(crate) notes: NoteParams,
  #[nested(group = "Envelope")]
  pub(crate) envelope: EnvelopeParams,
  #[nested(group = "Expression")]
  pub(crate) expression: ExpressionParams,
  #[id = "detect"]
  pub(crate) detect: EnumParam<Range>,
  #[id = "mix"]
  pub(crate) mix: FloatParam,
}

#[derive(Params)]
pub(crate) struct ShiftParams {
  #[id = "pitch"]
  pitch: FloatParam,
  #[id = "formant"]
  formant: FloatParam,
  #[id = "formant_follow"]
  formant_follow: FloatParam,
  #[id = "smooth"]
  smooth: FloatParam,
}

#[derive(Params)]
pub(crate) struct EffectParams {
  #[id = "detune"]
  detune: FloatParam,
  #[id = "smear"]
  smear: FloatParam,
  #[id = "stereo"]
  stereo: FloatParam,
}

#[derive(Params)]
pub(crate) struct VibratoParams {
  #[id = "vibrato"]
  pub(crate) depth: FloatParam,
  #[id = "vibrato_rate"]
  pub(crate) rate: FloatParam,
  #[id = "vibrato_sync"]
  pub(crate) sync: BoolParam,
  #[id = "vibrato_division"]
  pub(crate) division: EnumParam<Division>,
}

#[derive(Params)]
pub(crate) struct NoteParams {
  #[id = "midi"]
  mode: EnumParam<Mode>,
  #[id = "latch"]
  latch: BoolParam,
  #[id = "glide"]
  glide: FloatParam,
  #[id = "glide_notes"]
  glide_notes_only: BoolParam,
}

#[derive(Params)]
pub(crate) struct EnvelopeParams {
  #[id = "attack"]
  attack: FloatParam,
  #[id = "decay"]
  decay: FloatParam,
  #[id = "sustain"]
  sustain: FloatParam,
  #[id = "release"]
  release: FloatParam,
}

#[derive(Params)]
pub(crate) struct ExpressionParams {
  #[id = "pressure_level"]
  pressure_level: FloatParam,
  #[id = "slide_formant"]
  slide_formant: FloatParam,
  /// semitones a channel's pitch bend reaches either way, for every channel but the first
  #[id = "bend_range"]
  pub(crate) bend_range: IntParam,
}

/// the length of a vibrato cycle against the host's tempo
#[derive(Enum, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Division {
  #[name = "1/1"]
  Whole,
  #[name = "1/2"]
  Half,
  #[name = "1/4"]
  Quarter,
  #[name = "1/4T"]
  QuarterTriplet,
  #[name = "1/8"]
  Eighth,
  #[name = "1/8T"]
  EighthTriplet,
  #[name = "1/16"]
  Sixteenth,
  #[name = "1/16T"]
  SixteenthTriplet,
  #[name = "1/32"]
  ThirtySecond,
}

impl Division {
  /// in quarter notes, which is what hosts count beats in
  pub(crate) fn beats(self) -> f64 {
    match self {
      Self::Whole => 4.0,
      Self::Half => 2.0,
      Self::Quarter => 1.0,
      Self::QuarterTriplet => 2.0 / 3.0,
      Self::Eighth => 0.5,
      Self::EighthTriplet => 1.0 / 3.0,
      Self::Sixteenth => 0.25,
      Self::SixteenthTriplet => 1.0 / 6.0,
      Self::ThirtySecond => 0.125,
    }
  }
}

fn semitones(name: &str) -> FloatParam {
  let range = FloatRange::Linear {
    min: -SHIFT_SEMITONES,
    max: SHIFT_SEMITONES,
  };
  FloatParam::new(name, 0.0, range)
    .with_smoother(RAMP)
    .with_unit(" st")
    .with_value_to_string(formatters::v2s_f32_rounded(2))
}

fn percent(name: &str, default: f32) -> FloatParam {
  FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
    .with_smoother(RAMP)
    .with_unit("%")
    .with_value_to_string(formatters::v2s_f32_percentage(0))
    .with_string_to_value(formatters::s2v_f32_percentage())
}

fn milliseconds(name: &str, default: f32) -> FloatParam {
  let range = FloatRange::Skewed {
    min: 0.0,
    max: 10_000.0,
    factor: FloatRange::skew_factor(-2.5),
  };
  FloatParam::new(name, default, range)
    .with_unit(" ms")
    .with_value_to_string(formatters::v2s_f32_rounded(0))
}

impl Default for CanWeTalkParams {
  fn default() -> Self {
    Self {
      shift: ShiftParams::default(),
      effects: EffectParams::default(),
      vibrato: VibratoParams::default(),
      notes: NoteParams::default(),
      envelope: EnvelopeParams::default(),
      expression: ExpressionParams::default(),
      detect: EnumParam::new("Detection Range", Range::Mid),
      mix: percent("Mix", 1.0),
    }
  }
}

impl Default for ShiftParams {
  fn default() -> Self {
    Self {
      pitch: semitones("Pitch"),
      formant: semitones("Formant"),
      formant_follow: percent("Formant Follow", 0.0),
      smooth: percent("Smooth", 0.0),
    }
  }
}

impl ShiftParams {
  /// the shape for the next sample. the semitone controls become octaves
  pub(crate) fn next(&self) -> Shape {
    Shape {
      pitch: f64::from(self.pitch.smoothed.next() / 12.0),
      formant: f64::from(self.formant.smoothed.next() / 12.0),
      formant_follow: f64::from(self.formant_follow.smoothed.next()),
      smooth: f64::from(self.smooth.smoothed.next()),
    }
  }
}

impl Default for EffectParams {
  fn default() -> Self {
    Self {
      detune: percent("Detune", 0.0),
      smear: percent("Smear", 0.0),
      stereo: percent("Stereo", 0.0),
    }
  }
}

impl EffectParams {
  /// the amounts for the next sample, for one channel if `mono`
  pub(crate) fn next(&self, mono: bool) -> Amounts {
    let stereo = self.stereo.smoothed.next();
    Amounts {
      detune: f64::from(self.detune.smoothed.next()),
      smear: f64::from(self.smear.smoothed.next()),
      // one channel has no sides to widen, and the widener would only comb it
      stereo: if mono { 0.0 } else { f64::from(stereo) },
    }
  }
}

impl Default for VibratoParams {
  fn default() -> Self {
    Self {
      depth: FloatParam::new(
        "Vibrato",
        0.0,
        FloatRange::Linear {
          min: 0.0,
          max: 200.0,
        },
      )
      .with_smoother(RAMP)
      .with_unit(" ct")
      .with_value_to_string(formatters::v2s_f32_rounded(0)),
      rate: FloatParam::new(
        "Vibrato Rate",
        5.0,
        FloatRange::Linear {
          min: 2.0,
          max: 15.0,
        },
      )
      .with_unit(" Hz")
      .with_value_to_string(formatters::v2s_f32_rounded(2)),
      sync: BoolParam::new("Vibrato Sync", false),
      division: EnumParam::new("Vibrato Division", Division::Sixteenth),
    }
  }
}

impl Default for NoteParams {
  fn default() -> Self {
    Self {
      mode: EnumParam::new("MIDI", Mode::Poly),
      latch: BoolParam::new("Latch", false),
      glide: percent("Glide", 0.0),
      glide_notes_only: BoolParam::new("Glide Notes Only", false),
    }
  }
}

impl NoteParams {
  pub(crate) fn play(&self) -> Play {
    Play {
      mode: self.mode.value(),
      latch: self.latch.value(),
      glide: f64::from(self.glide.value()),
      glide_notes_only: self.glide_notes_only.value(),
    }
  }
}

impl Default for EnvelopeParams {
  fn default() -> Self {
    Self {
      attack: milliseconds("Attack", 5.0),
      decay: milliseconds("Decay", 200.0),
      sustain: percent("Sustain", 1.0),
      release: milliseconds("Release", 200.0),
    }
  }
}

impl EnvelopeParams {
  /// the times in seconds
  pub(crate) fn adsr(&self) -> Adsr {
    Adsr {
      attack: f64::from(self.attack.value() / 1000.0),
      decay: f64::from(self.decay.value() / 1000.0),
      sustain: f64::from(self.sustain.value()),
      release: f64::from(self.release.value() / 1000.0),
    }
  }
}

impl Default for ExpressionParams {
  fn default() -> Self {
    Self {
      pressure_level: percent("Pressure to Level", 0.0),
      slide_formant: semitones("Slide to Formant"),
      // 48 is what MPE controllers send over unless told otherwise
      bend_range: IntParam::new("MPE Bend Range", 48, IntRange::Linear { min: 1, max: 96 })
        .with_unit(" st"),
    }
  }
}

impl ExpressionParams {
  /// the settings for the next sample. the slide's semitones become octaves
  pub(crate) fn next(&self) -> Expression {
    Expression {
      pressure_level: f64::from(self.pressure_level.smoothed.next()),
      slide_formant: f64::from(self.slide_formant.smoothed.next() / 12.0),
    }
  }
}
