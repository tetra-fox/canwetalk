//! which notes sound and which voice plays each: a voice for each held note, and outside the
//! gate modes one voice at the input's own pitch while none is held

use crate::ring::Ring;
use crate::shifter::Settings;
use crate::voice::{SLIDE_REST, Voice};

const VOICES: usize = 16;
/// notes that wait for a voice while more are held than there are voices
const MAX_WAITING: usize = 16;
/// keys tracked as down at once, for latch to know when the last one comes up
const MAX_DOWN: usize = 64;
/// channels a note can be on
const CHANNELS: usize = 16;

/// a key on a channel. under MPE each sounding note has a channel to itself, so what is sent
/// on the channel bends that note alone
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Note {
  pub(crate) channel: u8,
  pub(crate) key: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
  /// notes are ignored and one voice follows the input pitch
  Off,
  /// one voice plays the latest note
  Mono,
  /// each held note gets a voice, up to `VOICES`
  Poly,
}

/// what was last sent on a channel
#[derive(Clone, Copy)]
struct Channel {
  /// semitones. the first channel's bends every note, the others' their own
  bend: f64,
  /// 0 to 1, as is the slide. a note on the channel starts from them
  pressure: f64,
  slide: f64,
}

impl Channel {
  const REST: Self = Self {
    bend: 0.0,
    pressure: 0.0,
    slide: SLIDE_REST,
  };
}

pub(crate) struct Notes {
  voices: [Voice; VOICES],
  /// the voice that follows the input pitch while no note is held
  follower: Option<usize>,
  /// held notes that lost their voice to a later one, the latest last
  waiting: Vec<Note>,
  /// notes sound on after their keys come up, until the next key pressed with none down
  latch: bool,
  /// the keys that are down, which under latch are not all the notes that sound
  down: Vec<Note>,
  channels: [Channel; CHANNELS],
  mode: Mode,
  /// nothing sounds without a note
  gate: bool,
  /// counts notes, to tell which voice has had its note longest
  clock: u64,
}

/// semitones a note on `channel` is bent by. a channel the host should not have sent bends
/// nothing
fn channel_bend(channels: &[Channel; CHANNELS], channel: u8) -> f64 {
  let own = channels.get(usize::from(channel)).filter(|_| channel != 0);
  channels[0].bend + own.map_or(0.0, |own| own.bend)
}

impl Notes {
  /// voices start out at `input_pitch`
  pub(crate) fn new(sample_rate: f64, input_pitch: f64) -> Self {
    let mut notes = Self {
      voices: std::array::from_fn(|_| Voice::new(sample_rate, input_pitch)),
      follower: None,
      waiting: Vec::with_capacity(MAX_WAITING),
      latch: false,
      down: Vec::with_capacity(MAX_DOWN),
      channels: [Channel::REST; CHANNELS],
      mode: Mode::Off,
      gate: false,
      clock: 0,
    };
    notes.release_all();
    notes
  }

  /// does not allocate
  pub(crate) fn reset(&mut self) {
    for voice in &mut self.voices {
      voice.reset();
    }
    self.channels = [Channel::REST; CHANNELS];
    self.release_all();
  }

  /// drops every note. outside the gate modes the first voice is left following the input
  fn release_all(&mut self) {
    self.waiting.clear();
    self.down.clear();
    for voice in &mut self.voices {
      voice.note = None;
      voice.sounding = false;
    }
    self.follower = None;
    if !self.gate {
      self.voices[0].sounding = true;
      self.voices[0].envelope.trigger();
      self.follower = Some(0);
    }
  }

  /// a change of either drops every note
  pub(crate) fn set_mode(&mut self, mode: Mode, gate: bool) {
    if mode != self.mode || gate != self.gate {
      self.mode = mode;
      self.gate = gate;
      self.release_all();
    }
  }

  pub(crate) fn set_latch(&mut self, latch: bool) {
    if latch == self.latch {
      return;
    }
    self.latch = latch;
    if !self.latch {
      // what only the latch was holding goes, a waiting note before the voices so that
      // none of them takes it up
      let down = &self.down;
      self.waiting.retain(|note| down.contains(note));
      for index in 0..VOICES {
        if let Some(note) = self.voices[index]
          .note
          .filter(|note| !self.down.contains(note))
        {
          self.release(note);
        }
      }
    }
  }

  pub(crate) fn on(&mut self, note: Note) {
    let count = match self.mode {
      Mode::Off => return,
      Mode::Mono => 1,
      Mode::Poly => VOICES,
    };
    if self.latch && self.down.is_empty() {
      // the first key after all came up starts over. the notes it replaces fade out where
      // they are, and it takes one of their voices so it can glide from there
      self.waiting.clear();
      for voice in &mut self.voices {
        voice.sounding &= voice.note.is_none();
        voice.note = None;
      }
    }
    if !self.down.contains(&note) && self.down.len() < MAX_DOWN {
      self.down.push(note);
    }
    self.clock += 1;
    let voices = &mut self.voices[..count];
    // the follower first, then a voice with nothing to play, then the one longest on its note
    let index = (self.follower.take())
      .or_else(|| voices.iter().position(|voice| voice.note == Some(note)))
      .or_else(|| voices.iter().position(|voice| !voice.sounding))
      .unwrap_or_else(|| {
        (0..count)
          .min_by_key(|&index| voices[index].started)
          .expect("there is at least one voice")
      });
    let voice = &mut voices[index];
    if let Some(displaced) = voice.note.filter(|&held| held != note) {
      if self.waiting.len() == MAX_WAITING {
        self.waiting.remove(0);
      }
      self.waiting.push(displaced);
    }
    // a channel past the sixteenth is not one anything was sent on
    let sent = self
      .channels
      .get(usize::from(note.channel))
      .unwrap_or(&Channel::REST);
    let bend = channel_bend(&self.channels, note.channel);
    voice.start(note, bend, sent.pressure, sent.slide);
    voice.started = self.clock;
  }

  pub(crate) fn off(&mut self, note: Note) {
    self.down.retain(|&down| down != note);
    if !self.latch {
      self.release(note);
    }
  }

  /// stops a note sounding
  fn release(&mut self, note: Note) {
    self.waiting.retain(|&waiting| waiting != note);
    let Some(index) = self
      .voices
      .iter()
      .position(|voice| voice.note == Some(note))
    else {
      return;
    };
    if let Some(waiting) = self.waiting.pop() {
      self.voices[index].note = Some(waiting);
      self.voices[index].tuning = 0.0;
      return;
    }
    self.voices[index].note = None;
    if !self.gate && self.voices.iter().all(|voice| voice.note.is_none()) {
      // the last voice playing goes back to following the input
      self.follower = Some(index);
    } else {
      self.voices[index].sounding = false;
    }
  }

  /// bends a channel's notes by `semitones`, or every note if it is the first channel
  pub(crate) fn bend(&mut self, channel: u8, semitones: f64) {
    // a channel past the sixteenth is not one a note can be on
    if let Some(sent) = self.channels.get_mut(usize::from(channel)) {
      sent.bend = semitones;
    }
  }

  /// sets the pressure, 0 to 1, of the notes on a channel and of those to come on it
  pub(crate) fn channel_pressure(&mut self, channel: u8, pressure: f64) {
    if let Some(sent) = self.channels.get_mut(usize::from(channel)) {
      sent.pressure = pressure;
    }
    for voice in self.voices_on(channel) {
      voice.pressure_sent = pressure;
    }
  }

  /// sets the slide, 0 to 1, of the notes on a channel and of those to come on it
  pub(crate) fn channel_slide(&mut self, channel: u8, slide: f64) {
    if let Some(sent) = self.channels.get_mut(usize::from(channel)) {
      sent.slide = slide;
    }
    for voice in self.voices_on(channel) {
      voice.slide_sent = slide;
    }
  }

  fn voices_on(&mut self, channel: u8) -> impl Iterator<Item = &mut Voice> {
    self
      .voices
      .iter_mut()
      .filter(move |voice| voice.note.is_some_and(|note| note.channel == channel))
  }

  /// the voice a sounding note is on
  fn voice_of(&mut self, note: Note) -> Option<&mut Voice> {
    self
      .voices
      .iter_mut()
      .find(|voice| voice.note == Some(note))
  }

  /// sets the pressure of one sounding note
  pub(crate) fn note_pressure(&mut self, note: Note, pressure: f64) {
    if let Some(voice) = self.voice_of(note) {
      voice.pressure_sent = pressure;
    }
  }

  /// sets the slide of one sounding note
  pub(crate) fn note_slide(&mut self, note: Note, slide: f64) {
    if let Some(voice) = self.voice_of(note) {
      voice.slide_sent = slide;
    }
  }

  /// tunes one sounding note by `semitones`
  pub(crate) fn tune(&mut self, note: Note, semitones: f64) {
    if let Some(voice) = self.voice_of(note) {
      voice.tuning = semitones;
    }
  }

  /// one sample of every voice, summed. `vibrato` is the ratio it moves their pitches by
  pub(crate) fn tick(
    &mut self,
    input_pitch: f64,
    vibrato: f64,
    settings: &Settings,
    input: &[Ring; 2],
  ) -> [f32; 2] {
    let mut out = [0.0; 2];
    for voice in &mut self.voices {
      let bend = voice
        .note
        .map_or(0.0, |note| channel_bend(&self.channels, note.channel));
      let voiced = voice.tick(input_pitch, bend, vibrato, settings, input);
      for (out, sample) in out.iter_mut().zip(voiced) {
        *out += sample;
      }
    }
    out
  }
}
