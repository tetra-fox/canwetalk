//! canwetalk, a pitch and formant shifter for a monophonic input, played by MIDI notes

use std::sync::Arc;

use nice_plug::midi::{Channel, Key};
use nice_plug::prelude::*;

use crate::notes::Note;
use crate::params::CanWeTalkParams;
use crate::shifter::Shifter;

mod effects;
mod envelope;
mod grains;
mod marks;
mod notes;
mod params;
mod pitch;
mod ring;
mod shifter;
mod voice;

/// semitones the first channel's pitch bend reaches either way. it bends every note, as a
/// plain keyboard's wheel does and as the MPE master channel does
const MASTER_BEND_SEMITONES: f32 = 2.0;
/// the control change MPE sends a note's slide as
const SLIDE_CC: u8 = 74;
/// the largest sample taken for sound, a million times full scale. a fault upstream can send
/// larger ones or ones that are no numbers, which would overflow the sums taken over the
/// input and never leave the filters
const INPUT_LIMIT: f32 = 1e6;

pub struct CanWeTalk {
  params: Arc<CanWeTalkParams>,
  shifter: Shifter,
  sample_rate: f32,
}

impl Default for CanWeTalk {
  fn default() -> Self {
    // the host's rate arrives in `activate`, which builds the shifter again
    let sample_rate = 48_000.0;
    let params = Arc::<CanWeTalkParams>::default();
    Self {
      shifter: Shifter::new(sample_rate, params.detect.value()),
      params,
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
    self.shifter = Shifter::new(self.sample_rate, self.params.detect.value());
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
    let mono = buffer.channels() == 1;
    let block = self
      .params
      .block(context.transport(), self.sample_rate, mono);

    let mut next_event = context.next_event();
    for (index, mut samples) in buffer.iter_samples().enumerate() {
      let settings = self.params.settings(&block, index);
      // before the sample's notes, which are taken up in the mode they come in
      self.shifter.notes.set_play(&settings.play);
      while let Some(event) = next_event.filter(|event| event.timing() <= index as u32) {
        self.handle(event);
        next_event = context.next_event();
      }
      let mix = self.params.mix.smoothed.next();

      let mut dry = [0.0; 2];
      for (dry, sample) in dry.iter_mut().zip(samples.iter_mut()) {
        // false for a sample that is no number too, which is then silence
        if sample.abs() <= INPUT_LIMIT {
          *dry = *sample;
        }
      }
      if mono {
        dry[1] = dry[0];
      }
      let wet = self.shifter.process(dry, &settings);
      for ((sample, wet), dry) in samples.iter_mut().zip(wet).zip(dry) {
        *sample = dry + (wet - dry) * mix;
      }
    }

    ProcessStatus::Tail(self.shifter.tail() as u32)
  }
}

/// a host that names no channel for a note is not sending MPE, so the note goes on the first
/// channel with every other. one it names no key for is not a note to play
fn note(channel: Channel, key: Key) -> Option<Note> {
  Note::new(channel.number().unwrap_or(0), key.number()?)
}

impl CanWeTalk {
  /// passes on what the host sent about notes and their channels. the rest is ignored
  fn handle(&mut self, event: PluginNoteEvent<Self>) {
    let notes = &mut self.shifter.notes;
    match event {
      NoteEvent::NoteOn { channel, key, .. } if let Some(note) = note(channel, key) => {
        notes.on(note);
      }
      NoteEvent::NoteOff { channel, key, .. } if let Some(note) = note(channel, key) => {
        notes.off(note);
      }
      NoteEvent::PolyTuning {
        channel,
        key,
        tuning,
        ..
      } if let Some(note) = note(channel, key) => {
        notes.tune(note, f64::from(tuning));
      }
      NoteEvent::PolyPressure {
        channel,
        key,
        pressure,
        ..
      } if let Some(note) = note(channel, key) => {
        notes.note_pressure(note, f64::from(pressure));
      }
      NoteEvent::PolyBrightness {
        channel,
        key,
        brightness,
        ..
      } if let Some(note) = note(channel, key) => {
        notes.note_slide(note, f64::from(brightness));
      }
      NoteEvent::MidiChannelPressure {
        channel, pressure, ..
      } => {
        notes.channel_pressure(channel, f64::from(pressure));
      }
      NoteEvent::MidiCC {
        channel,
        cc: SLIDE_CC,
        value,
        ..
      } => {
        notes.channel_slide(channel, f64::from(value));
      }
      NoteEvent::MidiPitchBend { channel, value, .. } => {
        let range = match channel {
          0 => MASTER_BEND_SEMITONES,
          _ => self.params.expression.bend_range.value() as f32,
        };
        notes.bend(channel, f64::from((2.0 * value - 1.0) * range));
      }
      _ => (),
    }
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

#[cfg(test)]
mod tests {
  use std::collections::VecDeque;

  use nice_plug::context::process::SendEventError;

  use super::*;

  const RATE: f32 = 48_000.0;

  /// stands in for the host over one buffer
  struct Host {
    transport: Transport,
    events: VecDeque<PluginNoteEvent<CanWeTalk>>,
  }

  impl ProcessContext<CanWeTalk> for Host {
    fn plugin_api(&self) -> PluginApi {
      PluginApi::Clap
    }

    fn execute_background(&self, _task: ()) {}

    fn execute_gui(&self, _task: ()) {}

    fn transport(&self) -> &Transport {
      &self.transport
    }

    fn next_event(&mut self) -> Option<PluginNoteEvent<CanWeTalk>> {
      self.events.pop_front()
    }

    fn try_send_event(
      &mut self,
      event: PluginNoteEvent<CanWeTalk>,
    ) -> Result<(), (PluginNoteEvent<CanWeTalk>, SendEventError)> {
      Err((event, SendEventError::NoOutputBuffer))
    }

    fn set_latency_samples(&self, _samples: u32) {}

    fn request_restart(&self) {}

    fn set_current_voice_capacity(&self, _capacity: u32) {}
  }

  /// a MIDI message and the sample it comes at
  type Message = (usize, [u8; 3]);

  /// what a new plugin with its controls at their defaults makes of `input`, handed over in
  /// buffers of the sizes `sizes` gives, over and over
  fn render(input: &[[f32; 2]], messages: &[Message], sizes: &[usize]) -> Vec<[f32; 2]> {
    let mut plugin = CanWeTalk::default();
    plugin.sample_rate = RATE;
    plugin.shifter = Shifter::new(RATE, plugin.params.detect.value());
    let mut output = Vec::with_capacity(input.len());
    let mut sizes = sizes.iter().cycle();
    while output.len() < input.len() {
      let start = output.len();
      let frames = &input[start
        ..input
          .len()
          .min(start + sizes.next().expect("at least one size"))];
      let events = messages
        .iter()
        .filter(|(at, _)| (start..start + frames.len()).contains(at))
        .map(|(at, bytes)| {
          NoteEvent::from_midi((at - start) as u32, bytes).expect("a note, bend or pressure")
        });
      let mut host = Host {
        transport: Transport::new(RATE),
        events: events.collect(),
      };
      let mut channels = [0, 1].map(|side| frames.iter().map(|frame| frame[side]).collect());
      let [left, right]: &mut [Vec<f32>; 2] = &mut channels;
      let mut buffer = Buffer::default();
      // SAFETY: the slices are of `channels`, which outlives `buffer`, and are as long as each
      // other
      unsafe {
        buffer.set_slices(frames.len(), |slices| {
          *slices = vec![left.as_mut_slice(), right.as_mut_slice()];
        });
      }
      let mut aux = AuxiliaryBuffers {
        inputs: &mut [],
        outputs: &mut [],
      };
      plugin.process(&mut buffer, &mut aux, &mut host);
      let [left, right] = &channels;
      output.extend(left.iter().zip(right).map(|(&left, &right)| [left, right]));
    }
    output
  }

  /// a few harmonics of a low note, not the same on the two sides
  fn tone(samples: usize) -> Vec<[f32; 2]> {
    let harmonic = |index: usize, number: f32| {
      (std::f32::consts::TAU * (index as f32 * number * 110.0 / RATE).fract()).sin() / number
    };
    (0..samples)
      .map(|index| {
        let low = harmonic(index, 1.0) + harmonic(index, 2.0);
        [low + harmonic(index, 3.0), low + harmonic(index, 5.0)]
      })
      .collect()
  }

  /// notes, a bend and pressure that come at samples no buffer size here divides
  const PLAYED: [Message; 7] = [
    (3001, [0x90, 57, 100]),
    (7777, [0xe0, 0, 0x50]),
    (9001, [0xd0, 90, 0]),
    (12_345, [0x90, 64, 100]),
    (20_011, [0x80, 57, 0]),
    (30_003, [0xe0, 0, 0x40]),
    (40_001, [0x80, 64, 0]),
  ];

  #[test]
  fn output_does_not_depend_on_how_the_host_splits_buffers() {
    let input = tone(RATE as usize);
    let whole = render(&input, &PLAYED, &[input.len()]);
    for sizes in [
      &[1][..],
      &[32],
      &[64],
      &[512],
      &[4096],
      &[1, 7, 480, 33, 2048, 129],
    ] {
      assert!(
        render(&input, &PLAYED, sizes) == whole,
        "buffers of {sizes:?} samples sound different"
      );
    }
  }

  /// a fault upstream can send samples that are no numbers or far too large to be sound. the
  /// output stays a number through them, and a note played after them sounds
  #[test]
  fn samples_that_are_not_sound_leave_nothing_behind() {
    let mut input = tone(RATE as usize);
    for (at, sample) in [(12_000, f32::NAN), (12_100, f32::INFINITY), (12_200, -1e30)] {
      input[at] = [sample; 2];
    }
    let note = [(RATE as usize / 2, [0x90, 64, 100])];
    let output = render(&input, &note, &[512]);
    let lost = output.iter().flatten().position(|x| !x.is_finite());
    assert_eq!(lost, None, "the output is not a number from there");
    let last = &output[output.len() - 4800..];
    assert!(
      last.iter().flatten().any(|x| x.abs() > 0.1),
      "the note is silent"
    );
  }
}
