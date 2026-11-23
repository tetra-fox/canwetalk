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
use skeuo::{Light, Look, Material, Mesh, Object, Scene, parts};

use crate::params::CanWeTalkParams;

const WIDTH: f32 = 480.0;
const HEIGHT: f32 = 220.0;
/// the knobs' centers down the panel, and their size
const KNOB_Y: f32 = 104.0;
const KNOB_RADIUS: f32 = 40.0;
const KNOB_HEIGHT: f32 = 30.0;

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

  /// the knob's center across the panel
  fn x(self) -> f32 {
    match self {
      Self::Pitch => 100.0,
      Self::Formant => 240.0,
      Self::Mix => 380.0,
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
  meshes: Arc<[Mesh]>,
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
            meshes: meshes(),
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

fn meshes() -> Arc<[Mesh]> {
  Arc::from([
    parts::plate(WIDTH, HEIGHT, 6.0),
    parts::knob_body(KNOB_RADIUS, KNOB_HEIGHT, 48, true),
    parts::knob_pointer(KNOB_RADIUS, KNOB_HEIGHT),
    parts::screw(6.0),
  ])
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
    let label = text(format!("{}  {}", param.name(), param))
      .size(13)
      .color(Color::WHITE);
    layers = layers.push(
      pin(label)
        .x(control.x() - KNOB_RADIUS)
        .y(KNOB_Y + KNOB_RADIUS + 22.0),
    );
  }
  layers.into()
}

fn scene(editor: &Editor) -> Scene {
  let mut objects = vec![Object {
    mesh: 0,
    transform: Mat4::IDENTITY,
    material: Material {
      specular: 0.15,
      shininess: 20.0,
      ..Material::matte(Vec3::new(0.30, 0.33, 0.38))
    },
  }];
  for control in Control::ALL {
    let turn = control.param(&editor.params).unmodulated_normalized_value();
    let transform = parts::knob_transform(control.x(), KNOB_Y, turn, parts::KNOB_SWEEP);
    objects.push(Object {
      mesh: 1,
      transform,
      material: Material::glossy(Vec3::new(0.08, 0.08, 0.09)),
    });
    objects.push(Object {
      mesh: 2,
      transform,
      material: Material::matte(Vec3::splat(0.95)),
    });
  }
  for (x, y) in [
    (14.0, 14.0),
    (WIDTH - 14.0, 14.0),
    (14.0, HEIGHT - 14.0),
    (WIDTH - 14.0, HEIGHT - 14.0),
  ] {
    objects.push(Object {
      mesh: 3,
      transform: Mat4::from_translation(Vec3::new(x, y, 0.0)) * Mat4::from_rotation_z(x + y),
      material: Material::glossy(Vec3::splat(0.6)),
    });
  }
  Scene {
    size: Vec2::new(WIDTH, HEIGHT),
    heights: [-8.0, 40.0],
    lean: Vec2::ZERO,
    light: Light::default(),
    look: Look::default(),
    meshes: editor.meshes.clone(),
    objects,
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
