//! which notes sound and which voice plays each: a voice for each held note, and outside the
//! gate modes one voice at the input's own pitch while none is held

use nice_plug::prelude::Enum;

use crate::marks::Cycle;
use crate::ring::Ring;
use crate::shifter::Settings;
use crate::voice::{SLIDE_REST, Scratch, Voice};

const VOICES: usize = 16;
/// notes that wait for a voice while more are held than there are voices
const MAX_WAITING: usize = 16;
/// channels a note can be on
const CHANNELS: usize = 16;
/// keys a channel has
const KEYS: u8 = 128;

/// a key on a channel. under MPE each sounding note has a channel to itself, so what is sent
/// on the channel bends that note alone
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Note {
  /// under `CHANNELS`
  channel: u8,
  /// under `KEYS`
  key: u8,
}

impl Note {
  /// none for a channel or key there is no such note on
  pub(crate) fn new(channel: u8, key: u8) -> Option<Self> {
    (usize::from(channel) < CHANNELS && key < KEYS).then_some(Self { channel, key })
  }

  pub(crate) fn key(self) -> u8 {
    self.key
  }
}

/// the keys that are down, a bit for each key of each channel
struct Down([u128; CHANNELS]);

impl Down {
  fn bit(note: Note) -> u128 {
    1 << note.key
  }

  fn press(&mut self, note: Note) {
    self.0[usize::from(note.channel)] |= Self::bit(note);
  }

  fn lift(&mut self, note: Note) {
    self.0[usize::from(note.channel)] &= !Self::bit(note);
  }

  fn holds(&self, note: Note) -> bool {
    self.0[usize::from(note.channel)] & Self::bit(note) != 0
  }

  fn is_empty(&self) -> bool {
    self.0 == [0; CHANNELS]
  }

  fn clear(&mut self) {
    self.0 = [0; CHANNELS];
  }
}

/// how notes are played, which is also a parameter the host sets
#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
  /// notes are ignored and one voice follows the input pitch
  Off,
  /// one voice plays the latest note, or follows the input pitch while none is held
  Mono,
  /// as Mono, but each held note gets a voice, up to `VOICES`
  Poly,
  /// as Mono, but nothing sounds without a note and notes rise and fall by the envelope
  #[name = "Mono Gate"]
  MonoGate,
  /// as Poly, but gated the same way
  #[name = "Poly Gate"]
  PolyGate,
}

impl Mode {
  pub(crate) fn is_gated(self) -> bool {
    matches!(self, Self::MonoGate | Self::PolyGate)
  }
}

/// how notes are taken up and how the pitch moves between them
#[derive(Clone, Copy, Debug)]
pub(crate) struct Play {
  pub(crate) mode: Mode,
  /// notes sound on after their keys come up, until the next key pressed with none down
  pub(crate) latch: bool,
  /// the pitch settles in about the cube of this in seconds
  pub(crate) glide: f64,
  /// glide between notes only, and follow the input pitch as it is
  pub(crate) glide_notes_only: bool,
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
  latch: bool,
  /// the keys that are down, which under latch are not all the notes that sound
  down: Down,
  channels: [Channel; CHANNELS],
  mode: Mode,
  /// counts notes, to tell which voice has had its note longest
  clock: u64,
  /// what the voices work a block out in, one after another
  scratch: Scratch,
}

/// semitones `note` is bent by, the first channel's bend and its own channel's if that is
/// another
fn note_bend(channels: &[Channel; CHANNELS], note: Note) -> f64 {
  let own = &channels[usize::from(note.channel)];
  channels[0].bend + if note.channel == 0 { 0.0 } else { own.bend }
}

/// a value a host sent for a note or channel, if it is a number. one that is not would make
/// the pitch or level it sets no number either, and the output with it
fn sent(value: f64) -> Option<f64> {
  value.is_finite().then_some(value)
}

impl Notes {
  /// voices start out at `input_pitch`
  pub(crate) fn new(sample_rate: f64, input_pitch: f64) -> Self {
    let mut notes = Self {
      voices: std::array::from_fn(|_| Voice::new(sample_rate, input_pitch)),
      follower: None,
      waiting: Vec::with_capacity(MAX_WAITING),
      latch: false,
      down: Down([0; CHANNELS]),
      channels: [Channel::REST; CHANNELS],
      mode: Mode::Off,
      clock: 0,
      scratch: Scratch::new(),
    };
    notes.release_all();
    notes
  }

  /// back to how `new` makes it but for the mode and latch, which are the host's settings.
  /// does not allocate
  pub(crate) fn reset(&mut self, input_pitch: f64) {
    // every field is named so that one added later cannot be left out. `release_all` sees
    // to the ones passed over here
    let Self {
      voices,
      channels,
      clock,
      follower: _,
      waiting: _,
      down: _,
      latch: _,
      mode: _,
      scratch: _,
    } = self;
    for voice in voices {
      voice.reset(input_pitch);
    }
    *channels = [Channel::REST; CHANNELS];
    *clock = 0;
    self.release_all();
  }

  /// lets go of every note. outside the gate modes the first voice is left following the input
  fn release_all(&mut self) {
    self.waiting.clear();
    self.down.clear();
    for voice in &mut self.voices {
      voice.sounding = false;
    }
    self.follower = None;
    if !self.mode.is_gated() {
      self.voices[0].note = None;
      self.voices[0].sounding = true;
      self.voices[0].envelope.trigger();
      self.follower = Some(0);
    }
  }

  /// takes up the mode and latch, which the notes that come after are played by
  pub(crate) fn set_play(&mut self, play: &Play) {
    self.set_mode(play.mode);
    self.set_latch(play.latch);
  }

  /// a change of mode lets go of every note
  fn set_mode(&mut self, mode: Mode) {
    if mode != self.mode {
      self.mode = mode;
      self.release_all();
    }
  }

  fn set_latch(&mut self, latch: bool) {
    if latch == self.latch {
      return;
    }
    self.latch = latch;
    if !self.latch {
      // what only the latch was holding goes, a waiting note before the voices so that
      // none of them takes it up
      let down = &self.down;
      self.waiting.retain(|&note| down.holds(note));
      for index in 0..VOICES {
        if let Some(note) = self.voices[index]
          .held()
          .filter(|&note| !self.down.holds(note))
        {
          self.release(note);
        }
      }
    }
  }

  pub(crate) fn on(&mut self, note: Note) {
    let count = match self.mode {
      Mode::Off => return,
      Mode::Mono | Mode::MonoGate => 1,
      Mode::Poly | Mode::PolyGate => VOICES,
    };
    if self.latch && self.down.is_empty() {
      // the first key after all came up starts over. the notes it replaces fade out where
      // they are, and it takes one of their voices so it can glide from there
      self.waiting.clear();
      for voice in &mut self.voices {
        voice.sounding &= voice.note.is_none();
      }
    }
    // a key pressed again while it waits gets a voice here, and would get another from there
    self.waiting.retain(|&waiting| waiting != note);
    self.down.press(note);
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
    if let Some(displaced) = voice.held().filter(|&held| held != note) {
      if self.waiting.len() == MAX_WAITING {
        self.waiting.remove(0);
      }
      self.waiting.push(displaced);
    }
    let sent = &self.channels[usize::from(note.channel)];
    let bend = note_bend(&self.channels, note);
    voice.start(note, bend, sent.pressure, sent.slide);
    voice.started = self.clock;
  }

  pub(crate) fn off(&mut self, note: Note) {
    self.down.lift(note);
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
      .position(|voice| voice.held() == Some(note))
    else {
      return;
    };
    if let Some(waiting) = self.waiting.pop() {
      let sent = &self.channels[usize::from(waiting.channel)];
      self.voices[index].resume(waiting, sent.pressure, sent.slide);
      return;
    }
    self.voices[index].sounding = false;
    if !self.mode.is_gated() && self.voices.iter().all(|voice| voice.held().is_none()) {
      // the last voice playing goes back to following the input
      self.voices[index].note = None;
      self.voices[index].sounding = true;
      self.follower = Some(index);
    }
  }

  /// bends a channel's notes by `semitones`, or every note if it is the first channel
  pub(crate) fn bend(&mut self, channel: u8, semitones: f64) {
    let Some(semitones) = sent(semitones) else {
      return;
    };
    // a channel past the sixteenth is not one a note can be on
    if let Some(sent) = self.channels.get_mut(usize::from(channel)) {
      sent.bend = semitones;
    }
  }

  /// sets the pressure, 0 to 1, of the notes on a channel and of those to come on it
  pub(crate) fn channel_pressure(&mut self, channel: u8, pressure: f64) {
    let Some(pressure) = sent(pressure) else {
      return;
    };
    if let Some(sent) = self.channels.get_mut(usize::from(channel)) {
      sent.pressure = pressure;
    }
    for voice in self.voices_on(channel) {
      voice.pressure_sent = pressure;
    }
  }

  /// sets the slide, 0 to 1, of the notes on a channel and of those to come on it
  pub(crate) fn channel_slide(&mut self, channel: u8, slide: f64) {
    let Some(slide) = sent(slide) else { return };
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

  /// the voice a held note is on
  fn voice_of(&mut self, note: Note) -> Option<&mut Voice> {
    self
      .voices
      .iter_mut()
      .find(|voice| voice.held() == Some(note))
  }

  /// sets the pressure of one sounding note
  pub(crate) fn note_pressure(&mut self, note: Note, pressure: f64) {
    let Some(pressure) = sent(pressure) else {
      return;
    };
    if let Some(voice) = self.voice_of(note) {
      voice.pressure_sent = pressure;
    }
  }

  /// sets the slide of one sounding note
  pub(crate) fn note_slide(&mut self, note: Note, slide: f64) {
    let Some(slide) = sent(slide) else { return };
    if let Some(voice) = self.voice_of(note) {
      voice.slide_sent = slide;
    }
  }

  /// tunes one sounding note by `semitones`
  pub(crate) fn tune(&mut self, note: Note, semitones: f64) {
    let Some(semitones) = sent(semitones) else {
      return;
    };
    if let Some(voice) = self.voice_of(note) {
      voice.tuning = semitones;
    }
  }

  /// octaves the formants of the voice that started last follow its pitch by, as its last
  /// grain had them, while any voice is held or follows the input
  pub(crate) fn followed(&self) -> Option<f64> {
    (self.voices.iter())
      .filter(|voice| voice.sounding)
      .max_by_key(|voice| voice.started)
      .map(Voice::followed)
  }

  /// adds every voice to the samples of `out`, of which `input` already holds every one's
  /// input. each has its own settings, and in `heard` its input cycle and the ratio the
  /// vibrato moves the pitches by
  pub(crate) fn tick(
    &mut self,
    heard: &[(Cycle, f64)],
    settings: &[Settings],
    input: &[Ring; 2],
    out: &mut [[f32; 2]],
  ) {
    for voice in &mut self.voices {
      let bend = voice
        .note
        .map_or(0.0, |note| note_bend(&self.channels, note));
      voice.tick(heard, bend, settings, input, &mut self.scratch, out);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// more keys can be down than there are voices or places to wait, and latch starts over
  /// only when the last of them is up
  #[test]
  fn latch_counts_every_key_that_is_down() {
    let key = |key| Note::new(0, key).expect("a key under 128");
    let mut notes = Notes::new(48_000.0, 0.01);
    notes.set_mode(Mode::Poly);
    notes.set_latch(true);
    for down in 0..=70 {
      notes.on(key(down));
    }
    for up in 0..70 {
      notes.off(key(up));
    }
    // key 70 is still down, so this one joins the notes that sound
    notes.on(key(100));
    assert!(
      notes.voice_of(key(69)).is_some(),
      "the latched notes were dropped"
    );
  }

  /// a note that gets its voice back from a later one goes on at the pressure and slide last
  /// sent on its own channel, not at those the later note left behind
  #[test]
  fn a_note_that_gets_its_voice_back_has_its_own_pressure_and_slide() {
    let note = |channel, key| Note::new(channel, key).expect("a channel and a key there are");
    let mut notes = Notes::new(48_000.0, 0.01);
    notes.set_mode(Mode::Mono);
    let (first, second) = (note(1, 60), note(2, 64));
    notes.channel_pressure(1, 0.8);
    notes.channel_slide(1, 0.9);
    notes.on(first);
    notes.channel_pressure(2, 0.1);
    notes.channel_slide(2, 0.2);
    notes.on(second);
    notes.off(second);
    let voice = notes.voice_of(first).expect("the first note is still held");
    assert_eq!((voice.pressure_sent, voice.slide_sent), (0.8, 0.9));
  }

  /// a note that lost its voice and waits for one is pressed again without coming up first,
  /// as two overlapping notes of one key are. it is still one note, on one voice
  #[test]
  fn a_key_pressed_again_while_it_waits_has_one_voice() {
    let key = |key| Note::new(0, key).expect("a key under 128");
    let mut notes = Notes::new(48_000.0, 0.01);
    notes.set_mode(Mode::Poly);
    for down in 0..=VOICES as u8 {
      notes.on(key(down));
    }
    notes.on(key(0));
    // two voices come free for the waiting notes
    notes.off(key(2));
    notes.off(key(3));
    let playing = |notes: &Notes| {
      (notes.voices.iter())
        .filter(|voice| voice.held() == Some(key(0)))
        .count()
    };
    assert_eq!(playing(&notes), 1);
    notes.off(key(0));
    assert_eq!(playing(&notes), 0);
  }
}
