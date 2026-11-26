//! the plugin's window: a lit panel drawn by potpot, with a control for every parameter and a
//! screen that shows the input's spectrum and the note it is at

mod note;
mod panel;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use nice_plug::prelude::*;
use potpot_controls::interaction::{Push, Readout, Touch};
use potpot_window::nice_plug::{Bound, Misbound};
use potpot_window::{Feed, Remembered, Settings};

use self::panel::{HEIGHT, READOUT_LETTERS, WIDTH};
use crate::notes::Mode;
use crate::params::{CanWeTalkParams, Division};
use crate::pitch::Range;

/// the plugin's panel as the wrapper has it
pub type Editor = potpot_window::nice_plug::Editor<panel::Panel>;

/// the controls on the panel, by their groups in the order the host lists them
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
  Pitch,
  Formant,
  FormantFollow,
  Smooth,
  Detune,
  Smear,
  Stereo,
  VibratoDepth,
  VibratoRate,
  VibratoSync,
  VibratoDivision,
  Mode,
  Latch,
  Glide,
  GlideNotesOnly,
  Attack,
  Decay,
  Sustain,
  Release,
  PressureLevel,
  SlideFormant,
  BendRange,
  Detect,
  Anchor,
  Mix,
}

/// the kind of part a control is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
  /// a large knob with figures around it
  Shift,
  Knob,
  /// a pointed knob that stops at each of a few choices
  Selector {
    choices: usize,
  },
  /// a push switch that stays in while it is on
  Switch,
}

impl Control {
  const ALL: [Self; 25] = [
    Self::Pitch,
    Self::Formant,
    Self::FormantFollow,
    Self::Smooth,
    Self::Detune,
    Self::Smear,
    Self::Stereo,
    Self::VibratoDepth,
    Self::VibratoRate,
    Self::VibratoSync,
    Self::VibratoDivision,
    Self::Mode,
    Self::Latch,
    Self::Glide,
    Self::GlideNotesOnly,
    Self::Attack,
    Self::Decay,
    Self::Sustain,
    Self::Release,
    Self::PressureLevel,
    Self::SlideFormant,
    Self::BendRange,
    Self::Detect,
    Self::Anchor,
    Self::Mix,
  ];

  fn kind(self) -> Kind {
    match self {
      Self::Pitch | Self::Formant => Kind::Shift,
      Self::VibratoSync | Self::Latch | Self::GlideNotesOnly | Self::Anchor => Kind::Switch,
      Self::VibratoDivision => Kind::Selector {
        choices: Division::variants().len(),
      },
      Self::Mode => Kind::Selector {
        choices: Mode::variants().len(),
      },
      Self::Detect => Kind::Selector {
        choices: Range::variants().len(),
      },
      Self::FormantFollow
      | Self::Smooth
      | Self::Detune
      | Self::Smear
      | Self::Stereo
      | Self::VibratoDepth
      | Self::VibratoRate
      | Self::Glide
      | Self::Attack
      | Self::Decay
      | Self::Sustain
      | Self::Release
      | Self::PressureLevel
      | Self::SlideFormant
      | Self::BendRange
      | Self::Mix => Kind::Knob,
    }
  }

  /// the control bound to its parameter. every readout is the same window in the panel, with
  /// room for as many letters
  fn bound(self, params: &CanWeTalkParams) -> Bound {
    let readout = Readout::new(READOUT_LETTERS);
    let CanWeTalkParams {
      shift,
      effects,
      vibrato,
      notes,
      envelope,
      expression,
      ..
    } = params;
    let mut bound = match self {
      Self::Pitch => Bound::new(&shift.pitch, readout),
      Self::Formant => Bound::new(&shift.formant, readout),
      Self::FormantFollow => Bound::new(&shift.formant_follow, readout),
      Self::Smooth => Bound::new(&shift.smooth, readout),
      Self::Detune => Bound::new(&effects.detune, readout),
      Self::Smear => Bound::new(&effects.smear, readout),
      Self::Stereo => Bound::new(&effects.stereo, readout),
      Self::VibratoDepth => Bound::new(&vibrato.depth, readout),
      Self::VibratoRate => Bound::new(&vibrato.rate, readout),
      Self::VibratoSync => Bound::new(&vibrato.sync, readout),
      Self::VibratoDivision => Bound::new(&vibrato.division, readout),
      Self::Mode => Bound::new(&notes.mode, readout),
      Self::Latch => Bound::new(&notes.latch, readout),
      Self::Glide => Bound::new(&notes.glide, readout),
      Self::GlideNotesOnly => Bound::new(&notes.glide_notes_only, readout),
      Self::Attack => Bound::new(&envelope.attack, readout),
      Self::Decay => Bound::new(&envelope.decay, readout),
      Self::Sustain => Bound::new(&envelope.sustain, readout),
      Self::Release => Bound::new(&envelope.release, readout),
      Self::PressureLevel => Bound::new(&expression.pressure_level, readout),
      Self::SlideFormant => Bound::new(&expression.slide_formant, readout),
      Self::BendRange => Bound::new(&expression.bend_range, readout),
      Self::Detect => Bound::new(&params.detect, readout),
      Self::Anchor => Bound::new(&params.anchor, readout),
      Self::Mix => Bound::new(&params.mix, readout),
    };
    // the shifts in semitones move a whole one at a time, unless turned finely
    if let Self::Pitch | Self::Formant | Self::SlideFormant = self {
      bound.step = Some(1.0);
    }
    if self.kind() == Kind::Switch {
      bound.touch = Touch::Press(Push::Latching);
    }
    bound
  }
}

/// the editor of a plugin with these parameters, which draws what the audio thread writes to
/// `feed`. the size the user gives its window is kept with the parameters
pub(crate) fn editor(params: Arc<CanWeTalkParams>, feed: Feed) -> Result<Editor, Misbound> {
  let mut settings = Settings::new("CanWeTalk", [WIDTH, HEIGHT]);
  // the window is no smaller and no larger than this many times the panel's size
  settings.scales = [0.6, 2.0];
  settings.feed = feed;
  let bound = Control::ALL.map(|control| control.bound(&params));
  let (get, set) = (Arc::clone(&params), Arc::clone(&params));
  let remembered = Remembered {
    get: Box::new(move || get.editor_scale.load(Ordering::Relaxed)),
    set: Box::new(move |scale| set.editor_scale.store(scale, Ordering::Relaxed)),
  };
  Editor::new(
    params,
    settings,
    bound.into(),
    remembered,
    panel::Panel::new,
  )
}
