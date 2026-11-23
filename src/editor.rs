//! the plugin's window: a lit panel drawn by skeuo, and over each of its knobs an unseen
//! iced_audio knob that takes the mouse

use std::sync::Arc;

use iced_audio::param::{nice_to_iced, set_nice_param};
use iced_audio::style::knob as knob_style;
use iced_audio::{Gesture, Knob};
use nice_plug::prelude::*;
use nice_plug_iced::iced::baseview::dpi::LogicalSize;
use nice_plug_iced::iced::widget::{pin, stack, text};
use nice_plug_iced::iced::{Color, Element, Length, PollSubNotifier, Task, Theme, poll_events};
use nice_plug_iced::{
  IcedEditor, IcedEditorState, IcedNiceContext, IcedNiceSettings, PersistentState, application,
  create_iced_editor,
};
use skeuo::glam::{Mat4, Vec2, Vec3};
use skeuo::parts;
use skeuo::{Light, Look, Material, Mesh, Object, Scene};

use crate::params::CanWeTalkParams;

const WIDTH: f32 = 500.0;
const HEIGHT: f32 = 250.0;
/// the knobs' centers down the panel, and their size
const KNOB_Y: f32 = 122.0;
const KNOB_RADIUS: f32 = 38.0;
const KNOB_HEIGHT: f32 = 30.0;
/// how high the section the knobs are on stands off the panel
const UP: f32 = 2.5;

/// the knobs on the panel, left to right
#[derive(Clone, Copy, Debug)]
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

  /// the kind of knob it is: the two shifts match, and the mix is set apart
  fn knob(self) -> parts::Knob {
    match self {
      Self::Pitch | Self::Formant => parts::Knob::skirted(KNOB_RADIUS, KNOB_HEIGHT),
      Self::Mix => parts::Knob {
        skirt: 0.0,
        top: parts::Top::Flat,
        ..parts::Knob::knurled(KNOB_RADIUS * 0.8, KNOB_HEIGHT)
      },
    }
  }

  /// what is engraved under the knob
  fn name(self) -> &'static str {
    match self {
      Self::Pitch => "PITCH",
      Self::Formant => "FORMANT",
      Self::Mix => "MIX",
    }
  }

  /// what the two ends of the knob's scale are marked
  fn ends(self) -> (&'static str, &'static str) {
    match self {
      Self::Pitch | Self::Formant => ("-24", "+24"),
      Self::Mix => ("0", "100"),
    }
  }

  /// the knob's center across the panel
  fn x(self) -> f32 {
    match self {
      Self::Pitch => 105.0,
      Self::Formant => 250.0,
      Self::Mix => 395.0,
    }
  }
}

#[derive(Clone, Debug)]
enum Message {
  Turned(Control, Gesture),
  /// the host moved a parameter, so the panel is drawn again
  Polled,
}

struct Editor {
  params: Arc<CanWeTalkParams>,
  context: IcedNiceContext,
  panel: Panel,
}

pub(crate) fn create(params: Arc<CanWeTalkParams>) -> Option<IcedEditor> {
  create_iced_editor(
    IcedEditorState::from_size(LogicalSize::new(WIDTH, HEIGHT), 1.0),
    (),
    PollSubNotifier::new(),
    IcedNiceSettings::new().with_tile("CanWeTalk"),
    move |persistent: PersistentState<()>, context| {
      let params = params.clone();
      Ok(
        application(
          persistent,
          context,
          move |_persistent: PersistentState<()>, context| Editor {
            params: params.clone(),
            context,
            panel: panel(),
          },
          update,
          view,
        )
        .subscription(|_| poll_events().map(|()| Message::Polled))
        .run(),
      )
    },
  )
}

/// what the panel is made of: every mesh, the things on it that never move, and for each
/// control the first of its three meshes, which are its knob, the face let into it and the
/// line painted on it
struct Panel {
  meshes: Arc<[Mesh]>,
  fixed: Vec<Object>,
  knobs: [usize; 3],
}

fn panel() -> Panel {
  let mut meshes = Vec::new();
  let mut fixed = Vec::new();
  let mut put = |meshes: &mut Vec<Mesh>, mesh: Mesh, x: f32, y: f32, z: f32, material: Material| {
    fixed.push(Object {
      mesh: meshes.len(),
      transform: Mat4::from_translation(Vec3::new(x, y, z)),
      material,
    });
    meshes.push(mesh);
  };
  let steel = Vec3::new(0.30, 0.32, 0.36);
  let ink = Material::matte(Vec3::splat(0.86));
  let hole = Material::matte(Vec3::splat(0.012));
  let (middle, across) = (WIDTH / 2.0, HEIGHT / 2.0);

  // a brushed plate in a dark rim, with a darker section standing on it for the knobs
  put(
    &mut meshes,
    parts::plate(WIDTH, HEIGHT, 6.0),
    0.0,
    0.0,
    0.0,
    Material::brushed(steel),
  );
  put(
    &mut meshes,
    parts::frame(WIDTH, HEIGHT, 7.0, 7.0, 10.0),
    middle,
    across,
    0.0,
    Material::glossy(Vec3::new(0.05, 0.05, 0.06)),
  );
  put(
    &mut meshes,
    parts::slab(WIDTH - 44.0, 168.0, UP, 10.0),
    middle,
    KNOB_Y + 12.0,
    0.0,
    Material::brushed(steel * 0.6),
  );
  // the plugin's name along the top, and what it is at the other end
  put(
    &mut meshes,
    parts::legend("CANWETALK", 11.0, 1.9),
    86.0,
    24.0,
    0.0,
    Material::matte(Vec3::splat(0.07)),
  );
  put(
    &mut meshes,
    parts::legend("PITCH + FORMANT SHIFTER", 6.0, 1.0),
    WIDTH - 110.0,
    25.0,
    0.0,
    Material::matte(Vec3::splat(0.07)),
  );
  for control in Control::ALL {
    let (x, radius) = (control.x(), control.knob().radius);
    // a ring of dashes around each knob with its ends numbered, and its name under it
    put(
      &mut meshes,
      parts::dashes(19, parts::KNOB_SWEEP, radius + 8.0, 3.2, 0.6),
      x,
      KNOB_Y,
      UP,
      ink,
    );
    let (low, high) = control.ends();
    put(
      &mut meshes,
      parts::legend(low, 6.0, 1.0),
      x - radius - 6.0,
      KNOB_Y + radius + 6.0,
      UP,
      ink,
    );
    put(
      &mut meshes,
      parts::legend(high, 6.0, 1.0),
      x + radius + 6.0,
      KNOB_Y + radius + 6.0,
      UP,
      ink,
    );
    put(
      &mut meshes,
      parts::legend(control.name(), 8.0, 1.4),
      x,
      KNOB_Y + KNOB_RADIUS + 24.0,
      UP,
      ink,
    );
  }
  // a screw in a dark seat in each corner
  for (x, y) in [
    (22.0, 22.0),
    (WIDTH - 22.0, 22.0),
    (22.0, HEIGHT - 22.0),
    (WIDTH - 22.0, HEIGHT - 22.0),
  ] {
    if y < across {
      continue;
    }
    put(
      &mut meshes,
      parts::Button::round(11.0, 1.0).seat(2.0),
      x,
      y - 4.0,
      0.0,
      hole,
    );
    put(
      &mut meshes,
      parts::screw(5.5),
      x,
      y - 4.0,
      0.0,
      Material::chrome(Vec3::splat(0.6)),
    );
  }

  let mut knobs = [0; 3];
  for (slot, control) in knobs.iter_mut().zip(Control::ALL) {
    let knob = control.knob();
    *slot = meshes.len();
    meshes.extend([knob.body(), knob.cap(0.86), knob.line()]);
  }
  Panel {
    meshes: Arc::from(meshes),
    fixed,
    knobs,
  }
}

fn update(editor: &mut Editor, message: Message) -> Task<Message> {
  match message {
    Message::Turned(control, gesture) => {
      let setter = editor.context.nice_context.param_setter();
      set_nice_param(control.param(&editor.params), gesture, &setter);
    }
    Message::Polled => {}
  }
  Task::none()
}

fn view(editor: &Editor) -> Element<'_, Message, Theme> {
  let mut layers = stack![skeuo::iced::panel(scene(editor))];
  for control in Control::ALL {
    let param = control.param(&editor.params);
    let knob = Knob::new(nice_to_iced(param))
      .on_gesture(move |gesture| Message::Turned(control, gesture))
      .size(Length::Fixed(KNOB_RADIUS * 2.0))
      .style(Unseen);
    layers = layers.push(
      pin(knob)
        .x(control.x() - KNOB_RADIUS)
        .y(KNOB_Y - KNOB_RADIUS),
    );
    // the name is engraved on the panel, so only the value is written here
    let label = text(param.to_string()).size(12).color(Color::WHITE);
    layers = layers.push(
      pin(label)
        .x(control.x() - 22.0)
        .y(KNOB_Y + KNOB_RADIUS + 31.0),
    );
  }
  layers.into()
}

fn scene(editor: &Editor) -> Scene {
  let panel = &editor.panel;
  let mut objects = panel.fixed.clone();
  for (control, body) in Control::ALL.into_iter().zip(panel.knobs) {
    let turn = control.param(&editor.params).unmodulated_normalized_value();
    let transform = Mat4::from_translation(Vec3::Z * UP)
      * parts::knob_transform(control.x(), KNOB_Y, turn, parts::KNOB_SWEEP);
    let black = Vec3::new(0.035, 0.035, 0.04);
    let parts = [
      Material::glossy(black),
      Material::spun(Vec3::new(0.78, 0.79, 0.81)),
      Material::matte(black),
    ];
    for (part, material) in parts.into_iter().enumerate() {
      objects.push(Object {
        mesh: body + part,
        transform,
        material,
      });
    }
  }
  Scene {
    size: Vec2::new(WIDTH, HEIGHT),
    heights: [-8.0, 48.0],
    // seen a little from in front, under a wide soft light
    lean: Vec2::new(0.0, -0.3),
    light: Light {
      ambient: Vec3::splat(0.34),
      softness: 5.0,
      ..Light::default()
    },
    look: Look {
      soften: 0.25,
      ..Look::default()
    },
    meshes: panel.meshes.clone(),
    objects,
    lamps: Vec::new(),
  }
}

/// the look of a knob that leaves all the drawing to the panel under it
struct Unseen;

impl knob_style::StyleSheet for Unseen {
  type Style = Theme;

  fn idle(&self, _theme: &Theme) -> knob_style::Appearance {
    knob_style::Appearance::Circle(knob_style::CircleAppearance {
      color: Color::TRANSPARENT,
      border_width: 0.0,
      border_color: Color::TRANSPARENT,
      notch: knob_style::NotchShape::None,
    })
  }

  fn hovered(&self, theme: &Theme) -> knob_style::Appearance {
    self.idle(theme)
  }

  fn gesturing(&self, theme: &Theme) -> knob_style::Appearance {
    self.idle(theme)
  }

  fn disabled(&self, theme: &Theme) -> knob_style::Appearance {
    self.idle(theme)
  }
}
