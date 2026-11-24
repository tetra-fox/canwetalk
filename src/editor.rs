//! the plugin's window: a lit panel drawn by skeuo on a baseview window, with the knobs
//! turned and their values typed by what the pointer and the keys do on it

mod panel;

use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use baseview::HandlerError;
use baseview::dpi::LogicalSize;
use controls::fit::{chosen, shaped};
use controls::furniture::{GripLook, MenuLook};
use controls::interaction::{Dial, Feel, Key, Readout};
use controls_window::{Opening, Remembered, Shared, Tell};
use nice_plug::context::gui::GuiContext;
use nice_plug::editor::dpi::NativeSize;
use nice_plug::editor::{EditorHandle, HostMethods, SpawnedEditor};
use nice_plug::prelude::*;

use self::panel::{HEIGHT, Panel, READOUT_LETTERS, WIDTH, readout_shows};
use crate::params::CanWeTalkParams;

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

  /// the control's parameter as the window has it: what the control does, and the
  /// parameter's ways to the host
  fn bound(self, params: &Arc<CanWeTalkParams>, gui: &Arc<GuiContext>) -> controls_window::Param {
    let (value, begin, set, end) = (
      (Arc::clone(params), Arc::clone(gui)),
      (Arc::clone(params), Arc::clone(gui)),
      (Arc::clone(params), Arc::clone(gui)),
      (Arc::clone(params), Arc::clone(gui)),
    );
    controls_window::Param {
      dial: self.dial(params),
      value: Box::new(move || self.param(&value.0).unmodulated_normalized_value()),
      begin: Box::new(move || (begin.1.param_setter()).begin_set_parameter(self.param(&begin.0))),
      set: Box::new(move |to| {
        (set.1.param_setter()).set_parameter_normalized(self.param(&set.0), to);
      }),
      end: Box::new(move || (end.1.param_setter()).end_set_parameter(self.param(&end.0))),
    }
  }

  /// the control as the panel's behaviour knows it: its step, and its parameter's ways of
  /// reading and writing a value
  fn dial(self, params: &Arc<CanWeTalkParams>) -> Dial {
    // each of the dial's ways keeps the parameters alive for itself
    let with = |params: &Arc<CanWeTalkParams>| Arc::clone(params);
    let (plain, turn, write, read) = (with(params), with(params), with(params), with(params));
    Dial {
      default: self.param(params).default_normalized_value(),
      step: self.step(),
      plain: Box::new(move |value| self.param(&plain).preview_plain(value)),
      turn: Box::new(move |plain| self.param(&turn).preview_normalized(plain)),
      write: Box::new(move |value, unit| {
        self.param(&write).normalized_value_to_string(value, unit)
      }),
      read: Box::new(move |text| self.param(&read).string_to_normalized_value(text)),
      feel: Feel::default(),
    }
  }
}

/// the window is no smaller and no larger than this many times its own size
const SCALES: [f32; 2] = [0.75, 3.0];

/// how the window may be sized: at the panel's own shape, from the smallest to the largest
/// it is drawn at
const RESIZE: ResizeHint = ResizeHint {
  preserve_aspect_ratio: true,
  aspect_ratio_width: WIDTH as u32,
  aspect_ratio_height: HEIGHT as u32,
  ..ResizeHint::RESIZABLE.with_min_max_logical_size(Some(sized(SCALES[0])), Some(sized(SCALES[1])))
};

/// how many of the host's units go to a point. sizes pass to and from the host in pixels,
/// apart from on macos, where they are in points
fn per_point(display_scale: f64) -> f64 {
  if cfg!(target_os = "macos") {
    1.0
  } else {
    display_scale
  }
}

/// the size the host is told the editor has, in the host's units
fn reported(remembered: &Remembered, shared: &Shared) -> NativeSize<u32> {
  NativeSize::from_size(sized((remembered.get)()).into(), shared.display_scale())
}

/// the window's size in the screen's own points at so many times the panel's
const fn sized(scale: f32) -> LogicalSize<f32> {
  LogicalSize::new(WIDTH * scale, HEIGHT * scale)
}

pub struct Editor {
  params: Arc<CanWeTalkParams>,
  /// the scale the user chose for the window, which is kept with the plugin's state
  remembered: Arc<Remembered>,
  shared: Arc<Shared>,
}

impl Editor {
  pub(crate) fn new(params: Arc<CanWeTalkParams>) -> Self {
    let (get, set) = (Arc::clone(&params), Arc::clone(&params));
    Self {
      params,
      remembered: Arc::new(Remembered {
        get: Box::new(move || get.editor_scale.load(Ordering::Relaxed)),
        set: Box::new(move |scale| set.editor_scale.store(scale, Ordering::Relaxed)),
      }),
      shared: Arc::default(),
    }
  }
}

impl nice_plug::editor::Editor for Editor {
  type Handle = Handle;

  fn spawn(
    &self,
    parent: Option<ParentWindowHandle>,
    wait_for_parent: bool,
    fallback_scale_factor: Option<f64>,
    gui_context: GuiContext,
    host: Option<HostMethods>,
  ) -> Result<SpawnedEditor<Handle>, Box<dyn Error>> {
    let host = host.map(|host| {
      baseview::host::Host::new()
        .with_callbacks(HostCallbacks(host.callbacks))
        .with_main_thread(MainThreadCaller(host.main_thread_caller))
    });
    let panel = controls_window::Panel {
      title: "CanWeTalk".to_owned(),
      size: [WIDTH, HEIGHT],
      scales: SCALES,
      readout: Readout {
        room: READOUT_LETTERS,
        shows: readout_shows,
      },
      menu: MenuLook::default(),
      grip: GripLook::default(),
    };
    let gui = Arc::new(gui_context);
    let bound = Control::ALL.map(|control| control.bound(&self.params, &gui));
    let opening = Opening {
      parent: parent.as_ref(),
      wait_for_parent,
      host_scale: fallback_scale_factor,
      host,
    };
    let params = Arc::clone(&self.params);
    let opened = controls_window::open(
      panel,
      bound.into(),
      Arc::clone(&self.remembered),
      Arc::clone(&self.shared),
      opening,
      move |gpu| Panel::new(gpu, params),
    )?;
    Ok(SpawnedEditor {
      handle: Handle {
        tell: opened.tell,
        remembered: Arc::clone(&self.remembered),
        shared: Arc::clone(&self.shared),
      },
      window: opened.window,
    })
  }

  fn size(&self) -> NativeSize<u32> {
    reported(&self.remembered, &self.shared)
  }

  fn resize_hint(&self) -> ResizeHint {
    RESIZE
  }
}

/// the host's side of an open window
pub struct Handle {
  tell: Tell,
  remembered: Arc<Remembered>,
  shared: Arc<Shared>,
}

impl EditorHandle for Handle {
  type Window = baseview::Window;
  type Error = baseview::Error;

  fn run_until_closed(window: baseview::Window) -> Result<(), baseview::Error> {
    window.run_until_closed()
  }

  fn set_parent(
    &self,
    parent: ParentWindowHandle,
    window: &baseview::Window,
  ) -> Result<(), baseview::Error> {
    let parented = window.set_parent(&parent);
    self.tell.shown();
    parented
  }

  fn show(&self, window: &baseview::Window) -> Result<(), baseview::Error> {
    let shown = window.show();
    self.tell.shown();
    shown
  }

  fn hide(&self, window: &baseview::Window) -> Result<(), baseview::Error> {
    window.hide()
  }

  fn set_size(
    &self,
    new_size: NativeSize<u32>,
    window: &baseview::Window,
  ) -> Result<(), baseview::Error> {
    // a size the host comes up with is the user's doing, at the host's own frame, and is
    // what the editor opens at from then on. the window's settling as it opens is not
    let told = reported(&self.remembered, &self.shared);
    let (given, told) = ([new_size.width, new_size.height], [told.width, told.height]);
    let per_point = per_point(window.size().scale_factor);
    if let Some(scale) = chosen(given, told, per_point, [WIDTH, HEIGHT], SCALES) {
      (self.remembered.set)(scale);
    }
    window.resize(new_size)
  }

  fn adjust_size(
    &self,
    new_size: NativeSize<u32>,
    window: &baseview::Window,
  ) -> Option<NativeSize<u32>> {
    let now = window.size();
    let current = NativeSize::from_logical_or_physical(now.logical, now.physical);
    let limit = |scale: f32| {
      let limit = NativeSize::<u32>::from_size(sized(scale).into(), now.scale_factor);
      [limit.width, limit.height]
    };
    let [width, height] = shaped(
      [new_size.width, new_size.height],
      [current.width, current.height],
      [limit(SCALES[0]), limit(SCALES[1])],
      [WIDTH, HEIGHT],
    );
    Some(NativeSize::new(width, height))
  }

  fn host_main_thread_callback(&self, window: &baseview::Window) {
    window.host_main_thread_callback();
  }

  fn set_fallback_scale_factor(
    &self,
    scale_factor: f64,
    window: &baseview::Window,
  ) -> Result<(), baseview::Error> {
    window.suggest_fallback_scale_factor(scale_factor)
  }

  fn on_virtual_key_from_host(
    &self,
    key_code: VirtualKeyCode,
    is_down: bool,
    modifiers: Modifiers,
  ) -> bool {
    if !self.shared.typing() {
      return false;
    }
    let extend = modifiers.contains(Modifiers::SHIFT);
    let key = match key_code {
      VirtualKeyCode::Return | VirtualKeyCode::NumpadEnter => Some(Key::Enter),
      VirtualKeyCode::Escape => Some(Key::Escape),
      VirtualKeyCode::Backspace => Some(Key::Backspace),
      VirtualKeyCode::Delete => Some(Key::Delete),
      VirtualKeyCode::ArrowLeft => Some(Key::Left { extend }),
      VirtualKeyCode::ArrowRight => Some(Key::Right { extend }),
      VirtualKeyCode::Home => Some(Key::Home { extend }),
      VirtualKeyCode::End => Some(Key::End { extend }),
      VirtualKeyCode::Tab => Some(Key::Tab),
      VirtualKeyCode::Space => Some(Key::Letter(' ')),
      _ => None,
    };
    if is_down && let Some(key) = key {
      self.tell.key(key);
    }
    // every key is kept from the host while a value is typed, and so is its release
    true
  }

  fn state_changed(&self) {
    self.tell.state_changed();
  }

  fn param_value_changed(&self, _id: &str, _normalized_value: f32) {
    self.tell.param_changed();
  }

  fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {
    // the panel shows the values without their modulation
  }
}

/// baseview's host callbacks from the ones nice-plug hands an editor
struct HostCallbacks(Box<dyn nice_plug::editor::HostCallbacks>);

impl baseview::host::HostCallbacks for HostCallbacks {
  fn request_resize(&mut self, new_size: baseview::WindowSize) -> Result<(), HandlerError> {
    (self.0)
      .request_resize(new_size.physical.into(), new_size.scale_factor)
      .map_err(HandlerError::from_boxed)
  }

  fn destroyed(&mut self) {
    self.0.destroyed();
  }
}

struct MainThreadCaller(Box<dyn nice_plug::editor::HostMainThreadCaller>);

impl baseview::host::HostMainThreadCaller for MainThreadCaller {
  fn call_main_thread(&mut self) {
    self.0.call_main_thread();
  }
}
