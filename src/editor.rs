//! the plugin's window: a lit panel drawn by potpot, with the knobs turned and their values
//! typed by what the pointer and the keys do on it

mod panel;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use nice_plug::prelude::*;
use potpot_controls::interaction::Readout;
use potpot_window::nice_plug::{Bound, Misbound};
use potpot_window::{Feed, Remembered, Settings};

use self::panel::{HEIGHT, READOUT_LETTERS, WIDTH};
use crate::params::CanWeTalkParams;

/// the plugin's panel as the wrapper has it
pub type Editor = potpot_window::nice_plug::Editor<panel::Panel>;

/// the knobs on the panel, left to right
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
  Pitch,
  Formant,
  Mix,
}

impl Control {
  const ALL: [Self; 3] = [Self::Pitch, Self::Formant, Self::Mix];

  fn param(self, params: &CanWeTalkParams) -> &FloatParam {
    match self {
      Self::Pitch => &params.shift.pitch,
      Self::Formant => &params.shift.formant,
      Self::Mix => &params.mix,
    }
  }

  /// the coarse step the control moves by, in its own units: a semitone of the shifts. the
  /// mix turns smoothly
  fn step(self) -> Option<f32> {
    match self {
      Self::Pitch | Self::Formant => Some(1.0),
      Self::Mix => None,
    }
  }
}

/// the editor of a plugin with these parameters, which draws what the audio thread writes to
/// `feed`. the size the user gives its window is kept with the parameters
pub(crate) fn editor(params: Arc<CanWeTalkParams>, feed: Feed) -> Result<Editor, Misbound> {
  let mut settings = Settings::new("CanWeTalk", [WIDTH, HEIGHT]);
  // the window is no smaller and no larger than this many times the panel's size
  settings.scales = [0.75, 3.0];
  settings.feed = feed;
  let bound = Control::ALL.map(|control| {
    // every readout is the same window in the panel, with room for as many letters
    let mut bound = Bound::new(control.param(&params), Readout::new(READOUT_LETTERS));
    bound.step = control.step();
    bound
  });
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
