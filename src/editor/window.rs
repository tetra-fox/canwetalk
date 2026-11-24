//! the open window: the surface the panel is drawn on, and baseview's events turned into
//! what the panel is told

use std::cell::{Cell, RefCell, RefMut};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use baseview::dpi::LogicalSize;
use baseview::{
  EventStatus, HandlerError, MouseButton, MouseEvent, ScrollDelta, TimerHandle, WindowContext,
  WindowEvent, WindowHandler, WindowSize,
};
use nice_plug::context::gui::GuiContext;
use nice_plug::prelude::*;
use skeuo::glam::Vec2;
use skeuo::{FlatRenderer, Place, Renderer};

use super::interaction::{Action, Button, Edit, Event, Interaction, Key, Menu, Moment, SCALES};
use super::panel::{
  HEIGHT, Panel, Picture, READOUT_LETTERS, WIDTH, fitted, point_at, readout_shows,
};
use super::sent::Sent;
use super::{Control, Shared, ToWindow};
use crate::params::CanWeTalkParams;

/// how often the panel is told that time has passed, and what the window shows is checked
/// against what it should
const TICK: Duration = Duration::from_millis(50);
/// what shows of the window around the panel when the window is not the panel's shape
const AROUND: wgpu::Color = wgpu::Color {
  r: 0.004,
  g: 0.004,
  b: 0.005,
  a: 1.0,
};
/// how many pixels of a wheel that scrolls by them count as one line
const PIXELS_PER_LINE: f32 = 20.0;

pub(super) struct Window {
  context: WindowContext,
  gui: GuiContext,
  params: Arc<CanWeTalkParams>,
  shared: Arc<Shared>,
  /// what the host's side of the editor has to tell the window
  told: mpsc::Receiver<ToWindow>,
  tick: TimerHandle,
  /// whether baseview has asked for a frame yet, before which it is not to be asked for one
  started: Cell<bool>,
  /// the size the window was last given, until the state has taken it in
  sized: Cell<Option<WindowSize>>,
  /// the system's clipboard, from the first use of it on. on x11 what was copied is there
  /// for other programs only for as long as this is kept
  clipboard: RefCell<Option<arboard::Clipboard>>,
  /// baseview calls a handler through a shared reference, always on the window's one
  /// thread. it can call one while another is still running, when something the first one
  /// does has the system deliver more: so the state is only ever taken with
  /// [`Window::state`], which gives nothing rather than panic when it is in use
  state: RefCell<State>,
}

struct State {
  surface: wgpu::Surface<'static>,
  config: wgpu::SurfaceConfiguration,
  device: wgpu::Device,
  queue: wgpu::Queue,
  renderer: Renderer,
  /// draws what is over the picture
  flat: FlatRenderer,
  panel: Panel,
  /// the panel as it was last drawn, which is what the pointer is on
  picture: Picture,
  interaction: Interaction,
  /// what was set from here and has yet to reach the parameters
  sent: Sent,
  /// where the pointer last was, in the panel's points
  pointer: Vec2,
  /// the display's scale factor, which the window's size in pixels is that much more than
  /// its size in the screen's points
  display_scale: f64,
  /// whether the surface has yet to be made the size the window now is
  unfitted: bool,
  /// what the frame last put on the surface shows. none if the surface has had no frame
  /// since it was made or made again, or since the window was shown
  shown: Option<Frame>,
  again: Again,
  /// how many frames in a row could not be drawn, or were drawn on a surface that no longer
  /// suits the window
  failed: u32,
  /// how many times the panel's size the window is to become, once for each frame at most
  wanted_scale: Option<f32>,
}

/// everything a frame shows that can change from one to the next
#[derive(Clone, Debug, PartialEq)]
struct Frame {
  /// where each control stands
  values: [f32; 3],
  /// the window's width and height in pixels
  window: [u32; 2],
  /// how many times the panel's size it is drawn at
  scale: f32,
  edit: Option<Edit>,
  menu: Option<Menu>,
}

/// how often in a row the window asks for a frame that then cannot be drawn, before it
/// leaves off until something happens to it
const TRIES: u32 = 3;

/// whether the frame on the surface has been put there a second time.
///
/// wine draws a child window's vulkan surface into the host's window by copying it at the
/// moment a frame is presented, which is before the frame is there to copy. so what the
/// host's window shows is the frame before the last, and nothing at all after the first.
/// the last frame is presented once more, a tick or two later, when it is there to be copied
/// TODO: draw each frame once when wine copies a frame after it is shown
#[derive(Clone, Copy, Debug, PartialEq)]
enum Again {
  /// a new frame has just been drawn
  Fresh,
  /// a tick has passed since, and the same frame is due once more
  Due,
  Done,
}

/// whether the window asks baseview for a frame of its own accord: when the one on the
/// surface is not what the panel is like now or is due a second time, as long as there is
/// a surface to draw on and asking has not come to nothing too often
fn asks(stale: bool, again: Again, failed: u32, window: [u32; 2]) -> bool {
  (stale || again == Again::Due) && failed < TRIES && window[0] > 0 && window[1] > 0
}

/// whether a frame has to be drawn: when none is on the surface, or the one that is shows
/// anything other than what the panel is like now
fn due(
  shown: Option<&Frame>,
  values: [f32; 3],
  window: [u32; 2],
  scale: f32,
  edit: Option<&Edit>,
  menu: Option<&Menu>,
) -> bool {
  shown.is_none_or(|shown| {
    (shown.values, shown.window, shown.scale) != (values, window, scale)
      || shown.edit.as_ref() != edit
      || shown.menu.as_ref() != menu
  })
}

impl State {
  fn window(&self) -> [u32; 2] {
    [self.config.width, self.config.height]
  }

  /// how many times its own size the panel is drawn at
  fn scale(&self) -> f32 {
    let [_, _, width, _] = fitted(self.window());
    (f64::from(width) / self.display_scale) as f32 / WIDTH
  }

  /// where each control stands: at what was last set from here if that has yet to reach its
  /// parameter, or else where the parameter is. the parameters are looked at afresh
  fn values(&mut self, params: &CanWeTalkParams) -> [f32; 3] {
    Control::ALL.map(|control| {
      let value = control.param(params).unmodulated_normalized_value();
      self.sent.look(control, value);
      self.sent.shown(control, value)
    })
  }

  /// whether what is on the surface is not what the panel is like now
  fn stale(&mut self, params: &CanWeTalkParams) -> bool {
    let (values, window, scale) = (self.values(params), self.window(), self.scale());
    let (edit, menu) = (self.interaction.edit(), self.interaction.menu());
    due(self.shown.as_ref(), values, window, scale, edit, menu)
  }
}

impl Window {
  pub(super) fn new(
    context: WindowContext,
    gui: GuiContext,
    params: Arc<CanWeTalkParams>,
    shared: Arc<Shared>,
    told: mpsc::Receiver<ToWindow>,
    top_level: bool,
  ) -> Result<Self, HandlerError> {
    let size = context.size();
    (shared.display_scale).store(size.scale_factor.to_bits(), Ordering::Relaxed);
    #[cfg(target_os = "linux")]
    if top_level && let Err(error) = hint_shape(&context, size.scale_factor) {
      nice_error!("the window manager could not be told the editor's shape: {error}");
    }
    #[cfg(not(target_os = "linux"))]
    let _ = top_level;

    // direct3d 12 is started only where nothing else can draw: under wine with nvidia's
    // driver, dropping an instance that started it makes each later swapchain that has shown
    // a frame take 22 seconds or more to destroy, on the host's thread
    let others = wgpu::Backends::VULKAN | wgpu::Backends::METAL;
    let (surface, adapter) = match drawn_with(&context, others) {
      Ok(found) => found,
      Err(first) => drawn_with(&context, wgpu::Backends::DX12).map_err(|second| {
        HandlerError::from_boxed(format!("{first}, and with direct3d 12: {second}").into())
      })?,
    };
    // no more is asked of the adapter than the renderer uses, apart from textures as large as
    // the adapter has, for the window's own size
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
      required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
      ..Default::default()
    }))?;
    let mut config = surface
      .get_default_config(&adapter, size.physical.width, size.physical.height)
      .ok_or_else(|| {
        HandlerError::from_boxed("the graphics adapter cannot draw on the window".into())
      })?;
    // waiting for the display would hold up the host's own thread
    config.present_mode = wgpu::PresentMode::AutoNoVsync;
    surface.configure(&device, &config);

    let renderer = Renderer::new(&device, config.format);
    let flat = FlatRenderer::new(&device, config.format);
    let dials = Control::ALL.map(|control| control.dial(&params)).into();
    let interaction = Interaction::new(dials, [WIDTH, HEIGHT], READOUT_LETTERS, readout_shows);
    let mut panel = Panel::new();
    let values = Control::ALL.map(|control| control.param(&params).unmodulated_normalized_value());
    let [_, _, width, _] = fitted([config.width, config.height]);
    let pixel = WIDTH / width as f32;
    let scale = (f64::from(width) / size.scale_factor) as f32 / WIDTH;
    let picture = panel.picture(&params, values, &interaction, pixel, scale);
    let tick = context.create_timer(TICK)?;
    Ok(Self {
      context,
      gui,
      params,
      shared,
      told,
      tick,
      started: Cell::new(false),
      sized: Cell::new(None),
      clipboard: RefCell::new(None),
      state: RefCell::new(State {
        surface,
        config,
        device,
        queue,
        renderer,
        flat,
        panel,
        picture,
        interaction,
        sent: Sent::new(values),
        pointer: Vec2::ZERO,
        display_scale: size.scale_factor,
        unfitted: false,
        shown: None,
        again: Again::Done,
        failed: 0,
        wanted_scale: None,
      }),
    })
  }

  /// asks baseview for a frame, once it has drawn its first.
  ///
  /// baseview 0.4.2 on x11 asks the server to tell it of the next frame as soon as a redraw
  /// is requested, but only starts listening for the answer when the window is mapped
  /// (`PresentState::handle_present_notify` and `handle_window_mapped` in its
  /// `platform/x11/present.rs`). a request made before that is never answered, and every
  /// later one is dropped for waiting on it, so the window is never drawn at all. it asks
  /// for the first frame itself once the window is mapped.
  /// TODO: ask at any time once baseview holds a request until it can be answered
  fn redraw(&self) {
    if self.started.get() {
      self.context.request_redraw();
    }
  }

  /// the state, with the size the window was last given taken into it, or nothing while a
  /// handler further up already has it. what that leaves undone is made up for at the next
  /// tick, which draws the panel as it then is
  fn state(&self) -> Option<RefMut<'_, State>> {
    let mut state = self.state.try_borrow_mut().ok()?;
    if let Some(size) = self.sized.take() {
      // the surface is made to fit when the next frame is drawn, the latest size only
      state.config.width = size.physical.width;
      state.config.height = size.physical.height;
      state.display_scale = size.scale_factor;
      state.unfitted = true;
      (state.shown, state.failed) = (None, 0);
    }
    Some(state)
  }

  /// asks for a frame if the one on the surface is not what the panel is like now. the
  /// window is told when to draw by what happens to it, and this makes up for anything it
  /// was not told: a parameter moved with no report of it, or a frame that was lost
  fn refresh(&self) {
    let Some(mut state) = self.state() else {
      return;
    };
    let asks = asks(
      state.stale(&self.params),
      state.again,
      state.failed,
      state.window(),
    );
    drop(state);
    if asks {
      self.redraw();
    }
  }

  /// tells the panel of something, and then the host and the window of what came of it
  fn handle(&self, event: impl FnOnce(&mut State) -> Event) {
    let actions = {
      let Some(mut state) = self.state() else {
        return;
      };
      let values = state.values(&self.params);
      let event = event(&mut state);
      let moment = Moment {
        now: Instant::now(),
        values: &values,
        scale: state.scale(),
      };
      let actions = state.interaction.handle(event, moment);
      for action in &actions {
        match action {
          Action::Set(control, value) => state.sent.set(Control::ALL[*control], *value),
          Action::Scale(scale) => state.wanted_scale = Some(*scale),
          _ => {}
        }
      }
      let typing = state.interaction.edit().is_some();
      self.shared.typing.store(typing, Ordering::Relaxed);
      actions
    };
    // the state is let go of first: taking the keys can bring an event back in at once
    let setter = self.gui.param_setter();
    for action in actions {
      match action {
        Action::Begin(control) => {
          setter.begin_set_parameter(Control::ALL[control].param(&self.params));
        }
        Action::Set(control, value) => {
          setter.set_parameter_normalized(Control::ALL[control].param(&self.params), value);
          self.redraw();
        }
        Action::End(control) => setter.end_set_parameter(Control::ALL[control].param(&self.params)),
        // the window is made that size when the next frame is drawn, so that a stream of
        // these asks for no more sizes than are shown
        Action::Scale(_) => self.redraw(),
        Action::Focus => {
          if let Err(error) = self.context.focus() {
            nice_error!("the editor could not take the keys: {error}");
          }
        }
        Action::Redraw => self.redraw(),
        Action::Copy(text) => self.copy(text),
        Action::WantClipboard => self.read_clipboard(),
      }
    }
  }

  /// does something with the system's clipboard, which is opened at the first use of it. the
  /// state is not held meanwhile: the system may deliver other things to the window while
  /// it is asked
  fn clipboard<T>(
    &self,
    with: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>,
  ) -> Result<T, arboard::Error> {
    let mut clipboard =
      (self.clipboard.try_borrow_mut()).map_err(|_| arboard::Error::ClipboardOccupied)?;
    if clipboard.is_none() {
      *clipboard = Some(arboard::Clipboard::new()?);
    }
    with(clipboard.as_mut().expect("it was just made"))
  }

  fn copy(&self, text: String) {
    if let Err(error) = self.clipboard(|clipboard| clipboard.set_text(text)) {
      nice_error!("the editor could not copy to the clipboard: {error}");
    }
  }

  /// hands the panel the clipboard's text. it is read here and now: on windows and macos
  /// that is a call into the system, and on x11 a question to the program that owns the
  /// clipboard, which this window's own thread waits on and no thread of the host's
  fn read_clipboard(&self) {
    // a clipboard with no text on it is an error to arboard, and nothing to paste here
    let text = self.clipboard(|clipboard| clipboard.get_text()).ok();
    self.handle(|_| Event::Clipboard(text));
  }
}

impl WindowHandler for Window {
  fn draw(&self) -> Result<(), HandlerError> {
    // asked for with the state let go of: on windows the new size comes back in at once
    let wanted = self.state().and_then(|mut state| state.wanted_scale.take());
    if let Some(scale) = wanted
      && let Err(error) = self
        .context
        .resize(LogicalSize::new(WIDTH * scale, HEIGHT * scale))
    {
      nice_error!("the editor's window could not be resized: {error}");
    }
    self.started.set(true);
    // drawn at the next tick if this is from inside another handler
    let Some(mut state) = self.state() else {
      return Ok(());
    };
    // a window with no size, as a minimized one has, has no surface to draw on
    if state.window().contains(&0) {
      return Ok(());
    }
    // a surface made again has nothing on it, so the frame that was is no longer shown
    if std::mem::take(&mut state.unfitted) {
      state.surface.configure(&state.device, &state.config);
      state.shown = None;
    }
    let (values, window, scale) = (state.values(&self.params), state.window(), state.scale());
    let State {
      surface,
      config,
      device,
      queue,
      renderer,
      flat,
      panel,
      picture,
      interaction,
      shown,
      again,
      failed,
      ..
    } = &mut *state;
    // a frame that cannot be drawn is not asked for again from here, where the asking would
    // come straight back round: the next tick asks, a few times and then no more until the
    // window is sized or shown. getting the surface's next image can itself take a second
    let frame = match surface.get_current_texture() {
      Ok(frame) => frame,
      // the surface does not fit the window or is gone, and is made again
      Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
        surface.configure(device, config);
        *shown = None;
        *failed += 1;
        return Ok(());
      }
      Err(wgpu::SurfaceError::Timeout) => {
        *shown = None;
        *failed += 1;
        return Ok(());
      }
      Err(error @ (wgpu::SurfaceError::OutOfMemory | wgpu::SurfaceError::Other)) => {
        return Err(error.into());
      }
    };

    // the panel goes on as much of the window as has its shape
    let [left, top, width, height] = fitted(window);
    let pixel = WIDTH / width as f32;
    *picture = panel.picture(&self.params, values, interaction, pixel, scale);
    renderer.prepare(device, queue, &picture.scene, [width, height]);
    flat.prepare(
      device,
      queue,
      &picture.over,
      picture.scene.size,
      [width, height],
    );
    let view = (frame.texture).create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
      label: Some("canwetalk editor"),
    });
    if [width, height] != window {
      // the rest of the window, which the panel's own pass leaves as it finds it
      encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("canwetalk around"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
          view: &view,
          depth_slice: None,
          resolve_target: None,
          ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(AROUND),
            store: wgpu::StoreOp::Store,
          },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
      });
    }
    let place = Place {
      x: left as f32,
      y: top as f32,
      width: width as f32,
      height: height as f32,
    };
    renderer.render(&mut encoder, &view, place, [left, top, width, height]);
    flat.render(&mut encoder, &view, place, [left, top, width, height]);
    queue.submit([encoder.finish()]);
    let suboptimal = frame.suboptimal;
    frame.present();
    let drawn = Frame {
      values,
      window,
      scale,
      edit: interaction.edit().cloned(),
      menu: interaction.menu().copied(),
    };
    *again = match again {
      // the same frame a second time, a tick or more after the first
      Again::Due if shown.as_ref() == Some(&drawn) => Again::Done,
      _ if shown.as_ref() == Some(&drawn) => *again,
      _ => Again::Fresh,
    };
    *shown = Some(drawn);
    if suboptimal && *failed < TRIES {
      // drawn on as it was, and made to suit the window for the frames after. a surface
      // made again has nothing on it, so the same frame is drawn on it at the next tick
      surface.configure(device, config);
      *shown = None;
      *failed += 1;
    } else if !suboptimal {
      *failed = 0;
    }
    Ok(())
  }

  fn poll(&self) {
    while let Ok(told) = self.told.try_recv() {
      match told {
        ToWindow::Key(key) => self.handle(|_| Event::Key(key)),
        ToWindow::Shown => {
          if let Some(mut state) = self.state() {
            (state.shown, state.failed) = (None, 0);
          }
        }
        ToWindow::State => {
          // the state that was loaded may have the window at another size
          let scale = self.params.editor_scale.load(Ordering::Relaxed);
          let differs = self.state().is_some_and(|mut state| {
            let differs = (scale - state.scale()).abs() > 0.001;
            if differs {
              state.wanted_scale = Some(scale);
            }
            differs
          });
          if differs {
            self.redraw();
          }
        }
      }
    }
    self.refresh();
  }

  fn resized(&self, new_size: WindowSize) -> Result<(), HandlerError> {
    (self.shared.display_scale).store(new_size.scale_factor.to_bits(), Ordering::Relaxed);
    // kept for the state to take in, which a handler further up may have just now
    self.sized.set(Some(new_size));
    // the size is remembered as the scale the panel is drawn at in it
    let [_, _, width, _] = fitted([new_size.physical.width, new_size.physical.height]);
    let scale = (f64::from(width) / new_size.scale_factor) as f32 / WIDTH;
    (self.params.editor_scale).store(scale.clamp(SCALES[0], SCALES[1]), Ordering::Relaxed);
    self.redraw();
    Ok(())
  }

  fn on_event(&self, event: baseview::Event) -> EventStatus {
    match event {
      baseview::Event::Mouse(event) => {
        self.on_mouse(event);
        EventStatus::Captured
      }
      baseview::Event::Keyboard(event) => {
        if !(self.state()).is_some_and(|state| state.interaction.takes_keys()) {
          return EventStatus::Ignored;
        }
        if event.state == keyboard_types::KeyState::Down {
          for key in keys(&event) {
            self.handle(|_| Event::Key(key));
          }
        }
        EventStatus::Captured
      }
      baseview::Event::Window(WindowEvent::Unfocused) => {
        self.handle(|_| Event::Unfocused);
        EventStatus::Captured
      }
      baseview::Event::Window(WindowEvent::WillClose) => {
        self.handle(|_| Event::Closed);
        EventStatus::Captured
      }
      _ => EventStatus::Ignored,
    }
  }

  fn on_timer(&self, timer: &TimerHandle) {
    if timer == self.tick {
      self.handle(|_| Event::Tick);
      if let Some(mut state) = self.state()
        && state.again == Again::Fresh
      {
        state.again = Again::Due;
      }
      self.refresh();
    }
  }
}

impl Window {
  fn on_mouse(&self, event: MouseEvent) {
    match event {
      MouseEvent::CursorMoved {
        position,
        modifiers,
      } => self.handle(|state| {
        let at = point_at(
          fitted(state.window()),
          Vec2::new(position.x as f32, position.y as f32),
        );
        state.pointer = at;
        Event::Move {
          at: at.into(),
          // only the menu shows what the pointer is over
          over: (state.interaction.menu()).and_then(|_| state.picture.target_at(at)),
          fine: fine(modifiers),
        }
      }),
      MouseEvent::ButtonPressed { button, modifiers } => {
        let Some(button) = button_of(button) else {
          return;
        };
        self.handle(|state| Event::Press {
          button,
          at: state.pointer.into(),
          over: state.picture.target_at(state.pointer),
          reset: modifiers.ctrl() || modifiers.meta(),
        });
      }
      MouseEvent::ButtonReleased { button, .. } => {
        if let Some(button) = button_of(button) {
          self.handle(|_| Event::Release(button));
        }
      }
      MouseEvent::WheelScrolled { delta, modifiers } => {
        let lines = match delta {
          ScrollDelta::Lines { y, .. } => y,
          ScrollDelta::Pixels { y, .. } => y / PIXELS_PER_LINE,
        };
        self.handle(|state| Event::Wheel {
          over: state.picture.target_at(state.pointer),
          lines,
          fine: fine(modifiers),
        });
      }
      _ => {}
    }
  }
}

/// a surface on the window and an adapter that can draw on it, with one of some graphics apis
fn drawn_with(
  context: &WindowContext,
  backends: wgpu::Backends,
) -> Result<(wgpu::Surface<'static>, wgpu::Adapter), HandlerError> {
  let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
    backends,
    ..Default::default()
  });
  let surface = instance.create_surface(context.platform_handle())?;
  let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
    compatible_surface: Some(&surface),
    ..Default::default()
  }))?;
  Ok((surface, adapter))
}

/// tells the window manager that the window keeps the panel's shape, and how small and large
/// it may be, so that it holds the window to them while its frame is dragged. baseview sets
/// the sizes alone, and this takes the place of what it set
#[cfg(target_os = "linux")]
fn hint_shape(
  context: &WindowContext,
  display_scale: f64,
) -> Result<(), Box<dyn std::error::Error>> {
  use wgpu::rwh::{HasWindowHandle, RawWindowHandle};
  use x11rb::properties::{AspectRatio, WmSizeHints};

  use super::sized;

  let RawWindowHandle::Xlib(window) = context.window_handle()?.as_raw() else {
    return Err("the window is not an x11 one".into());
  };
  let (connection, _) = x11rb::connect(None)?;
  let pixels = |scale: f32| {
    let size = sized(scale).to_physical::<i32>(display_scale);
    (size.width, size.height)
  };
  let shape = AspectRatio::new(WIDTH as i32, HEIGHT as i32);
  let hints = WmSizeHints {
    min_size: Some(pixels(SCALES[0])),
    max_size: Some(pixels(SCALES[1])),
    aspect: Some((shape, shape)),
    ..WmSizeHints::default()
  };
  (hints.set_normal_hints(&connection, window.window as u32)?).check()?;
  Ok(())
}

/// whether a key that slows a knob down is held
fn fine(modifiers: keyboard_types::Modifiers) -> bool {
  modifiers.alt() || modifiers.shift()
}

fn button_of(button: MouseButton) -> Option<Button> {
  match button {
    MouseButton::Left => Some(Button::Left),
    MouseButton::Right => Some(Button::Right),
    _ => None,
  }
}

/// what a key press means to a value being typed. a key held with one that makes it a
/// command types nothing
fn keys(event: &keyboard_types::KeyboardEvent) -> Vec<Key> {
  use keyboard_types::{Key as Pressed, NamedKey};

  let command = event.modifiers.ctrl() || event.modifiers.meta();
  let extend = event.modifiers.shift();
  match &event.key {
    Pressed::Character(letters) if command => match letters.to_ascii_lowercase().as_str() {
      "a" => vec![Key::SelectAll],
      "c" => vec![Key::Copy],
      "x" => vec![Key::Cut],
      "v" => vec![Key::Paste],
      _ => Vec::new(),
    },
    Pressed::Character(letters) if !event.modifiers.alt() => {
      letters.chars().map(Key::Letter).collect()
    }
    Pressed::Named(named) => match named {
      NamedKey::Enter => vec![Key::Enter],
      NamedKey::Escape => vec![Key::Escape],
      NamedKey::Backspace => vec![Key::Backspace],
      NamedKey::Delete => vec![Key::Delete],
      NamedKey::ArrowLeft => vec![Key::Left { extend }],
      NamedKey::ArrowRight => vec![Key::Right { extend }],
      NamedKey::Home => vec![Key::Home { extend }],
      NamedKey::End => vec![Key::End { extend }],
      NamedKey::Tab => vec![Key::Tab],
      _ => Vec::new(),
    },
    _ => Vec::new(),
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::editor::interaction::Item;

  /// a frame of a window of the panel's own size, with nothing typed or chosen
  fn frame(values: [f32; 3]) -> Frame {
    Frame {
      values,
      window: [750, 375],
      scale: 1.0,
      edit: None,
      menu: None,
    }
  }

  /// whether a frame is due on a surface showing `shown`, with the panel like `now`
  fn is_due(shown: Option<&Frame>, now: &Frame) -> bool {
    let Frame {
      values,
      window,
      scale,
      edit,
      menu,
    } = now;
    due(
      shown,
      *values,
      *window,
      *scale,
      edit.as_ref(),
      menu.as_ref(),
    )
  }

  #[test]
  fn a_frame_is_due_when_none_is_shown_or_the_one_shown_is_not_the_panel_as_it_is() {
    let shown = frame([0.5, 0.5, 1.0]);
    // the same panel on a surface that shows it: nothing to draw, however often it is asked
    assert!(!is_due(Some(&shown), &shown));
    // a surface that has had no frame, as when it has just been made or made again
    assert!(is_due(None, &shown));
    // a control somewhere else, whoever moved it and whether or not anything said so
    assert!(is_due(Some(&shown), &frame([0.5, 0.25, 1.0])));
    // the window another size, or the panel drawn larger in it
    let resized = Frame {
      window: [900, 375],
      ..shown.clone()
    };
    assert!(is_due(Some(&shown), &resized));
    let scaled = Frame {
      scale: 1.5,
      ..shown.clone()
    };
    assert!(is_due(Some(&shown), &scaled));
    // the menu up, moved to another row, and away again
    let menu = |hovered| Frame {
      menu: Some(Menu {
        control: 0,
        at: [10.0, 10.0],
        hovered,
      }),
      ..shown.clone()
    };
    assert!(is_due(Some(&shown), &menu(None)));
    assert!(is_due(Some(&menu(None)), &menu(Some(Item::Reset))));
    assert!(is_due(Some(&menu(None)), &shown));
    assert!(!is_due(Some(&menu(None)), &menu(None)));
    // a value being typed, and one more letter of it
    let typed = |text: &str| Frame {
      edit: Some(Edit {
        control: 0,
        text: text.to_owned(),
        caret: text.len(),
        anchor: text.len(),
      }),
      ..shown.clone()
    };
    assert!(is_due(Some(&shown), &typed("1")));
    assert!(is_due(Some(&typed("1")), &typed("12")));
    assert!(!is_due(Some(&typed("12")), &typed("12")));
  }

  #[test]
  fn the_window_asks_for_a_frame_only_when_one_is_due_and_can_be_drawn() {
    let window = [750, 375];
    // nothing changed and the frame put there twice: nothing is asked for, tick after tick
    assert!(!asks(false, Again::Done, 0, window));
    // a frame that is not the panel as it is now
    assert!(asks(true, Again::Done, 0, window));
    // the frame just drawn waits a tick before it is due again, and is then asked for once
    assert!(!asks(false, Again::Fresh, 0, window));
    assert!(asks(false, Again::Due, 0, window));
    // frames that could not be drawn are asked for again a few times and then no more
    assert!(asks(true, Again::Done, TRIES - 1, window));
    assert!(!asks(true, Again::Done, TRIES, window));
    assert!(!asks(false, Again::Due, TRIES, window));
    // and a window with no size has nothing to draw on
    assert!(!asks(true, Again::Done, 0, [0, 375]));
    assert!(!asks(true, Again::Due, 0, [750, 0]));
  }

  #[test]
  fn a_value_of_the_editors_own_coming_back_is_no_new_frame_and_the_hosts_is() {
    // what the panel shows of the pitch as its parameter is looked at, the way the window
    // does before it decides whether to draw
    let mut sent = Sent::new([0.5, 0.5, 1.0]);
    let now = |sent: &mut Sent, parameter: f32| {
      sent.look(Control::Pitch, parameter);
      frame([sent.shown(Control::Pitch, parameter), 0.5, 1.0])
    };
    // the editor sets 0.6 and draws it, with the parameter still where it was
    sent.set(Control::Pitch, 0.6);
    let shown = now(&mut sent, 0.5);
    assert_eq!(shown.values[0], 0.6);
    // the parameter gets there: the same picture, whether or not the wrapper reports it
    assert!(!is_due(Some(&shown), &now(&mut sent, 0.6)));
    // the host moves it on with no report at all: seen the next time it is looked at
    assert!(is_due(Some(&shown), &now(&mut sent, 0.8)));
    // and the host moving it to where the editor once had it is drawn too
    let shown = now(&mut sent, 0.8);
    assert!(is_due(Some(&shown), &now(&mut sent, 0.6)));
  }
}
