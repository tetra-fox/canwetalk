//! canwetalk, a pitch and formant shifter for a monophonic input, played by MIDI notes

use std::sync::Arc;

use nice_plug::midi::{Channel, Key};
use nice_plug::plugin::ParamValue;
use nice_plug::prelude::*;
use potpot_window::Feed;

use crate::notes::{Note, VOICES};
use crate::params::CanWeTalkParams;
use crate::pitch::Range;
use crate::shifter::{BLOCK, Settings, Shifter};
use crate::spectrum::{Analyzer, INPUT_BANDS, OUTPUT_BANDS, SLOTS, write_played};

mod editor;
mod effects;
mod envelope;
mod grains;
mod last;
mod marks;
mod notes;
mod params;
mod pitch;
mod ring;
mod shifter;
#[cfg(test)]
mod signals;
mod spectrum;
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
  /// the spectra of the input and of the output, after the mix
  heard: Analyzer,
  made: Analyzer,
  /// what the editor draws the spectra, the detected pitch and the voices' pitches from
  feed: Feed,
  sample_rate: f32,
  /// the settings for each sample of a block, kept only for the room it has for them
  settings: Vec<Settings>,
}

impl Default for CanWeTalk {
  fn default() -> Self {
    // the host's rate arrives in `activate`, which builds the shifter and the analyzers again
    let sample_rate = 48_000.0;
    let params = Arc::<CanWeTalkParams>::default();
    let feed = Feed::new(SLOTS);
    Self {
      shifter: Shifter::new(sample_rate, params.detect.value()),
      heard: Analyzer::new(sample_rate, INPUT_BANDS, &feed),
      made: Analyzer::new(sample_rate, OUTPUT_BANDS, &feed),
      feed,
      params,
      sample_rate,
      settings: Vec::with_capacity(BLOCK),
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

  type Editor = editor::Editor;
  type SysExMessage = ();
  type BackgroundTask = ();

  fn filter_state(state: &mut PluginState) {
    // a session saved before the detection ranges had ids holds the index of one in the order
    // they had then. the key is the id the parameter has in `params`
    const SAVED_ORDER: [Range; 3] = [Range::High, Range::Mid, Range::Bass];
    let Some(value) = state.params.get_mut("detect") else {
      return;
    };
    if let ParamValue::I32(index) = *value
      && let Some(range) = usize::try_from(index)
        .ok()
        .and_then(|at| SAVED_ORDER.get(at))
    {
      let ids = Range::ids().expect("the detection ranges have ids");
      *value = ParamValue::String(ids[range.to_index()].to_owned());
    }
  }

  fn params(&self) -> Arc<dyn Params> {
    self.params.clone()
  }

  fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Self::Editor> {
    editor::editor(self.params.clone(), self.feed.clone())
      .inspect_err(|error| nice_error!("the plugin has no editor: {error}"))
      .ok()
  }

  fn activate(
    &mut self,
    _audio_io_layout: &AudioIOLayout,
    buffer_config: &BufferConfig,
    _context: &mut impl ActivateContext<Self>,
  ) -> bool {
    self.sample_rate = buffer_config.sample_rate;
    self.shifter = Shifter::new(self.sample_rate, self.params.detect.value());
    self.heard = Analyzer::new(self.sample_rate, INPUT_BANDS, &self.feed);
    self.made = Analyzer::new(self.sample_rate, OUTPUT_BANDS, &self.feed);
    true
  }

  fn reset(&mut self) {
    self.shifter.reset();
    self.heard.reset(&self.feed);
    self.made.reset(&self.feed);
    write_played(&self.feed, None, None, &[0.0; VOICES]);
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

    let samples = buffer.samples();
    let channels = buffer.as_slice();
    let mut next_event = context.next_event();
    let mut start = 0;
    while start < samples {
      let first = self.params.settings(&block, start);
      // before the sample's notes, which are taken up in the mode they come in. the mode and
      // latch are not smoothed, so they stay as they are to the end of the buffer
      self.shifter.notes.set_play(&first.play);
      while let Some(event) = next_event.filter(|event| event.timing() <= start as u32) {
        self.handle(event);
        next_event = context.next_event();
      }
      // a block goes up to the next sample that has something played at it
      let played = next_event.map_or(samples, |event| event.timing() as usize);
      let count = BLOCK.min(samples.min(played) - start);
      self.settings.clear();
      self.settings.push(first);
      for index in start + 1..start + count {
        self.settings.push(self.params.settings(&block, index));
      }

      let mut dry = [[0.0; 2]; BLOCK];
      for (index, dry) in dry[..count].iter_mut().enumerate() {
        for (dry, channel) in dry.iter_mut().zip(channels.iter()) {
          let sample = channel[start + index];
          // false for a sample that is no number too, which is then silence
          if sample.abs() <= INPUT_LIMIT {
            *dry = sample;
          }
        }
        if mono {
          dry[1] = dry[0];
        }
      }
      let mut wet = dry;
      self.shifter.process(&mut wet[..count], &self.settings);
      let mut mixed = [[0.0; 2]; BLOCK];
      let frames = dry.iter().zip(wet).zip(&mut mixed).take(count);
      for (index, ((dry, wet), mixed)) in frames.enumerate() {
        let mix = self.params.mix.smoothed.next();
        let sides = channels.iter_mut().zip(dry).zip(wet).zip(mixed.iter_mut());
        for (((channel, dry), wet), mixed) in sides {
          *mixed = dry + (wet - dry) * mix;
          channel[start + index] = *mixed;
        }
        if mono {
          mixed[1] = mixed[0];
        }
      }
      self.heard.take(&dry[..count], &self.feed);
      self.made.take(&mixed[..count], &self.feed);
      let (pitch, followed) = (self.shifter.detected_hz(), self.shifter.formant_followed());
      write_played(&self.feed, pitch, followed, &self.shifter.made_hz());
      start += count;
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
  const VST3_CLASS_ID: [u8; 16] = [
    0xa4, 0x67, 0xeb, 0x81, 0x0b, 0x6f, 0x76, 0x91, 0x6e, 0xff, 0x73, 0xa8, 0xe6, 0x40, 0x94, 0xfa,
  ];
  const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
    &[Vst3SubCategory::Fx, Vst3SubCategory::PitchShift];
}

nice_export_clap!(CanWeTalk);
nice_export_vst3!(CanWeTalk);

#[cfg(test)]
mod tests {
  use std::collections::VecDeque;

  use nice_plug::context::process::SendEventError;
  use nice_plug::midi::VoiceID;
  use nice_plug::params::internals::ParamPtr;

  use super::*;
  use crate::signals::vowel_sampled_at;

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

  /// something a host sends about a note or a channel
  #[derive(Clone, Copy)]
  enum Played {
    Midi([u8; 3]),
    /// what a host sends for one note of a channel and a key: semitones of tuning, pressure
    /// and brightness, which is the slide
    Tuning(u8, u8, f32),
    Pressure(u8, u8, f32),
    Brightness(u8, u8, f32),
  }

  impl Played {
    fn event(self, timing: u32) -> PluginNoteEvent<CanWeTalk> {
      let voice_id = VoiceID::Wildcard;
      let of = |channel, key| (Channel::Number(channel), Key::Number(key));
      match self {
        Self::Midi(bytes) => {
          NoteEvent::from_midi(timing, &bytes).expect("a message the plugin takes")
        }
        Self::Tuning(channel, key, tuning) => {
          let (channel, key) = of(channel, key);
          NoteEvent::PolyTuning {
            timing,
            voice_id,
            channel,
            key,
            tuning,
          }
        }
        Self::Pressure(channel, key, pressure) => {
          let (channel, key) = of(channel, key);
          NoteEvent::PolyPressure {
            timing,
            voice_id,
            channel,
            key,
            pressure,
          }
        }
        Self::Brightness(channel, key, brightness) => {
          let (channel, key) = of(channel, key);
          NoteEvent::PolyBrightness {
            timing,
            voice_id,
            channel,
            key,
            brightness,
          }
        }
      }
    }
  }

  /// what a host does with a new plugin over one render. times are in milliseconds
  struct Take {
    rate: f32,
    /// one channel in and out, else two
    mono: bool,
    input: Vec<[f32; 2]>,
    played: Vec<(usize, Played)>,
    /// controls set while it runs, by their id and to their plain value. a host hands a
    /// change over with the buffer that starts at it
    set: Vec<(usize, &'static str, f32)>,
    /// beats per minute and whether the host plays
    tempo: Option<(f64, bool)>,
  }

  impl Take {
    fn new(rate: f32, input: Vec<[f32; 2]>) -> Self {
      Self {
        rate,
        mono: false,
        input,
        played: Vec::new(),
        set: Vec::new(),
        tempo: None,
      }
    }

    /// the sample a time in milliseconds is at. off the round numbers that buffers start at
    fn sample(&self, milliseconds: usize) -> usize {
      milliseconds * self.rate as usize / 1000 + 1
    }
  }

  /// what the plugin makes of `take`, handed over in buffers of the sizes `sizes` gives, over
  /// and over, and cut short where a control is set
  fn render(take: &Take, sizes: &[usize]) -> Vec<[f32; 2]> {
    run(take, sizes).1
  }

  /// the plugin after `take`, as `render` plays it, and what it made
  fn run(take: &Take, sizes: &[usize]) -> (CanWeTalk, Vec<[f32; 2]>) {
    let Take {
      rate, ref input, ..
    } = *take;
    let mut plugin = CanWeTalk::default();
    let controls = plugin.params.param_map();
    for (_, control, _) in &controls {
      // SAFETY: the control is in `plugin.params`, which lives to the end of this function
      unsafe { control._internal_update_smoother(rate, true) };
    }
    plugin.sample_rate = rate;
    plugin.shifter = Shifter::new(rate, plugin.params.detect.value());
    plugin.heard = Analyzer::new(rate, INPUT_BANDS, &plugin.feed);
    plugin.made = Analyzer::new(rate, OUTPUT_BANDS, &plugin.feed);
    let mut output = Vec::with_capacity(input.len());
    let mut sizes = sizes.iter().cycle();
    while output.len() < input.len() {
      let start = output.len();
      let mut end = input
        .len()
        .min(start + sizes.next().expect("at least one size"));
      for &(at, id, plain) in &take.set {
        let at = take.sample(at);
        if at == start {
          let (_, control, _) = (controls.iter())
            .find(|control| control.0 == id)
            .expect("a control of that id");
          // SAFETY: as above
          unsafe {
            if control._internal_set_normalized_value(control.preview_normalized(plain)) {
              control._internal_update_smoother(rate, false);
            }
          }
        } else if at > start {
          end = end.min(at);
        }
      }
      let frames = &input[start..end];
      let events = take
        .played
        .iter()
        .map(|&(at, played)| (take.sample(at), played))
        .filter(|(at, _)| (start..end).contains(at))
        .map(|(at, played)| played.event((at - start) as u32));
      let mut transport = Transport::new(rate);
      if let Some((tempo, playing)) = take.tempo {
        transport.tempo = Some(tempo);
        transport.playing = playing;
        transport.pos_beats = Some(start as f64 / f64::from(rate) / 60.0 * tempo);
      }
      let mut host = Host {
        transport,
        events: events.collect(),
      };
      let mut channels = [0, 1].map(|side| frames.iter().map(|frame| frame[side]).collect());
      let [left, right]: &mut [Vec<f32>; 2] = &mut channels;
      let mut buffer = Buffer::default();
      // SAFETY: the slices are of `channels`, which outlives `buffer`, and are as long as each
      // other
      unsafe {
        buffer.set_slices(frames.len(), |slices| {
          *slices = if take.mono {
            vec![left.as_mut_slice()]
          } else {
            vec![left.as_mut_slice(), right.as_mut_slice()]
          };
        });
      }
      let mut aux = AuxiliaryBuffers {
        inputs: &mut [],
        outputs: &mut [],
      };
      plugin.process(&mut buffer, &mut aux, &mut host);
      let [left, right] = &channels;
      // one channel is heard on both sides
      let right = if take.mono { left } else { right };
      output.extend(left.iter().zip(right).map(|(&left, &right)| [left, right]));
    }
    (plugin, output)
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

  /// a voice about `hz` whose pitch rises and falls, with a gap of silence and then a burst
  /// of noise in it, and not the same on the two sides
  fn voice(rate: f32, seconds: f32, hz: f32) -> Vec<[f32; 2]> {
    let samples = (rate * seconds) as usize;
    let wander = |index: usize| {
      let turn = (1.7 * index as f32 / rate).fract();
      hz * 2f32.powf(0.1 * (std::f32::consts::TAU * turn).sin())
    };
    let mut voice = vowel_sampled_at(rate, samples, wander);
    let mut state = 0x2545_f491_u32;
    for (index, sample) in voice.iter_mut().enumerate() {
      state ^= state << 13;
      state ^= state >> 17;
      state ^= state << 5;
      *sample = match 100 * index / samples {
        40..46 => 0.0,
        46..52 => 0.2 * (state as f32 / u32::MAX as f32 - 0.5),
        _ => 0.1 * *sample,
      };
    }
    let behind = |index: usize| voice[index.saturating_sub(29)];
    (0..samples)
      .map(|index| [voice[index], 0.6 * voice[index] + 0.4 * behind(index)])
      .collect()
  }

  fn on(at: usize, channel: u8, key: u8) -> (usize, Played) {
    (at, Played::Midi([0x90 | channel, key, 100]))
  }

  fn off(at: usize, channel: u8, key: u8) -> (usize, Played) {
    (at, Played::Midi([0x80 | channel, key, 0]))
  }

  /// notes, a bend and pressure
  const PLAYED: [(usize, Played); 7] = [
    (62, Played::Midi([0x90, 57, 100])),
    (162, Played::Midi([0xe0, 0, 0x50])),
    (187, Played::Midi([0xd0, 90, 0])),
    (257, Played::Midi([0x90, 64, 100])),
    (416, Played::Midi([0x80, 57, 0])),
    (625, Played::Midi([0xe0, 0, 0x40])),
    (833, Played::Midi([0x80, 64, 0])),
  ];

  /// four notes that start and stop one after another
  fn chord() -> Vec<(usize, Played)> {
    vec![
      on(50, 0, 57),
      on(120, 0, 60),
      on(200, 0, 64),
      on(330, 0, 67),
      off(500, 0, 57),
      off(620, 0, 64),
      off(700, 0, 60),
      off(900, 0, 67),
    ]
  }

  /// every take the reference has, by name
  fn takes() -> Vec<(&'static str, Take)> {
    let plain = |rate: f32, seconds: f32| Take::new(rate, voice(rate, seconds, 110.0));
    let second = || plain(RATE, 1.0);
    let mut takes = vec![
      ("follow", second()),
      (
        "follow shifted",
        Take {
          set: vec![(0, "pitch", 4.0), (0, "formant", -3.0)],
          ..second()
        },
      ),
      (
        "follow mono",
        Take {
          mono: true,
          set: vec![(0, "formant", 4.0)],
          ..second()
        },
      ),
      (
        "follow unanchored",
        Take {
          set: vec![(0, "anchor", 0.0), (0, "pitch", -5.0)],
          ..second()
        },
      ),
      (
        "pitch",
        Take {
          set: vec![
            (0, "pitch", 7.0),
            (0, "formant", -5.0),
            (400, "pitch", -12.0),
            (450, "pitch", 3.0),
            (700, "pitch", 0.0),
          ],
          ..second()
        },
      ),
      (
        "formant",
        Take {
          played: vec![on(100, 0, 52), off(900, 0, 52)],
          set: vec![
            (0, "formant", 12.0),
            (300, "formant", -12.0),
            (300, "formant_follow", 0.5),
            (600, "pitch", -24.0),
            (600, "smooth", 1.0),
          ],
          ..second()
        },
      ),
      (
        "chord",
        Take {
          played: chord(),
          ..second()
        },
      ),
      (
        "chord unanchored",
        Take {
          played: chord(),
          set: vec![(0, "anchor", 0.0), (0, "pitch", 5.0)],
          ..second()
        },
      ),
      (
        "more notes than voices",
        Take {
          played: (0..18)
            .flat_map(|note| {
              let key = 40 + note as u8;
              [on(20 + 10 * note, 0, key), off(600 + 15 * note, 0, key)]
            })
            .collect(),
          ..second()
        },
      ),
      (
        "envelope",
        Take {
          played: chord(),
          set: vec![
            (0, "midi", 4.0),
            (0, "attack", 50.0),
            (0, "decay", 100.0),
            (0, "sustain", 0.5),
            (0, "release", 300.0),
            (400, "sustain", 0.8),
          ],
          ..second()
        },
      ),
      (
        "envelope with no times",
        Take {
          played: chord(),
          set: vec![
            (0, "midi", 3.0),
            (0, "attack", 0.0),
            (0, "decay", 0.0),
            (0, "sustain", 0.3),
            (0, "release", 0.0),
          ],
          ..second()
        },
      ),
      (
        "latch",
        Take {
          played: vec![
            on(100, 0, 57),
            off(200, 0, 57),
            on(300, 0, 60),
            on(320, 0, 67),
            off(400, 0, 60),
            off(410, 0, 67),
            on(600, 0, 50),
            off(650, 0, 50),
          ],
          set: vec![(0, "latch", 1.0), (800, "latch", 0.0)],
          ..second()
        },
      ),
      (
        "glide",
        Take {
          played: PLAYED.to_vec(),
          set: vec![(0, "midi", 1.0), (0, "glide", 0.5), (500, "glide", 0.2)],
          ..second()
        },
      ),
      (
        "glide between notes only",
        Take {
          played: PLAYED.to_vec(),
          set: vec![(0, "glide", 0.4), (0, "glide_notes", 1.0)],
          ..second()
        },
      ),
      (
        "expression",
        Take {
          played: vec![
            (40, Played::Midi([0xd1, 30, 0])),
            (45, Played::Midi([0xb2, 74, 100])),
            on(50, 1, 57),
            on(100, 2, 64),
            on(150, 3, 69),
            (200, Played::Midi([0xe1, 0, 0x60])),
            (250, Played::Midi([0xd1, 110, 0])),
            (300, Played::Midi([0xb2, 74, 10])),
            (350, Played::Midi([0xe0, 0, 0x30])),
            (400, Played::Midi([0xa3, 69, 90])),
            (450, Played::Tuning(2, 64, 0.5)),
            (500, Played::Pressure(1, 57, 0.4)),
            (550, Played::Brightness(3, 69, 0.9)),
            (600, Played::Midi([0xe2, 0x10, 0x20])),
            off(700, 2, 64),
            (750, Played::Midi([0xb1, 74, 127])),
            off(800, 1, 57),
            off(850, 3, 69),
          ],
          set: vec![
            (0, "pressure_level", 1.0),
            (0, "slide_formant", 12.0),
            (0, "bend_range", 12.0),
            (650, "pressure_level", 0.5),
          ],
          ..second()
        },
      ),
      (
        "vibrato",
        Take {
          played: vec![on(300, 0, 57), off(900, 0, 57)],
          set: vec![
            (0, "vibrato", 100.0),
            (0, "vibrato_rate", 6.0),
            (700, "vibrato", 0.0),
          ],
          ..second()
        },
      ),
      (
        "vibrato by the tempo",
        Take {
          played: vec![on(100, 0, 57)],
          set: vec![
            (0, "vibrato", 50.0),
            (0, "vibrato_sync", 1.0),
            (0, "vibrato_division", 4.0),
          ],
          tempo: Some((120.0, true)),
          ..second()
        },
      ),
      (
        "vibrato by the tempo of a stopped host",
        Take {
          played: vec![on(100, 0, 57)],
          set: vec![(0, "vibrato", 50.0), (0, "vibrato_sync", 1.0)],
          tempo: Some((97.0, false)),
          ..second()
        },
      ),
      (
        "effects",
        Take {
          played: chord(),
          set: vec![
            (0, "detune", 0.6),
            (0, "smear", 0.4),
            (0, "stereo", 0.7),
            (400, "smear", 0.0),
            (600, "smear", 1.0),
            (700, "detune", 0.0),
            (850, "detune", 1.0),
          ],
          ..second()
        },
      ),
      (
        "effects mono",
        Take {
          mono: true,
          played: chord(),
          set: vec![(0, "detune", 0.6), (0, "smear", 0.4), (0, "stereo", 0.7)],
          ..second()
        },
      ),
      (
        "mix",
        Take {
          played: chord(),
          set: vec![(0, "mix", 0.5)],
          ..second()
        },
      ),
      (
        "bass",
        Take {
          played: vec![on(300, 0, 33)],
          set: vec![(0, "detect", 0.0)],
          ..Take::new(RATE, voice(RATE, 1.0, 45.0))
        },
      ),
      (
        "bass at 192 kHz",
        Take {
          played: vec![on(300, 0, 33)],
          set: vec![(0, "detect", 0.0)],
          ..Take::new(192_000.0, voice(192_000.0, 0.75, 45.0))
        },
      ),
      (
        "high",
        Take {
          played: vec![on(300, 0, 81), on(400, 0, 93)],
          set: vec![(0, "detect", 2.0)],
          ..Take::new(RATE, voice(RATE, 1.0, 440.0))
        },
      ),
      (
        "detection range changes",
        Take {
          played: chord(),
          set: vec![(500, "detect", 0.0)],
          ..second()
        },
      ),
    ];
    for (name, mode) in [
      ("mode off", 0.0),
      ("mode mono", 1.0),
      ("mode poly", 2.0),
      ("mode mono gate", 3.0),
      ("mode poly gate", 4.0),
    ] {
      let take = Take {
        played: PLAYED.to_vec(),
        // on to the next mode with the last note still down
        set: vec![
          (0, "formant", 2.0),
          (0, "midi", mode),
          (700, "midi", (mode + 1.0) % 5.0),
        ],
        ..second()
      };
      takes.push((name, take));
    }
    for (name, rate) in [
      ("chord at 44.1 kHz", 44_100.0),
      ("chord at 96 kHz", 96_000.0),
      ("chord at 192 kHz", 192_000.0),
    ] {
      let take = Take {
        played: chord()
          .into_iter()
          .map(|(at, played)| (at / 2, played))
          .collect(),
        set: vec![(0, "detune", 0.3), (0, "stereo", 0.4), (0, "formant", 3.0)],
        ..plain(rate, 0.5)
      };
      takes.push((name, take));
    }
    takes.push((
      "four seconds of everything",
      Take {
        played: (0..4)
          .flat_map(|second| {
            let later = move |(at, played)| (at + 1000 * second, played);
            chord().into_iter().map(later)
          })
          .collect(),
        set: vec![
          (0, "pitch", -3.0),
          (0, "formant", 2.0),
          (0, "vibrato", 30.0),
          (0, "glide", 0.3),
          (0, "detune", 0.2),
          (0, "smear", 0.1),
          (0, "stereo", 0.3),
          (2000, "pitch", 2.0),
        ],
        ..plain(RATE, 4.0)
      },
    ));
    takes.push((
      "unanchored at 96 kHz",
      Take {
        played: vec![on(100, 0, 60)],
        set: vec![(0, "anchor", 0.0), (0, "smear", 0.3)],
        ..plain(96_000.0, 0.5)
      },
    ));
    takes
  }

  /// buffer sizes the takes are rendered in, one after another
  const SIZES: [&[usize]; 6] = [
    &[512],
    &[64],
    &[1, 7, 480, 33, 2048, 129],
    &[32],
    &[4096],
    &[96],
  ];

  /// FNV-1a over the bits of every sample
  fn hash(output: &[[f32; 2]]) -> u64 {
    let bytes = output
      .iter()
      .flatten()
      .flat_map(|x| x.to_bits().to_le_bytes());
    bytes.fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
      (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
  }

  /// hashes of what the plugin made of each take when the reference was taken. they hold for
  /// one platform's math library, whose last bit is not the same everywhere
  const REFERENCE: [(&str, u64); 35] = [
    ("follow", 0xd730fc61f7b1501b),
    ("follow shifted", 0x092689eb1764fd78),
    ("follow mono", 0x14ff7c05d32b9d19),
    ("follow unanchored", 0x9b5e50b62d29b155),
    ("pitch", 0xcbe3b07907b78eb0),
    ("formant", 0x11bc814531417388),
    ("chord", 0xdccfde8fa97fe2e8),
    ("chord unanchored", 0xd5b14b11ec99ee39),
    ("more notes than voices", 0x74104c49d8a9f71e),
    ("envelope", 0x70e99783f0d46c07),
    ("envelope with no times", 0x5005037837873986),
    ("latch", 0x71d1f72f422aea12),
    ("glide", 0xdf488dafc78aecc5),
    ("glide between notes only", 0x51becb177cbfec41),
    ("expression", 0x3d99a8cab4600148),
    ("vibrato", 0xd71839b0320faa6b),
    ("vibrato by the tempo", 0x98ac99cdf5b06e80),
    ("vibrato by the tempo of a stopped host", 0x368e84c6c5f64531),
    ("effects", 0xc2f20bcf1d7ccbc0),
    ("effects mono", 0xad3c8b949deb0ebd),
    ("mix", 0x7d825a8372ea69d2),
    ("bass", 0x98de606d6873eb6d),
    ("bass at 192 kHz", 0x4fabe5b589ba4385),
    ("high", 0xe13887e09018a198),
    ("detection range changes", 0x01c8be0f096f06e0),
    ("mode off", 0xb7ba8746aedef7c7),
    ("mode mono", 0xbd5308573262f058),
    ("mode poly", 0x9cbe1e2f4df0b1b1),
    ("mode mono gate", 0x37ac0e270e4faf54),
    ("mode poly gate", 0x81803f9d8e481493),
    ("chord at 44.1 kHz", 0xbd8936b1f5311c2d),
    ("chord at 96 kHz", 0x6b3de87d237cae8a),
    ("chord at 192 kHz", 0xb8ec1981f0b4cb89),
    ("four seconds of everything", 0x3cbe39590ac92270),
    ("unanchored at 96 kHz", 0x118d86f53def9c11),
  ];

  /// every sample of every take is what it was when the reference was taken, in buffers of
  /// two different sets of sizes. a change that is meant to move the output takes the
  /// reference again, from what this prints
  #[test]
  fn output_is_the_reference() {
    let takes = takes();
    let hashes: Vec<[u64; 2]> = std::thread::scope(|scope| {
      let renders: Vec<_> = (takes.iter().enumerate())
        .map(|(index, (name, take))| {
          [0, SIZES.len() / 2].map(|other| {
            scope.spawn(move || {
              let output = render(take, SIZES[(index + other) % SIZES.len()]);
              if other == 0 {
                keep(name, &output);
              }
              hash(&output)
            })
          })
        })
        .collect();
      (renders.into_iter())
        .map(|renders| renders.map(|render| render.join().expect("the render ran")))
        .collect()
    });
    for ((name, _), [hash, other]) in takes.iter().zip(&hashes) {
      assert!(hash == other, "{name} depends on the sizes of the buffers");
    }
    let now: Vec<_> = (takes.iter().zip(hashes))
      .map(|((name, _), [hash, _])| (*name, hash))
      .collect();
    if now != REFERENCE {
      let lines: Vec<_> = (now.iter())
        .map(|line @ (name, hash)| {
          let moved = if REFERENCE.contains(line) {
            ""
          } else {
            " // moved"
          };
          format!("    ({name:?}, {hash:#018x}),{moved}")
        })
        .collect();
      panic!(
        "the output is not the reference. it is now\n{}",
        lines.join("\n")
      );
    }
  }

  /// with CANWETALK_RENDERS naming a directory, writes `output` there under `name`, or if it
  /// is there already says how far `output` is from it, against the loudest sample of it
  fn keep(name: &str, output: &[[f32; 2]]) {
    let Some(directory) = std::env::var_os("CANWETALK_RENDERS") else {
      return;
    };
    let path = std::path::Path::new(&directory).join(name);
    let bytes: Vec<u8> = (output.iter().flatten())
      .flat_map(|x| x.to_le_bytes())
      .collect();
    let Ok(kept) = std::fs::read(&path) else {
      std::fs::write(&path, bytes).expect("the directory can be written to");
      return;
    };
    let kept = kept.as_chunks().0.iter().map(|&x| f32::from_le_bytes(x));
    let (mut loudest, mut furthest, mut moved) = (0f32, 0f32, 0);
    for (was, is) in kept.zip(output.iter().flatten()) {
      loudest = loudest.max(was.abs());
      furthest = furthest.max((was - is).abs());
      moved += usize::from(was.to_bits() != is.to_bits());
    }
    println!(
      "{name}: {moved} of {} samples moved, at most {:.1} dB under the loudest",
      2 * output.len(),
      -20.0 * (furthest / loudest).log10()
    );
  }

  /// draws the editor after a voice has played through the plugin under four held notes,
  /// shifted, so that its screen shows the spectra of the voice and of the output, the voice's
  /// note and the notes played, and the ring around the formant what formant follow adds.
  /// `just picture` writes it to a png file
  #[test]
  #[ignore = "needs a graphics device"]
  fn the_editor_draws_what_the_plugin_hears() {
    let take = Take {
      played: vec![
        on(200, 0, 57),
        on(300, 0, 60),
        on(400, 0, 64),
        on(500, 0, 67),
      ],
      set: vec![
        (0, "pitch", 7.0),
        (0, "formant", -5.0),
        (0, "formant_follow", 0.8),
      ],
      ..Take::new(RATE, voice(RATE, 1.0, 220.0))
    };
    let (plugin, _) = run(&take, &[512]);
    let editor = editor::editor(plugin.params.clone(), plugin.feed.clone())
      .expect("every control is bound to one of the plugin's parameters");
    let picture = editor.picture(2.0).expect("the editor is drawn");
    let Some(path) = std::env::var_os("CANWETALK_PICTURE") else {
      return;
    };
    let file = std::fs::File::create(path).expect("the png file can be made");
    let [width, height] = picture.size;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("the png header is written");
    (writer.write_image_data(&picture.pixels)).expect("the png pixels are written");
  }

  /// what the ring around the formant shows: the semitones formant follow adds to the formants
  /// of the voice that started last, which with no note is the one following the input
  #[test]
  fn formant_follow_is_published_as_the_shift_it_adds() {
    let set = |midi| {
      vec![
        (0, "midi", midi),
        (0, "pitch", 7.0),
        (0, "formant", -5.0),
        (0, "formant_follow", 0.8),
      ]
    };
    let take = |set| Take {
      set,
      ..Take::new(RATE, tone(RATE as usize / 2))
    };
    let (plugin, _) = run(&take(set(2.0)), &[512]);
    let followed = (plugin.shifter.formant_followed()).expect("the voice following the input");
    assert!((followed - 0.8 * 7.0).abs() < 1e-3, "{followed}");
    // in a gate mode with no note held nothing sounds
    let (gated, _) = run(&take(set(3.0)), &[512]);
    assert_eq!(gated.shifter.formant_followed(), None);
  }

  /// a session saved while the detection ranges were kept by their index in the order high,
  /// mid, bass loads with the range it was saved with
  #[test]
  fn an_old_session_keeps_its_detection_range() {
    for (index, range) in [(0, Range::High), (1, Range::Mid), (2, Range::Bass)] {
      let mut state = PluginState {
        version: String::new(),
        params: [("detect".to_owned(), ParamValue::I32(index))].into(),
        fields: Default::default(),
      };
      CanWeTalk::filter_state(&mut state);
      let ParamValue::String(id) = &state.params["detect"] else {
        panic!("{index} is not taken to an id");
      };
      // what nice-plug does with an id it loads for an enum parameter, to the parameter itself
      let params = CanWeTalkParams::default();
      let ParamPtr::EnumParam(detect) = params.detect.as_ptr() else {
        panic!("the detection range is an enum parameter");
      };
      // SAFETY: the pointer is into `params`, which lives to the end of the loop's turn
      assert!(unsafe { (*detect).set_from_id(id) }, "{id} is an id");
      assert_eq!(params.detect.value(), range, "{index} loads as {id}");
    }
  }

  /// each sounding voice is published at the pitch it makes, the shift and all, the latest
  /// note first
  #[test]
  fn the_voices_are_published_at_the_pitches_they_make() {
    let take = Take {
      played: vec![on(50, 0, 57), on(100, 0, 64)],
      set: vec![(0, "pitch", 7.0)],
      ..Take::new(RATE, tone(RATE as usize / 2))
    };
    let (plugin, _) = run(&take, &[512]);
    let made = plugin.shifter.made_hz();
    let shifted = |key: f32| 440.0 * 2f32.powf((key + 7.0 - 69.0) / 12.0);
    for (made, key) in made.iter().zip([64.0, 57.0]) {
      assert!((made - shifted(key)).abs() < 0.01, "{made} for key {key}");
    }
    assert!(made[2..].iter().all(|&hz| hz == 0.0), "{made:?}");
  }

  /// with no note held and no pitch in the input, no voice plays: the one that follows the
  /// input still sounds at the pitch it last had, but nothing of it is published
  #[test]
  fn a_silent_input_with_no_note_publishes_no_voice() {
    let take = Take {
      set: vec![(0, "pitch", -24.0), (0, "formant_follow", 0.8)],
      ..Take::new(RATE, vec![[0.0; 2]; RATE as usize / 2])
    };
    let (plugin, _) = run(&take, &[512]);
    assert_eq!(plugin.shifter.detected_hz(), None);
    assert_eq!(plugin.shifter.made_hz(), [0.0; VOICES]);
    assert_eq!(plugin.shifter.formant_followed(), None);
  }

  #[test]
  fn output_does_not_depend_on_how_the_host_splits_buffers() {
    let take = Take {
      played: PLAYED.to_vec(),
      ..Take::new(RATE, tone(RATE as usize))
    };
    let whole = render(&take, &[take.input.len()]);
    for sizes in [
      &[1][..],
      &[32],
      &[64],
      &[512],
      &[4096],
      &[1, 7, 480, 33, 2048, 129],
    ] {
      assert!(
        render(&take, sizes) == whole,
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
    let take = Take {
      played: vec![on(500, 0, 64)],
      ..Take::new(RATE, input)
    };
    let output = render(&take, &[512]);
    let lost = output.iter().flatten().position(|x| !x.is_finite());
    assert_eq!(lost, None, "the output is not a number from there");
    let last = &output[output.len() - 4800..];
    assert!(
      last.iter().flatten().any(|x| x.abs() > 0.1),
      "the note is silent"
    );
  }
}
