//! the plugin's window: a lit panel drawn by skeuo on a baseview window, with the knobs
//! turned and their values typed by what the pointer and the keys do on it

mod panel;
mod screen;
mod window;

use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;

use baseview::dpi::LogicalSize;
use baseview::{HandlerError, WindowSettings, WindowWaker};
use nice_plug::context::gui::GuiContext;
use nice_plug::editor::dpi::NativeSize;
use nice_plug::editor::{EditorHandle, HostMethods, SpawnedEditor};
use nice_plug::prelude::*;

use self::panel::{HEIGHT, WIDTH};
use self::window::{Guarded, Window};
use crate::params::CanWeTalkParams;
use controls::fit::{chosen, shaped};
use controls::interaction::{Dial, Feel, Key};

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

/// how many pixels the display has to a point, as well as that can be told with no window
/// open on it. a host asks the editor's size before it gives it a window to be in, and on
/// windows takes the answer in pixels: of the window the editor is to go in if there is
/// one, and else of the system
#[cfg(target_os = "windows")]
fn display_scale(parent: Option<&ParentWindowHandle>) -> f64 {
  use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};

  // SAFETY: neither call asks anything of what it is given. both answer for a program that
  // is not aware of the display's scale with the 96 it is drawn at
  let dpi = unsafe {
    match parent {
      Some(ParentWindowHandle::Win32Hwnd(window)) => GetDpiForWindow(window.get() as _),
      _ => GetDpiForSystem(),
    }
  };
  f64::from(dpi) / 96.0
}

/// how many pixels the display has to a point, as well as that can be told with no window
/// open on it. a host asks the editor's size before it gives it a window to be in, and on
/// x11 takes the answer in pixels. it is the setting baseview sizes its windows by
#[cfg(target_os = "linux")]
fn display_scale() -> f64 {
  let dpi = x11rb::connect(None).ok().and_then(|(connection, _)| {
    let settings = x11rb::resource_manager::new_from_default(&connection).ok()?;
    settings.get_value::<u32>("Xft.dpi", "").ok().flatten()
  });
  // with no server to ask or nothing set, baseview has one pixel to a point as well
  dpi.map_or(1.0, |dpi| f64::from(dpi) / 96.0)
}

/// the size the host is told the editor has, in the host's units
fn reported(params: &CanWeTalkParams, shared: &Shared) -> NativeSize<u32> {
  let scale = f64::from_bits(shared.display_scale.load(Ordering::Relaxed));
  let size = sized(params.editor_scale.load(Ordering::Relaxed));
  NativeSize::from_size(size.into(), scale)
}

/// the window's size in the screen's own points at so many times the panel's
const fn sized(scale: f32) -> LogicalSize<f32> {
  LogicalSize::new(WIDTH * scale, HEIGHT * scale)
}

pub struct Editor {
  params: Arc<CanWeTalkParams>,
  shared: Arc<Shared>,
}

/// what the host's side of the editor tells the open window, from the host's thread to the
/// window's
enum ToWindow {
  /// a key the host offered while a value is typed
  Key(Key),
  /// the window has been shown, or put in another window
  Shown,
  /// every parameter may have changed
  State,
}

/// what the open window and the host's side of the editor both reach
struct Shared {
  /// the display's scale factor as the bits of an f64, for the size the host is told: the
  /// last window's, and before any window what the system says
  display_scale: AtomicU64,
  /// whether a value is being typed, which is when keys the host offers are taken
  typing: AtomicBool,
}

impl Editor {
  pub(crate) fn new(params: Arc<CanWeTalkParams>) -> Self {
    #[cfg(target_os = "windows")]
    let display_scale = display_scale(None);
    #[cfg(target_os = "linux")]
    let display_scale = display_scale();
    // sizes go to and from the host in points there
    #[cfg(target_os = "macos")]
    let display_scale = 1.0_f64;
    Self {
      params,
      shared: Arc::new(Shared {
        display_scale: AtomicU64::new(display_scale.to_bits()),
        typing: AtomicBool::new(false),
      }),
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
    // baseview 0.4.2 makes a child window at the scale of the window it goes in, but in a
    // host that is aware of each monitor's scale the older way it reports the scale the host
    // gave, 1 if it gave none, and takes the first one the host gives later as a change:
    // the window is made that many times larger again (`WindowSharedState::init` and
    // `scale_factor`, and `WindowHandle::suggest_scale_factor`, in its `platform/win`). so
    // it is given the parent's scale as the host's from the start
    // TODO: pass the host's own on alone once baseview reports the scale it made the window at
    #[cfg(target_os = "windows")]
    let fallback_scale_factor = {
      let scale = display_scale(parent.as_ref());
      (self.shared.display_scale).store(scale.to_bits(), Ordering::Relaxed);
      fallback_scale_factor.or(Some(scale))
    };
    // with no window to go in it is the window manager's to size, which is held to the sizes
    // the panel is drawn at. in a host's window it takes the size the host gives it
    let top_level = parent.is_none() && !wait_for_parent;
    let settings = WindowSettings::new()
      .with_title("CanWeTalk")
      .with_size(sized(self.params.editor_scale.load(Ordering::Relaxed)))
      .with_min_size::<LogicalSize<f32>>(top_level.then(|| sized(SCALES[0])))
      .with_max_size::<LogicalSize<f32>>(top_level.then(|| sized(SCALES[1])))
      .with_parent(parent.as_ref())
      .with_wait_for_parent(wait_for_parent)
      .with_fallback_scale_factor(fallback_scale_factor);
    let (to_window, from_host) = mpsc::channel();
    let (params, shared) = (Arc::clone(&self.params), Arc::clone(&self.shared));
    let window = baseview::Window::create_with_host(
      settings,
      move |context| -> Result<Guarded, HandlerError> {
        Guarded::new(|| Window::new(context, gui_context, params, shared, from_host, top_level))
          // baseview hands on an error of the system's in place of this one
          .inspect_err(|error| nice_error!("the editor could not open: {error}"))
      },
      host,
    )?;
    Ok(SpawnedEditor {
      handle: Handle {
        waker: window.waker(),
        params: Arc::clone(&self.params),
        shared: Arc::clone(&self.shared),
        to_window,
      },
      window,
    })
  }

  fn size(&self) -> NativeSize<u32> {
    reported(&self.params, &self.shared)
  }

  fn resize_hint(&self) -> ResizeHint {
    RESIZE
  }
}

/// the host's side of an open window
pub struct Handle {
  waker: WindowWaker,
  params: Arc<CanWeTalkParams>,
  shared: Arc<Shared>,
  to_window: mpsc::Sender<ToWindow>,
}

impl Handle {
  fn tell(&self, told: ToWindow) {
    // a window that has closed has nothing left to tell
    if self.to_window.send(told).is_ok() {
      self.waker.request_poll();
    }
  }
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
    self.tell(ToWindow::Shown);
    parented
  }

  fn show(&self, window: &baseview::Window) -> Result<(), baseview::Error> {
    let shown = window.show();
    // whatever was drawn before it could be seen is drawn again
    self.tell(ToWindow::Shown);
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
    let told = reported(&self.params, &self.shared);
    let (given, told) = ([new_size.width, new_size.height], [told.width, told.height]);
    let per_point = per_point(window.size().scale_factor);
    if let Some(scale) = chosen(given, told, per_point, [WIDTH, HEIGHT], SCALES) {
      self.params.editor_scale.store(scale, Ordering::Relaxed);
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
    if !self.shared.typing.load(Ordering::Relaxed) {
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
      self.tell(ToWindow::Key(key));
    }
    // every key is kept from the host while a value is typed, and so is its release
    true
  }

  fn state_changed(&self) {
    self.tell(ToWindow::State);
  }

  fn param_value_changed(&self, _id: &str, _normalized_value: f32) {
    // the window looks at the parameters itself, and at once when it is woken. it does so
    // every so often anyway, so a report that never comes costs only that wait
    self.waker.request_poll();
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
