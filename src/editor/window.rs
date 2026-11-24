//! the open window: the panel drawn and put in it, and baseview's events turned into what
//! the panel is told

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
use super::screen::Screen;
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
/// the picture's pixels as they are drawn and read back: blue first and in srgb, which is
/// how a window is given them
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;
/// how long a frame may take the graphics device before it is given up on
const DRAWN_WITHIN: Duration = Duration::from_secs(1);
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
  device: wgpu::Device,
  queue: wgpu::Queue,
  /// what frames are drawn on, made at the first frame of each size the window takes
  canvas: Option<Canvas>,
  screen: Screen,
  /// the window's width and height in pixels
  window: [u32; 2],
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
  /// and the same in the window's own points
  on_window: Vec2,
  /// the display's scale factor, which the window's size in pixels is that much more than
  /// its size in the screen's points
  display_scale: f64,
  /// what the frame last put in the window shows. none if it has had no frame since it was
  /// sized or shown
  shown: Option<Frame>,
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

/// what a frame is drawn on and read back from, for one size of window. there is no
/// swapchain on the window: under wine, making one again for each size the window takes
/// while it is dragged keeps the host's thread in the graphics driver
struct Canvas {
  size: [u32; 2],
  texture: wgpu::Texture,
  view: wgpu::TextureView,
  /// the frame's pixels once it is drawn, in rows of `row` bytes
  read: wgpu::Buffer,
  row: u32,
}

impl Canvas {
  fn new(device: &wgpu::Device, size: [u32; 2]) -> Self {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
      label: Some("canwetalk canvas"),
      size: wgpu::Extent3d {
        width: size[0],
        height: size[1],
        depth_or_array_layers: 1,
      },
      mip_level_count: 1,
      sample_count: 1,
      dimension: wgpu::TextureDimension::D2,
      format: FORMAT,
      usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
      view_formats: &[],
    });
    // wgpu copies rows to a buffer at a multiple of so many bytes each
    let row = (size[0] * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let read = device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("canwetalk canvas read back"),
      size: u64::from(row) * u64::from(size[1]),
      usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
      mapped_at_creation: false,
    });
    Self {
      size,
      view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
      texture,
      read,
      row,
    }
  }
}

/// whether a frame has to be drawn: when none is in the window, or the one that is shows
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
  /// how many times its own size the panel is drawn at
  fn scale(&self) -> f32 {
    let [_, _, width, _] = fitted(self.window);
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

  /// whether what is in the window is not what the panel is like now
  fn stale(&mut self, params: &CanWeTalkParams) -> bool {
    let (values, window, scale) = (self.values(params), self.window, self.scale());
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

    // direct3d 12 is started only where nothing else can draw: under wine it runs on vulkan
    // anyway, and wine's shader compiler cannot build the renderer's shaders for it
    let adapter = match adapter_of(wgpu::Backends::VULKAN | wgpu::Backends::METAL) {
      Ok(adapter) => adapter,
      Err(first) => adapter_of(wgpu::Backends::DX12).map_err(|second| {
        HandlerError::from_boxed(format!("{first}, and with direct3d 12: {second}").into())
      })?,
    };
    // no more is asked of the adapter than the renderer uses, apart from textures as large as
    // the adapter has, for the window's own size
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
      required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
      ..Default::default()
    }))?;
    let screen = Screen::new(&context)?;
    let window = [size.physical.width, size.physical.height];

    let renderer = Renderer::new(&device, FORMAT);
    let flat = FlatRenderer::new(&device, FORMAT);
    let dials = Control::ALL.map(|control| control.dial(&params)).into();
    let interaction = Interaction::new(dials, [WIDTH, HEIGHT], READOUT_LETTERS, readout_shows);
    let mut panel = Panel::new();
    let values = Control::ALL.map(|control| control.param(&params).unmodulated_normalized_value());
    let [_, _, width, _] = fitted(window);
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
        device,
        queue,
        canvas: None,
        screen,
        window,
        renderer,
        flat,
        panel,
        picture,
        interaction,
        sent: Sent::new(values),
        pointer: Vec2::ZERO,
        on_window: Vec2::ZERO,
        display_scale: size.scale_factor,
        shown: None,
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
      state.window = [size.physical.width, size.physical.height];
      state.display_scale = size.scale_factor;
      state.shown = None;
    }
    Some(state)
  }

  /// asks for a frame if the one in the window is not what the panel is like now. the
  /// window is told when to draw by what happens to it, and this makes up for anything it
  /// was not told: a parameter moved with no report of it, or a frame put off while a handler
  /// further up had the state
  fn refresh(&self) {
    let Some(mut state) = self.state() else {
      return;
    };
    // a window with no size has nothing to draw
    let asks = state.stale(&self.params) && !state.window.contains(&0);
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
    // a window with no size, as a minimized one has, has nothing to draw
    if state.window.contains(&0) {
      return Ok(());
    }
    let (values, window, scale) = (state.values(&self.params), state.window, state.scale());
    let State {
      device,
      queue,
      canvas,
      screen,
      renderer,
      flat,
      panel,
      picture,
      interaction,
      shown,
      ..
    } = &mut *state;
    let drawn_on = match canvas {
      Some(canvas) if canvas.size == window => canvas,
      _ => canvas.insert(Canvas::new(device, window)),
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
    let view = &drawn_on.view;
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
      label: Some("canwetalk editor"),
    });
    if [width, height] != window {
      // the rest of the window, which the panel's own pass leaves as it finds it
      encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("canwetalk around"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
          view,
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
    renderer.render(&mut encoder, view, place, [left, top, width, height]);
    flat.render(&mut encoder, view, place, [left, top, width, height]);
    encoder.copy_texture_to_buffer(
      drawn_on.texture.as_image_copy(),
      wgpu::TexelCopyBufferInfo {
        buffer: &drawn_on.read,
        layout: wgpu::TexelCopyBufferLayout {
          offset: 0,
          bytes_per_row: Some(drawn_on.row),
          rows_per_image: None,
        },
      },
      drawn_on.texture.size(),
    );
    let submitted = queue.submit([encoder.finish()]);
    // the frame is waited for here, on the window's thread, which is the host's on windows.
    // the device takes a millisecond or so for it, and the pixels go in the window at once
    let (mapped, when_mapped) = mpsc::channel();
    (drawn_on.read.slice(..)).map_async(wgpu::MapMode::Read, move |result| {
      // the receiver is only gone once the frame has been given up on
      let _ = mapped.send(result);
    });
    let waited = device.poll(wgpu::PollType::Wait {
      submission_index: Some(submitted),
      timeout: Some(DRAWN_WITHIN),
    });
    let read = match (waited, when_mapped.try_recv()) {
      (Ok(_), Ok(Ok(()))) => &drawn_on.read,
      // the editor's window closes on this error. the buffer may still be on its way to being
      // read, and is not drawn on again before then
      (waited, mapped) => {
        *canvas = None;
        return Err(HandlerError::from_boxed(
          format!("the frame could not be read back: {waited:?}, {mapped:?}").into(),
        ));
      }
    };
    let put = screen.show(window, drawn_on.row, &read.slice(..).get_mapped_range());
    read.unmap();
    put?;
    *shown = Some(Frame {
      values,
      window,
      scale,
      edit: interaction.edit().cloned(),
      menu: interaction.menu().copied(),
    });
    Ok(())
  }

  fn poll(&self) {
    while let Ok(told) = self.told.try_recv() {
      match told {
        ToWindow::Key(key) => self.handle(|_| Event::Key(key)),
        ToWindow::Shown => {
          if let Some(mut state) = self.state() {
            state.shown = None;
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
        let pixel = Vec2::new(position.x as f32, position.y as f32);
        let at = point_at(fitted(state.window), pixel);
        state.pointer = at;
        state.on_window = pixel / state.display_scale as f32;
        Event::Move {
          at: at.into(),
          window: state.on_window.into(),
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
          window: state.on_window.into(),
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

/// a graphics adapter of one of some apis
fn adapter_of(backends: wgpu::Backends) -> Result<wgpu::Adapter, wgpu::RequestAdapterError> {
  let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
    backends,
    ..Default::default()
  });
  pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
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

  /// whether a frame is due in a window showing `shown`, with the panel like `now`
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
    // the same panel in a window that shows it: nothing to draw, however often it is asked
    assert!(!is_due(Some(&shown), &shown));
    // a window that has had no frame, as when it has just been sized or shown
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
