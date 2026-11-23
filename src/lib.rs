//! canwetalk, a pitch and formant shifter for a monophonic input, played by MIDI notes

use std::sync::Arc;

use nice_plug::midi::{Channel, Key};
use nice_plug::prelude::*;

use crate::pitch::Range;
use crate::shifter::{Mode, Note, Settings, Shifter};

mod effects;
mod grains;
mod pitch;
mod ring;
mod shifter;

/// how long the output can go on after the input stops, which is as long as smear holds on
/// to a stretch of input
const TAIL_SECONDS: f32 = 10.0;
/// semitones the first channel's pitch bend reaches either way. it bends every note, as a
/// plain keyboard's wheel does and as the MPE master channel does
const MASTER_BEND_SEMITONES: f32 = 2.0;
/// the pitch and formant controls reach this far either way
const SHIFT_SEMITONES: f32 = 24.0;

pub struct CanWeTalk {
  params: Arc<CanWeTalkParams>,
  shifter: Shifter,
  sample_rate: f32,
}

#[derive(Enum, Clone, Copy, PartialEq, Eq)]
enum MidiMode {
  Off,
  Mono,
  Poly,
}

/// the span of pitches the detector looks for
#[derive(Enum, Clone, Copy, PartialEq, Eq)]
enum DetectRange {
  High,
  Mid,
  Bass,
}

#[derive(Params)]
struct CanWeTalkParams {
  #[id = "pitch"]
  pitch: FloatParam,
  #[id = "formant"]
  formant: FloatParam,
  #[id = "formant_follow"]
  formant_follow: FloatParam,
  #[id = "detune"]
  detune: FloatParam,
  #[id = "smear"]
  smear: FloatParam,
  #[id = "stereo"]
  stereo: FloatParam,
  #[id = "smooth"]
  smooth: FloatParam,
  #[id = "glide"]
  glide: FloatParam,
  #[id = "glide_notes"]
  glide_notes_only: BoolParam,
  #[id = "midi"]
  midi: EnumParam<MidiMode>,
  #[id = "bend_range"]
  bend_range: IntParam,
  #[id = "detect"]
  detect: EnumParam<DetectRange>,
  #[id = "mix"]
  mix: FloatParam,
}

impl Default for CanWeTalkParams {
  fn default() -> Self {
    // the time a control takes to reach a new value
    let ramp = SmoothingStyle::Linear(100.0);
    let semitones = |name| {
      let range = FloatRange::Linear {
        min: -SHIFT_SEMITONES,
        max: SHIFT_SEMITONES,
      };
      FloatParam::new(name, 0.0, range)
        .with_smoother(ramp.clone())
        .with_unit(" st")
        .with_value_to_string(formatters::v2s_f32_rounded(2))
    };
    let percent = |name, default| {
      FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_smoother(ramp.clone())
        .with_unit("%")
        .with_value_to_string(formatters::v2s_f32_percentage(0))
        .with_string_to_value(formatters::s2v_f32_percentage())
    };
    Self {
      pitch: semitones("Pitch"),
      formant: semitones("Formant"),
      formant_follow: percent("Formant Follow", 0.0),
      detune: percent("Detune", 0.0),
      smear: percent("Smear", 0.0),
      stereo: percent("Stereo", 0.0),
      smooth: percent("Smooth", 0.0),
      glide: percent("Glide", 0.0),
      glide_notes_only: BoolParam::new("Glide Notes Only", false),
      midi: EnumParam::new("MIDI", MidiMode::Poly),
      // 48 is what MPE controllers send over unless told otherwise
      bend_range: IntParam::new("MPE Bend Range", 48, IntRange::Linear { min: 1, max: 96 })
        .with_unit(" st"),
      detect: EnumParam::new("Detection Range", DetectRange::Mid),
      mix: percent("Mix", 1.0),
    }
  }
}

impl Default for CanWeTalk {
  fn default() -> Self {
    // the host's rate arrives in `activate`, which builds the shifter again
    let sample_rate = 48_000.0;
    Self {
      params: Arc::default(),
      shifter: Shifter::new(sample_rate),
      sample_rate,
    }
  }
}

impl Plugin for CanWeTalk {
  const NAME: &'static str = "CanWeTalk";
  const VENDOR: &'static str = "tetra";
  const URL: &'static str = "https://tetra.cool";
  const EMAIL: &'static str = "";
  const VERSION: &'static str = env!("CARGO_PKG_VERSION");

  const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
    AudioIOLayout {
      main_input_channels: NonZeroU32::new(2),
      main_output_channels: NonZeroU32::new(2),
      ..AudioIOLayout::const_default()
    },
    AudioIOLayout {
      main_input_channels: NonZeroU32::new(1),
      main_output_channels: NonZeroU32::new(1),
      ..AudioIOLayout::const_default()
    },
  ];

  // pitch bend comes with the control changes
  const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;

  type Editor = ();
  type SysExMessage = ();
  type BackgroundTask = ();

  fn params(&self) -> Arc<dyn Params> {
    self.params.clone()
  }

  fn activate(
    &mut self,
    _audio_io_layout: &AudioIOLayout,
    buffer_config: &BufferConfig,
    _context: &mut impl ActivateContext<Self>,
  ) -> bool {
    self.sample_rate = buffer_config.sample_rate;
    self.shifter = Shifter::new(self.sample_rate);
    true
  }

  fn reset(&mut self) {
    self.shifter.reset();
  }

  fn process(
    &mut self,
    buffer: &mut Buffer,
    _aux: &mut AuxiliaryBuffers,
    context: &mut impl ProcessContext<Self>,
  ) -> ProcessStatus {
    let params = &self.params;
    let mono = buffer.channels() == 1;
    let mut settings = Settings {
      pitch: 0.0,
      formant: 0.0,
      formant_follow: 0.0,
      detune: 0.0,
      smear: 0.0,
      stereo: 0.0,
      smooth: 0.0,
      glide: f64::from(params.glide.value()),
      glide_notes_only: params.glide_notes_only.value(),
      mode: match params.midi.value() {
        MidiMode::Off => Mode::Off,
        MidiMode::Mono => Mode::Mono,
        MidiMode::Poly => Mode::Poly,
      },
      detect: match params.detect.value() {
        DetectRange::Bass => Range::Bass,
        DetectRange::Mid => Range::Mid,
        DetectRange::High => Range::High,
      },
    };

    let mut next_event = context.next_event();
    for (index, mut samples) in buffer.iter_samples().enumerate() {
      while let Some(event) = next_event {
        if event.timing() > index as u32 {
          break;
        }
        // a host that names no channel for a note is not sending MPE, so the note goes on the
        // first channel with every other. one it names no key for is not a note to play
        let note = |channel: Channel, key: Key| {
          Some(Note {
            channel: channel.number().unwrap_or(0),
            key: key.number()?,
          })
        };
        match event {
          NoteEvent::NoteOn { channel, key, .. } => {
            if let Some(note) = note(channel, key) {
              self.shifter.note_on(note);
            }
          }
          NoteEvent::NoteOff { channel, key, .. } => {
            if let Some(note) = note(channel, key) {
              self.shifter.note_off(note);
            }
          }
          NoteEvent::PolyTuning {
            channel,
            key,
            tuning,
            ..
          } => {
            if let Some(note) = note(channel, key) {
              self.shifter.tune(note, f64::from(tuning));
            }
          }
          NoteEvent::MidiPitchBend { channel, value, .. } => {
            let range = match channel {
              0 => MASTER_BEND_SEMITONES,
              _ => params.bend_range.value() as f32,
            };
            self
              .shifter
              .bend(channel, f64::from((2.0 * value - 1.0) * range));
          }
          _ => (),
        }
        next_event = context.next_event();
      }

      settings.pitch = f64::from(params.pitch.smoothed.next() / 12.0);
      settings.formant = f64::from(params.formant.smoothed.next() / 12.0);
      settings.formant_follow = f64::from(params.formant_follow.smoothed.next());
      settings.detune = f64::from(params.detune.smoothed.next());
      settings.smear = f64::from(params.smear.smoothed.next());
      // one channel has no sides to widen, and the widener would only comb it
      let stereo = params.stereo.smoothed.next();
      settings.stereo = if mono { 0.0 } else { f64::from(stereo) };
      settings.smooth = f64::from(params.smooth.smoothed.next());
      let mix = params.mix.smoothed.next();

      let mut dry = [0.0; 2];
      for (dry, sample) in dry.iter_mut().zip(samples.iter_mut()) {
        *dry = *sample;
      }
      if mono {
        dry[1] = dry[0];
      }
      let wet = self.shifter.process(dry, &settings);
      for ((sample, wet), dry) in samples.iter_mut().zip(wet).zip(dry) {
        *sample = dry + (wet - dry) * mix;
      }
    }

    ProcessStatus::Tail((TAIL_SECONDS * self.sample_rate) as u32)
  }
}

impl ClapPlugin for CanWeTalk {
  const CLAP_ID: &'static str = "cool.tetra.canwetalk";
  const CLAP_DESCRIPTION: Option<&'static str> = Some("Pitch and formant shifter played by MIDI");
  const CLAP_MANUAL_URL: Option<&'static str> = None;
  const CLAP_SUPPORT_URL: Option<&'static str> = None;
  const CLAP_FEATURES: &'static [ClapFeature] = &[
    ClapFeature::AudioEffect,
    ClapFeature::PitchShifter,
    ClapFeature::Stereo,
    ClapFeature::Mono,
  ];
}

impl Vst3Plugin for CanWeTalk {
  const VST3_CLASS_ID: [u8; 16] = *b"CanWeTalkByTetra";
  const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
    &[Vst3SubCategory::Fx, Vst3SubCategory::PitchShift];
}

nice_export_clap!(CanWeTalk);
nice_export_vst3!(CanWeTalk);
