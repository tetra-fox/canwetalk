//! the open window: the surface the panel is drawn on, and baseview's events turned into
//! what the panel is told

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread::JoinHandle;
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

use super::interaction::{Action, Button, Event, Interaction, Key, Moment, SCALES};
use super::panel::{HEIGHT, Panel, Picture, READOUT_LETTERS, WIDTH, readout_shows};
use super::sent::Sent;
use super::{Control, Shared, ToWindow};
use crate::params::CanWeTalkParams;

/// how often the panel is told that time has passed
const TICK: Duration = Duration::from_millis(50);
/// how many pixels of a wheel that scrolls by them count as one line
const PIXELS_PER_LINE: f32 = 20.0;

pub(super) struct Window {
  context: WindowContext,
  gui: GuiContext,
  params: Arc<CanWeTalkParams>,
  shared: Arc<Shared>,
  /// what the host's side of the editor and the clipboard's reader have to tell the window
  told: mpsc::Receiver<ToWindow>,
  /// the other end of that, for the clipboard's reader
  to_window: mpsc::Sender<ToWindow>,
  tick: TimerHandle,
  /// baseview calls a handler through a shared reference, always on the window's one thread
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
  /// how many times the panel's size the window is to become, once for each frame at most
  wanted_scale: Option<f32>,
  /// the system's clipboard, from the first copy on. on x11 what was copied is there for
  /// other programs only for as long as this is kept
  clipboard: Option<arboard::Clipboard>,
  /// the thread reading the clipboard, if one has been started
  reading: Option<JoinHandle<()>>,
}

impl Drop for State {
  fn drop(&mut self) {
    // the reader runs this library's code, which a host may unload once the window is gone.
    // it ends when the clipboard's owner answers or arboard gives up on it
    if let Some(reading) = self.reading.take() {
      // an error here is the reader having panicked, which leaves nothing to wait for
      let _ = reading.join();
    }
  }
}

impl State {
  /// how many times the panel's size the window is
  fn scale(&self) -> f32 {
    (f64::from(self.config.width) / self.display_scale) as f32 / WIDTH
  }

  /// where each control stands: at what was last set from here, or else where its parameter is
  fn values(&self, params: &CanWeTalkParams) -> [f32; 3] {
    Control::ALL.map(|control| {
      (self.sent.shown(control))
        .unwrap_or_else(|| control.param(params).unmodulated_normalized_value())
    })
  }
}

impl Window {
  pub(super) fn new(
    context: WindowContext,
    gui: GuiContext,
    params: Arc<CanWeTalkParams>,
    shared: Arc<Shared>,
    told: mpsc::Receiver<ToWindow>,
    to_window: mpsc::Sender<ToWindow>,
  ) -> Result<Self, HandlerError> {
    let size = context.size();
    (shared.display_scale).store(size.scale_factor.to_bits(), Ordering::Relaxed);

    let instance = wgpu::Instance::default();
    let surface = instance.create_surface(context.platform_handle())?;
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
      compatible_surface: Some(&surface),
      ..Default::default()
    }))?;
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
    let pixel = WIDTH / config.width as f32;
    let scale = (f64::from(config.width) / size.scale_factor) as f32 / WIDTH;
    let picture = panel.picture(&params, values, &interaction, pixel, scale);
    let tick = context.create_timer(TICK)?;
    Ok(Self {
      context,
      gui,
      params,
      shared,
      told,
      to_window,
      tick,
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
        sent: Sent::default(),
        pointer: Vec2::ZERO,
        display_scale: size.scale_factor,
        unfitted: false,
        wanted_scale: None,
        clipboard: None,
        reading: None,
      }),
    })
  }

  /// tells the panel of something, and then the host and the window of what came of it
  fn handle(&self, event: impl FnOnce(&mut State) -> Event) {
    let actions = {
      let mut state = self.state.borrow_mut();
      let event = event(&mut state);
      let values = state.values(&self.params);
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
          self.context.request_redraw();
        }
        Action::End(control) => setter.end_set_parameter(Control::ALL[control].param(&self.params)),
        // the window is made that size when the next frame is drawn, so that a stream of
        // these asks for no more sizes than are shown
        Action::Scale(_) => self.context.request_redraw(),
        Action::Focus => {
          if let Err(error) = self.context.focus() {
            nice_error!("the editor could not take the keys: {error}");
          }
        }
        Action::Redraw => self.context.request_redraw(),
        Action::Copy(text) => self.copy(text),
        Action::WantClipboard => self.read_clipboard(),
      }
    }
  }

  fn copy(&self, text: String) {
    let mut state = self.state.borrow_mut();
    let copied = match &mut state.clipboard {
      Some(clipboard) => clipboard.set_text(text),
      None => arboard::Clipboard::new().and_then(|mut clipboard| {
        let copied = clipboard.set_text(text);
        state.clipboard = Some(clipboard);
        copied
      }),
    };
    if let Err(error) = copied {
      nice_error!("the editor could not copy to the clipboard: {error}");
    }
  }

  /// has the clipboard's text come back as an event. it is read on a thread of its own: on
  /// x11 the read waits for the program that owns the clipboard to answer, for seconds if
  /// that one hangs
  fn read_clipboard(&self) {
    let mut state = self.state.borrow_mut();
    // a read already out answers for this one too
    if (state.reading.as_ref()).is_some_and(|reading| !reading.is_finished()) {
      return;
    }
    let (to_window, waker) = (self.to_window.clone(), self.context.waker());
    let reader = std::thread::Builder::new().spawn(move || {
      // a clipboard with no text on it is an error to arboard, and nothing to paste here
      let text = (arboard::Clipboard::new())
        .and_then(|mut clipboard| clipboard.get_text())
        .ok();
      // a window that has closed has no use for the text
      if to_window.send(ToWindow::Clipboard(text)).is_ok() {
        waker.request_poll();
      }
    });
    match reader {
      Ok(reading) => state.reading = Some(reading),
      Err(error) => nice_error!("the editor could not read the clipboard: {error}"),
    }
  }
}

impl WindowHandler for Window {
  fn draw(&self) -> Result<(), HandlerError> {
    // asked for with the state let go of: on windows the new size comes back in at once
    let wanted = self.state.borrow_mut().wanted_scale.take();
    if let Some(scale) = wanted
      && let Err(error) = self
        .context
        .resize(LogicalSize::new(WIDTH * scale, HEIGHT * scale))
    {
      nice_error!("the editor's window could not be resized: {error}");
    }
    let mut state = self.state.borrow_mut();
    if std::mem::take(&mut state.unfitted) {
      let State {
        surface,
        config,
        device,
        ..
      } = &*state;
      surface.configure(device, config);
    }
    let (values, scale) = (state.values(&self.params), state.scale());
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
      ..
    } = &mut *state;
    let frame = match surface.get_current_texture() {
      Ok(frame) => frame,
      // the surface does not fit the window or is gone: made again, it is drawn on next time
      Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
        surface.configure(device, config);
        self.context.request_redraw();
        return Ok(());
      }
      Err(wgpu::SurfaceError::Timeout) => {
        self.context.request_redraw();
        return Ok(());
      }
      Err(error @ (wgpu::SurfaceError::OutOfMemory | wgpu::SurfaceError::Other)) => {
        return Err(error.into());
      }
    };

    let pixel = WIDTH / config.width as f32;
    *picture = panel.picture(&self.params, values, interaction, pixel, scale);
    let size = [config.width, config.height];
    renderer.prepare(device, queue, &picture.scene, size);
    flat.prepare(device, queue, &picture.over, picture.scene.size, size);
    let view = (frame.texture).create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
      label: Some("canwetalk editor"),
    });
    let place = Place {
      x: 0.0,
      y: 0.0,
      width: size[0] as f32,
      height: size[1] as f32,
    };
    renderer.render(&mut encoder, &view, place, [0, 0, size[0], size[1]]);
    flat.render(&mut encoder, &view, place, [0, 0, size[0], size[1]]);
    queue.submit([encoder.finish()]);
    let suboptimal = frame.suboptimal;
    frame.present();
    // drawn on as it was, and made to fit for the next frame
    if suboptimal {
      surface.configure(device, config);
    }
    Ok(())
  }

  fn poll(&self) {
    while let Ok(told) = self.told.try_recv() {
      let changed = match told {
        ToWindow::Key(key) => {
          self.handle(|_| Event::Key(key));
          false
        }
        ToWindow::Clipboard(text) => {
          self.handle(|_| Event::Clipboard(text));
          false
        }
        ToWindow::Value(control, value) => self.state.borrow_mut().sent.reported(control, value),
        ToWindow::State => {
          let mut state = self.state.borrow_mut();
          state.sent.forget();
          // the state that was loaded may have the window at another size
          let scale = self.params.editor_scale.load(Ordering::Relaxed);
          if (scale - state.scale()).abs() > 0.001 {
            state.wanted_scale = Some(scale);
          }
          true
        }
      };
      if changed {
        self.context.request_redraw();
      }
    }
  }

  fn resized(&self, new_size: WindowSize) -> Result<(), HandlerError> {
    (self.shared.display_scale).store(new_size.scale_factor.to_bits(), Ordering::Relaxed);
    let mut state = self.state.borrow_mut();
    // the surface is made to fit when the next frame is drawn, the latest size only
    state.config.width = new_size.physical.width;
    state.config.height = new_size.physical.height;
    state.display_scale = new_size.scale_factor;
    state.unfitted = true;
    // the size is remembered as the scale it comes nearest, and a window that was given
    // another shape than the panel's is brought back to it at that scale
    let logical = new_size.logical;
    let scale = ((logical.width as f32 / WIDTH).max(logical.height as f32 / HEIGHT))
      .clamp(SCALES[0], SCALES[1]);
    self.params.editor_scale.store(scale, Ordering::Relaxed);
    let fits = |side: u32, points: f32| {
      (f64::from(side) - f64::from(points * scale) * new_size.scale_factor).abs() <= 1.0
    };
    if !(fits(new_size.physical.width, WIDTH) && fits(new_size.physical.height, HEIGHT)) {
      state.wanted_scale = Some(scale);
    }
    self.context.request_redraw();
    Ok(())
  }

  fn on_event(&self, event: baseview::Event) -> EventStatus {
    match event {
      baseview::Event::Mouse(event) => {
        self.on_mouse(event);
        EventStatus::Captured
      }
      baseview::Event::Keyboard(event) => {
        if !self.state.borrow().interaction.takes_keys() {
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
        // the picture is stretched over the whole window, whatever the display's scale
        let window = Vec2::new(state.config.width as f32, state.config.height as f32);
        let at =
          Vec2::new(position.x as f32, position.y as f32) / window * Vec2::new(WIDTH, HEIGHT);
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
