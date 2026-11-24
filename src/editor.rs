//! the plugin's window: a lit panel drawn by skeuo on a baseview window, with the knobs
//! turned and their values typed by what the pointer and the keys do on it

mod interaction;
mod panel;
mod screen;
mod sent;
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

use self::interaction::{Dial, Key, SCALES};
use self::panel::{HEIGHT, WIDTH};
use self::window::Window;
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
    }
  }
}

/// how the window may be sized: at the panel's own shape, from the smallest to the largest
/// it is drawn at
const RESIZE: ResizeHint = ResizeHint {
  preserve_aspect_ratio: true,
  aspect_ratio_width: WIDTH as u32,
  aspect_ratio_height: HEIGHT as u32,
  ..ResizeHint::RESIZABLE.with_min_max_logical_size(Some(sized(SCALES[0])), Some(sized(SCALES[1])))
};

/// the size of the panel's shape nearest one a host asks about, all in the same units: as
/// wide as asked if the width is what has changed more from the size the window has, and
/// else as tall as asked, and neither smaller than the least nor larger than the most.
///
/// nice-plug's own [`ResizeHint::adjust_size`] always takes the height and gives the width
/// from it, under which a window pulled by its side alone does not move
fn shaped(
  asked: NativeSize<u32>,
  current: NativeSize<u32>,
  [least, most]: [NativeSize<u32>; 2],
) -> NativeSize<u32> {
  let (across, down) = (WIDTH as u64, HEIGHT as u64);
  // each change as a part of the panel's own side, cross multiplied
  let wider = u64::from(asked.width.abs_diff(current.width)) * down;
  let taller = u64::from(asked.height.abs_diff(current.height)) * across;
  let width = if wider >= taller {
    asked.width
  } else {
    (u64::from(asked.height) * across / down) as u32
  };
  let width = width.clamp(least.width, most.width);
  NativeSize::new(width, (u64::from(width) * down / across) as u32)
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
  /// the display's scale factor as the bits of an f64, for the size the host is told
  display_scale: AtomicU64,
  /// whether a value is being typed, which is when keys the host offers are taken
  typing: AtomicBool,
}

impl Editor {
  pub(crate) fn new(params: Arc<CanWeTalkParams>) -> Self {
    Self {
      params,
      shared: Arc::new(Shared {
        // until a window has opened and found the display's own
        display_scale: AtomicU64::new(1.0_f64.to_bits()),
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
    let settings = WindowSettings::new()
      .with_title("CanWeTalk")
      .with_size(sized(self.params.editor_scale.load(Ordering::Relaxed)))
      .with_min_size::<LogicalSize<f32>>(Some(sized(SCALES[0])))
      .with_max_size::<LogicalSize<f32>>(Some(sized(SCALES[1])))
      .with_parent(parent.as_ref())
      .with_wait_for_parent(wait_for_parent)
      .with_fallback_scale_factor(fallback_scale_factor);
    let (to_window, from_host) = mpsc::channel();
    // with no window to go in it is the window manager's to size
    let top_level = parent.is_none() && !wait_for_parent;
    let (params, shared) = (Arc::clone(&self.params), Arc::clone(&self.shared));
    let window = baseview::Window::create_with_host(
      settings,
      move |context| -> Result<Window, HandlerError> {
        Window::new(context, gui_context, params, shared, from_host, top_level)
      },
      host,
    )?;
    Ok(SpawnedEditor {
      handle: Handle {
        waker: window.waker(),
        shared: Arc::clone(&self.shared),
        to_window,
      },
      window,
    })
  }

  fn size(&self) -> NativeSize<u32> {
    let scale = f64::from_bits(self.shared.display_scale.load(Ordering::Relaxed));
    let size = sized(self.params.editor_scale.load(Ordering::Relaxed));
    NativeSize::from_size(size.into(), scale)
  }

  fn resize_hint(&self) -> ResizeHint {
    RESIZE
  }
}

/// the host's side of an open window
pub struct Handle {
  waker: WindowWaker,
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
    window.resize(new_size)
  }

  fn adjust_size(
    &self,
    new_size: NativeSize<u32>,
    window: &baseview::Window,
  ) -> Option<NativeSize<u32>> {
    let now = window.size();
    let current = NativeSize::from_logical_or_physical(now.logical, now.physical);
    let limit = |scale: f32| NativeSize::from_size(sized(scale).into(), now.scale_factor);
    Some(shaped(
      new_size,
      current,
      [limit(SCALES[0]), limit(SCALES[1])],
    ))
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

#[cfg(test)]
mod tests {
  use super::*;

  /// the sizes a host may ask about at one and a half pixels to a point, from the window as
  /// it opens
  fn asked(width: u32, height: u32) -> (u32, u32) {
    let limits = [NativeSize::new(563, 281), NativeSize::new(2250, 1125)];
    let shaped = shaped(
      NativeSize::new(width, height),
      NativeSize::new(750, 375),
      limits,
    );
    (shaped.width, shaped.height)
  }

  #[test]
  fn a_size_a_host_asks_about_is_answered_with_the_panels_shape() {
    // the shape itself is left alone
    assert_eq!(asked(750, 375), (750, 375));
    assert_eq!(asked(1000, 500), (1000, 500));
    // pulled by its side the window follows the width, and by its foot the height
    assert_eq!(asked(900, 375), (900, 450));
    assert_eq!(asked(600, 375), (600, 300));
    assert_eq!(asked(750, 450), (900, 450));
    assert_eq!(asked(750, 300), (600, 300));
    // by its corner, whichever side has gone further for its length
    assert_eq!(asked(900, 500), (1000, 500));
    assert_eq!(asked(1000, 400), (1000, 500));
    // and no smaller or larger than the window is let be
    assert_eq!(asked(100, 375), (563, 281));
    assert_eq!(asked(750, 50), (563, 281));
    assert_eq!(asked(5000, 375), (2250, 1125));
    assert_eq!(asked(750, 3000), (2250, 1125));
  }
}
