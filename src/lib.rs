//! canwetalk, a pitch and formant shifter for a monophonic input, played by MIDI notes

use std::sync::Arc;

use nice_plug::midi::{Channel, Key};
use nice_plug::prelude::*;

use crate::notes::Note;
use crate::params::CanWeTalkParams;
use crate::shifter::{Settings, Shifter, Vibrato};

mod effects;
mod envelope;
mod grains;
mod notes;
mod params;
mod pitch;
mod ring;
mod shifter;
mod voice;

/// how long the output can go on after the input stops, which is as long as smear holds on
/// to a stretch of input
const TAIL_SECONDS: f32 = 10.0;
/// semitones the first channel's pitch bend reaches either way. it bends every note, as a
/// plain keyboard's wheel does and as the MPE master channel does
const MASTER_BEND_SEMITONES: f32 = 2.0;
/// the control change MPE sends a note's slide as
const SLIDE_CC: u8 = 74;

pub struct CanWeTalk {
  params: Arc<CanWeTalkParams>,
  shifter: Shifter,
  sample_rate: f32,
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
    // synced, the vibrato takes its rate from the tempo, and while the host plays its phase
    // from the position too, so a cycle starts on each division of the bar
    let transport = context.transport();
    let cycle_beats = params.vibrato.division.value().beats();
    let beats_per_sample = (transport.tempo)
      .filter(|_| params.vibrato.sync.value())
      .map(|tempo| tempo / 60.0 / f64::from(self.sample_rate));
    let vibrato_step = match beats_per_sample {
      Some(beats) => beats / cycle_beats,
      None => f64::from(params.vibrato.rate.value() / self.sample_rate),
    };
    let start_beats = transport
      .pos_beats()
      .filter(|_| transport.playing)
      .zip(beats_per_sample);
    let play = params.notes.play();
    let envelope = params.envelope.adsr();
    let detect = params.detect.value();

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
              self.shifter.notes.on(note);
            }
          }
          NoteEvent::NoteOff { channel, key, .. } => {
            if let Some(note) = note(channel, key) {
              self.shifter.notes.off(note);
            }
          }
          NoteEvent::PolyTuning {
            channel,
            key,
            tuning,
            ..
          } => {
            if let Some(note) = note(channel, key) {
              self.shifter.notes.tune(note, f64::from(tuning));
            }
          }
          NoteEvent::PolyPressure {
            channel,
            key,
            pressure,
            ..
          } => {
            if let Some(note) = note(channel, key) {
              self.shifter.notes.note_pressure(note, f64::from(pressure));
            }
          }
          NoteEvent::PolyBrightness {
            channel,
            key,
            brightness,
            ..
          } => {
            if let Some(note) = note(channel, key) {
              self.shifter.notes.note_slide(note, f64::from(brightness));
            }
          }
          NoteEvent::MidiChannelPressure {
            channel, pressure, ..
          } => {
            self
              .shifter
              .notes
              .channel_pressure(channel, f64::from(pressure));
          }
          NoteEvent::MidiCC {
            channel,
            cc: SLIDE_CC,
            value,
            ..
          } => {
            self.shifter.notes.channel_slide(channel, f64::from(value));
          }
          NoteEvent::MidiPitchBend { channel, value, .. } => {
            let range = match channel {
              0 => MASTER_BEND_SEMITONES,
              _ => params.expression.bend_range.value() as f32,
            };
            self
              .shifter
              .notes
              .bend(channel, f64::from((2.0 * value - 1.0) * range));
          }
          _ => (),
        }
        next_event = context.next_event();
      }

      let settings = Settings {
        shape: params.shift.next(),
        vibrato: Vibrato {
          depth: f64::from(params.vibrato.depth.smoothed.next() / 100.0),
          step: vibrato_step,
          phase: start_beats
            .map(|(start, per_sample)| (start + index as f64 * per_sample) / cycle_beats),
        },
        play,
        envelope,
        expression: params.expression.next(),
        effects: params.effects.next(mono),
        detect,
      };
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
