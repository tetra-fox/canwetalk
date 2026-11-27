//! canwetalk, a pitch and formant shifter for a monophonic input, played by MIDI notes

use std::collections::VecDeque;
use std::sync::Arc;

use nice_plug::midi::{Channel, Key};
use nice_plug::plugin::ParamValue;
use nice_plug::prelude::*;
use potpot_window::Feed;

use crate::notes::{Note, VOICES};
use crate::params::CanWeTalkParams;
use crate::pitch::Range;
use crate::shifter::{BLOCK, Settings, Shifter};
use crate::spectrum::{
  Analyzer, INPUT_BANDS, INPUT_LEVEL, OUTPUT_BANDS, OUTPUT_LEVEL, SLOTS, write_played,
};

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
/// notes and the like that can wait at once to be played, which they do while the input they
/// came with is still in the lookahead. as many as a host can hand over with four buffers
const WAITING_EVENTS: usize = 4 * CanWeTalk::INPUT_EVENT_CAPACITY;

pub struct CanWeTalk {
  params: Arc<CanWeTalkParams>,
  shifter: Shifter,
  /// the spectra of the input as the effect hears it and of the output, after the mix, so the
  /// two are shown at the same time
  heard: Analyzer,
  made: Analyzer,
  /// what the editor draws the spectra, the detected pitch and the voices' pitches from
  feed: Feed,
  sample_rate: f32,
  /// the settings for each sample of a block, kept only for the room it has for them
  settings: Vec<Settings>,
  /// what the host sent about notes, each with the sample it is played at, which is as far
  /// behind where it came as the output is behind the input
  waiting: VecDeque<(u64, PluginNoteEvent<Self>)>,
  /// samples taken since the plugin was activated or reset
  clock: u64,
}

impl Default for CanWeTalk {
  fn default() -> Self {
    // the host's rate arrives in `activate`, which builds the shifter and the analyzers again
    let sample_rate = 48_000.0;
    let params = Arc::<CanWeTalkParams>::default();
    let feed = Feed::new(SLOTS);
    Self {
      shifter: Shifter::new(sample_rate, params.detect.value()),
      heard: Analyzer::new(sample_rate, INPUT_BANDS, INPUT_LEVEL, &feed),
      made: Analyzer::new(sample_rate, OUTPUT_BANDS, OUTPUT_LEVEL, &feed),
      feed,
      params,
      sample_rate,
      settings: Vec::with_capacity(BLOCK),
      waiting: VecDeque::with_capacity(WAITING_EVENTS),
      clock: 0,
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
    context: &mut impl ActivateContext<Self>,
  ) -> bool {
    self.sample_rate = buffer_config.sample_rate;
    self.shifter = Shifter::new(self.sample_rate, self.params.detect.value());
    self.shifter.set_live(self.params.live.value());
    context.set_latency_samples(self.shifter.latency() as u32);
    self.heard = Analyzer::new(self.sample_rate, INPUT_BANDS, INPUT_LEVEL, &self.feed);
    self.made = Analyzer::new(self.sample_rate, OUTPUT_BANDS, OUTPUT_LEVEL, &self.feed);
    self.waiting.clear();
    self.clock = 0;
    true
  }

  // the shifter lets go of every note, and so the notes still waiting go too
  fn reset(&mut self) {
    self.shifter.reset();
    self.waiting.clear();
    self.clock = 0;
    self.rest_feed();
  }

  // the screen shows nothing heard or played while the host does not process
  fn stop_processing(&mut self) {
    self.rest_feed();
  }

  fn deactivate(&mut self) {
    self.rest_feed();
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

    let was = self.shifter.latency();
    self.shifter.set_live(self.params.live.value());
    let latency = self.shifter.latency();
    // the wrapper tells the host only when this is not what it told it last
    context.set_latency_samples(latency as u32);
    if latency < was {
      // the output jumps ahead, and what waited to be played with the input it skips over is
      // played now, before anything that comes in this buffer
      while let Some((_, event)) = self.waiting.pop_front() {
        self.handle(event);
      }
    }
    while let Some(event) = context.next_event() {
      if self.waiting.len() == WAITING_EVENTS {
        // played early rather than lost, and in the order it came
        let (_, early) = self
          .waiting
          .pop_front()
          .expect("the waiting events are full");
        self.handle(early);
      }
      let at = self.clock + u64::from(event.timing()) + latency as u64;
      self.waiting.push_back((at, event));
    }

    let samples = buffer.samples();
    let channels = buffer.as_slice();
    let mut start = 0;
    while start < samples {
      let now = self.clock + start as u64;
      let first = self.params.settings(&block, start);
      // before the sample's notes, which are taken up in the mode they come in. the mode and
      // latch are not smoothed, so they stay as they are to the end of the buffer
      self.shifter.notes.set_play(&first.play);
      while let Some(&(_, event)) = self.waiting.front().filter(|(at, _)| *at <= now) {
        self.waiting.pop_front();
        self.handle(event);
      }
      // a block goes up to the next sample that has something played at it
      let played = self
        .waiting
        .front()
        .map_or(samples, |&(at, _)| (at - self.clock) as usize);
      let count = BLOCK.min(samples.min(played) - start);
      self.settings.clear();
      self.settings.push(first);
      for index in start + 1..start + count {
        self.settings.push(self.params.settings(&block, index));
      }

      let mut wet = [[0.0; 2]; BLOCK];
      for (index, input) in wet[..count].iter_mut().enumerate() {
        for (input, channel) in input.iter_mut().zip(channels.iter()) {
          let sample = channel[start + index];
          // false for a sample that is no number too, which is then silence
          if sample.abs() <= INPUT_LIMIT {
            *input = sample;
          }
        }
        if mono {
          input[1] = input[0];
        }
      }
      // the input as the effect heard it, which the output is mixed and shown against
      let mut dry = [[0.0; 2]; BLOCK];
      (self.shifter).process(&mut wet[..count], &mut dry[..count], &self.settings);
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
      // the detected pitch is the detector's, which hears the input the lookahead before the
      // spectra do. the screen draws a frame in about that long, so it is shown as it is
      let (pitch, followed) = (self.shifter.detected_hz(), self.shifter.formant_followed());
      write_played(&self.feed, pitch, followed, &self.shifter.made_hz());
      start += count;
    }
    self.clock += samples as u64;

    ProcessStatus::Tail(self.shifter.tail() as u32)
  }
}

/// a host that names no channel for a note is not sending MPE, so the note goes on the first
/// channel with every other. one it names no key for is not a note to play
fn note(channel: Channel, key: Key) -> Option<Note> {
  Note::new(channel.number().unwrap_or(0), key.number()?)
}

impl CanWeTalk {
  /// forgets what the editor's spectra have heard and writes to the feed that nothing is heard
  /// or played. fills in place and stores, so the audio thread can call it
  fn rest_feed(&mut self) {
    self.heard.reset(&self.feed);
    self.made.reset(&self.feed);
    write_played(&self.feed, None, None, &[0.0; VOICES]);
  }

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
  use std::cell::Cell;
  use std::collections::VecDeque;

  use nice_plug::context::process::SendEventError;
  use nice_plug::midi::VoiceID;
  use nice_plug::params::internals::ParamPtr;

  use super::*;
  use crate::signals::vowel_sampled_at;

  const RATE: f32 = 48_000.0;

  /// stands in for the host over one buffer
  struct Host<'a> {
    transport: Transport,
    events: VecDeque<PluginNoteEvent<CanWeTalk>>,
    /// the latency the plugin last told the host it has
    latency: &'a Cell<Option<u32>>,
  }

  impl ProcessContext<CanWeTalk> for Host<'_> {
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

    fn set_latency_samples(&self, samples: u32) {
      self.latency.set(Some(samples));
    }

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
    let (plugin, output, _) = run_telling_latency(take, sizes);
    (plugin, output)
  }

  /// `run`, and the latency the plugin last told the host it has
  fn run_telling_latency(take: &Take, sizes: &[usize]) -> (CanWeTalk, Vec<[f32; 2]>, Option<u32>) {
    let latency = Cell::new(None);
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
    plugin.heard = Analyzer::new(rate, INPUT_BANDS, INPUT_LEVEL, &plugin.feed);
    plugin.made = Analyzer::new(rate, OUTPUT_BANDS, OUTPUT_LEVEL, &plugin.feed);
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
        latency: &latency,
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
    (plugin, output, latency.get())
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
        "chord live",
        Take {
          played: chord(),
          set: vec![(0, "live", 1.0)],
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
  const REFERENCE: [(&str, u64); 36] = [
    ("follow", 0x68fc3332d2da7c64),
    ("follow shifted", 0xce2ddd1f5cf98a8d),
    ("follow mono", 0x3b699648e864589d),
    ("follow unanchored", 0x575995c407375efa),
    ("pitch", 0x65706a7c1281408e),
    ("formant", 0x7c8a7d0369dbb8f7),
    ("chord", 0xa75138986bdae7a7),
    ("chord unanchored", 0x8c2feeeec36307f6),
    ("more notes than voices", 0x23467457bb3f904d),
    ("envelope", 0xd67264cb0445c28e),
    ("envelope with no times", 0xff66977d85997431),
    ("latch", 0xa5ecb6720bcbcba7),
    ("glide", 0x819fa3d4b911cba3),
    ("glide between notes only", 0x17075f245066f9a9),
    ("expression", 0x04a0e2a5baeeb192),
    ("vibrato", 0x85a51241d3473c2f),
    ("vibrato by the tempo", 0xddb23573b2a3c3aa),
    ("vibrato by the tempo of a stopped host", 0x44f3fc525e27359f),
    ("effects", 0x4a45ea57da188804),
    ("effects mono", 0x9411beb6cee4c5cd),
    ("chord live", 0x4972a1fa78fa2e17),
    ("mix", 0xd042e4f805872d45),
    ("bass", 0x8f1527fcfed414c1),
    ("bass at 192 kHz", 0x0944fdd7be8f3a15),
    ("high", 0xf41a08746a3a2c71),
    ("detection range changes", 0x30b2bcd24cacccf1),
    ("mode off", 0x0e8e536daa2e899c),
    ("mode mono", 0xd2590b80107561e1),
    ("mode poly", 0x2f768d73356a1cb1),
    ("mode mono gate", 0x20a3b7c901b47240),
    ("mode poly gate", 0x003533dcf08612cf),
    ("chord at 44.1 kHz", 0xd4b7fd9478273cba),
    ("chord at 96 kHz", 0xa924b6fd8ba39e06),
    ("chord at 192 kHz", 0x38072506d0a7d071),
    ("four seconds of everything", 0xe57dbb58614c81c0),
    ("unanchored at 96 kHz", 0x965bcf4e615bd2ea),
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
  /// `just picture` writes it to a png file, and its first sample goes to the file
  /// `CANWETALK_FIRST_PICTURE` names, when it names one
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
    // the picture refined until settled, and its first sample before that
    let written = [
      ("CANWETALK_PICTURE", &picture.pixels),
      ("CANWETALK_FIRST_PICTURE", &picture.first),
    ];
    for (variable, pixels) in written {
      let Some(path) = std::env::var_os(variable) else {
        continue;
      };
      let file = std::fs::File::create(path).expect("the png file can be made");
      let [width, height] = picture.size;
      let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
      encoder.set_color(png::ColorType::Rgba);
      encoder.set_depth(png::BitDepth::Eight);
      let mut writer = encoder.write_header().expect("the png header is written");
      (writer.write_image_data(pixels)).expect("the png pixels are written");
    }
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

  /// in a gate mode a note plays on through its release, and is published until the release
  /// has run out
  #[test]
  fn a_released_note_is_published_until_it_falls_silent() {
    let take = |seconds: f32| Take {
      played: vec![on(50, 0, 57), off(200, 0, 57)],
      set: vec![(0, "midi", 3.0), (0, "release", 500.0)],
      ..Take::new(RATE, tone((RATE * seconds) as usize))
    };
    let (releasing, _) = run(&take(0.4), &[512]);
    assert!(
      releasing.shifter.made_hz()[0] > 0.0,
      "silent in the release"
    );
    let (released, _) = run(&take(0.9), &[512]);
    assert_eq!(released.shifter.made_hz(), [0.0; VOICES]);
  }

  /// the voice that follows the input is published for a while after the input's pitch is
  /// lost, and not long after
  #[test]
  fn the_following_voice_outlasts_its_pitch_for_a_moment() {
    let take = |silent: f32| {
      let mut input = tone(RATE as usize / 2);
      input.resize(input.len() + (RATE * silent) as usize, [0.0; 2]);
      Take {
        set: vec![(0, "pitch", 7.0)],
        ..Take::new(RATE, input)
      }
    };
    let (soon, _) = run(&take(0.1), &[512]);
    assert_eq!(soon.shifter.detected_hz(), None);
    assert!(soon.shifter.made_hz()[0] > 0.0, "gone straight away");
    let (later, _) = run(&take(0.3), &[512]);
    assert_eq!(later.shifter.made_hz(), [0.0; VOICES]);
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

  /// the input passes through the effect unchanged with no shift, so with half of it mixed
  /// in the output is the input as it was, as far behind as the plugin told the host. a dry
  /// signal that was not as far behind would be heard twice, a comb
  #[test]
  fn the_dry_signal_lines_up_with_the_effect() {
    for (live, behind) in [(0.0, (0.02 * RATE) as usize), (1.0, 0)] {
      let take = Take {
        set: vec![(0, "mix", 0.5), (0, "live", live)],
        ..Take::new(RATE, tone(RATE as usize / 2))
      };
      let (_, output, told) = run_telling_latency(&take, &[512]);
      assert_eq!(told, Some(behind as u32), "live at {live}");
      // once the voice has faded in for the plain input
      let settled = RATE as usize / 100;
      let furthest = (output[settled + behind..].iter().flatten())
        .zip(take.input[settled..].iter().flatten())
        .fold(0f32, |furthest, (out, input)| {
          furthest.max((out - input).abs())
        });
      assert!(furthest < 1e-3, "{furthest} off, live at {live}");
    }
  }

  /// in a gate mode nothing sounds until a note is played, and the note starts with the input
  /// that came with it, as far behind it as the plugin told the host the output is
  #[test]
  fn a_note_starts_with_the_input_it_came_with() {
    for live in [0.0, 1.0] {
      let take = Take {
        played: vec![on(200, 0, 57)],
        set: vec![(0, "midi", 3.0), (0, "attack", 0.0), (0, "live", live)],
        ..Take::new(RATE, tone(RATE as usize / 2))
      };
      let (_, output, told) = run_telling_latency(&take, &[512]);
      let starts = take.sample(200) + told.expect("the latency is told") as usize;
      let loudest = |frames: &[[f32; 2]]| frames.iter().flatten().fold(0f32, |a, x| a.max(x.abs()));
      // the take's first sample is played in the mode a new plugin has, and the voice that
      // follows the input there is released over the first 100 ms
      let early = loudest(&output[take.sample(100)..starts]);
      assert_eq!(early, 0.0, "sounds early, live at {live}");
      let first = loudest(&output[starts..starts + RATE as usize / 50]);
      assert!(
        first > 0.1,
        "{first} in the note's first 20 ms, live at {live}"
      );
    }
  }
}
