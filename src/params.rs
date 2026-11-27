//! the plugin's parameters, grouped as the host shows them, and the shifter's settings they
//! stand for

use nice_plug::prelude::*;

use crate::effects::Amounts;
use crate::envelope::Adsr;
use crate::grains::Shape;
use crate::notes::{Mode, Play};
use crate::pitch::Range;
use crate::shifter::{Settings, Vibrato};
use crate::voice::Expression;

/// the pitch and formant controls reach this far either way
pub(crate) const SHIFT_SEMITONES: f32 = 24.0;
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
  #[id = "anchor"]
  pub(crate) anchor: BoolParam,
  /// on, the effect has no latency. off, it hears the input a little after the pitch detector
  /// does, which is the latency it reports
  #[id = "live"]
  pub(crate) live: BoolParam,
  #[id = "mix"]
  pub(crate) mix: FloatParam,
  /// how many times its own size the editor's window is, kept with the plugin's state so
  /// that it opens again at the size it was left at
  #[persist = "editor-scale"]
  pub(crate) editor_scale: AtomicF32,
}

/// what stands for a whole block of samples
pub(crate) struct Block {
  /// whether there is one channel
  mono: bool,
  detect: Range,
  anchor: bool,
  vibrato: VibratoClock,
}

/// what a block's vibrato runs by
struct VibratoClock {
  /// cycles per sample
  step: f64,
  /// beats in a cycle
  cycle_beats: f64,
  /// the host's position in beats at the block's first sample, and the beats it moves on
  /// each sample. only while the vibrato is synced and the host plays
  position: Option<(f64, f64)>,
}

#[derive(Params)]
pub(crate) struct ShiftParams {
  #[id = "pitch"]
  pub(crate) pitch: FloatParam,
  #[id = "formant"]
  pub(crate) formant: FloatParam,
  #[id = "formant_follow"]
  pub(crate) formant_follow: FloatParam,
  #[id = "smooth"]
  pub(crate) smooth: FloatParam,
}

#[derive(Params)]
pub(crate) struct EffectParams {
  #[id = "detune"]
  pub(crate) detune: FloatParam,
  #[id = "smear"]
  pub(crate) smear: FloatParam,
  #[id = "stereo"]
  pub(crate) stereo: FloatParam,
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
  pub(crate) mode: EnumParam<Mode>,
  #[id = "latch"]
  pub(crate) latch: BoolParam,
  #[id = "glide"]
  pub(crate) glide: FloatParam,
  #[id = "glide_notes"]
  pub(crate) glide_notes_only: BoolParam,
}

#[derive(Params)]
pub(crate) struct EnvelopeParams {
  #[id = "attack"]
  pub(crate) attack: FloatParam,
  #[id = "decay"]
  pub(crate) decay: FloatParam,
  #[id = "sustain"]
  pub(crate) sustain: FloatParam,
  #[id = "release"]
  pub(crate) release: FloatParam,
}

#[derive(Params)]
pub(crate) struct ExpressionParams {
  #[id = "pressure_level"]
  pub(crate) pressure_level: FloatParam,
  #[id = "slide_formant"]
  pub(crate) slide_formant: FloatParam,
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
  fn beats(self) -> f64 {
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
      anchor: BoolParam::new("Anchor Grains", true),
      live: BoolParam::new("Live", false),
      mix: percent("Mix", 1.0),
      editor_scale: AtomicF32::new(1.0),
    }
  }
}

impl CanWeTalkParams {
  /// reads the controls that are not smoothed, once for a block
  pub(crate) fn block(&self, transport: &Transport, sample_rate: f32, mono: bool) -> Block {
    Block {
      mono,
      detect: self.detect.value(),
      anchor: self.anchor.value(),
      vibrato: self.vibrato.clock(transport, sample_rate),
    }
  }

  /// the settings for the sample `index` into `block`. moves the smoothed controls on by one
  /// sample, so this is called once for each sample in turn
  pub(crate) fn settings(&self, block: &Block, index: usize) -> Settings {
    Settings {
      shape: self.shift.next(),
      vibrato: self.vibrato.next(&block.vibrato, index),
      play: self.notes.next(),
      envelope: self.envelope.next(),
      expression: self.expression.next(),
      effects: self.effects.next(block.mono),
      detect: block.detect,
      anchor: block.anchor,
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
  fn next(&self) -> Shape {
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
  fn next(&self, mono: bool) -> Amounts {
    let stereo = self.stereo.smoothed.next();
    Amounts {
      detune: f64::from(self.detune.smoothed.next()),
      smear: f64::from(self.smear.smoothed.next()),
      // one channel has no sides to widen
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

impl VibratoParams {
  /// synced, the vibrato takes its rate from the tempo, and while the host plays its phase
  /// from the position too, so a cycle starts on each division of the bar
  fn clock(&self, transport: &Transport, sample_rate: f32) -> VibratoClock {
    let cycle_beats = self.division.value().beats();
    let beats_per_sample = (transport.tempo)
      .filter(|_| self.sync.value())
      .map(|tempo| tempo / 60.0 / f64::from(sample_rate));
    VibratoClock {
      step: match beats_per_sample {
        Some(beats) => beats / cycle_beats,
        None => f64::from(self.rate.value() / sample_rate),
      },
      cycle_beats,
      position: transport
        .pos_beats()
        .filter(|_| transport.playing)
        .zip(beats_per_sample),
    }
  }

  /// the vibrato for the sample `index` into the block of `clock`. the depth's cents become
  /// semitones
  fn next(&self, clock: &VibratoClock, index: usize) -> Vibrato {
    Vibrato {
      depth: f64::from(self.depth.smoothed.next() / 100.0),
      step: clock.step,
      phase: clock
        .position
        .map(|(start, per_sample)| (start + index as f64 * per_sample) / clock.cycle_beats),
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
  /// how notes are played for the next sample
  fn next(&self) -> Play {
    Play {
      mode: self.mode.value(),
      latch: self.latch.value(),
      glide: f64::from(self.glide.smoothed.next()),
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
  /// the envelope for the next sample, its times in seconds
  fn next(&self) -> Adsr {
    Adsr {
      attack: f64::from(self.attack.value() / 1000.0),
      decay: f64::from(self.decay.value() / 1000.0),
      sustain: f64::from(self.sustain.smoothed.next()),
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
  fn next(&self) -> Expression {
    Expression {
      pressure_level: f64::from(self.pressure_level.smoothed.next()),
      slide_formant: f64::from(self.slide_formant.smoothed.next() / 12.0),
    }
  }
}
