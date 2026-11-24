//! what the panel is made of and where everything on it goes

use std::ops::Range;
use std::sync::Arc;

use nice_plug::prelude::Param;
use skeuo::glam::{Mat4, Vec2, Vec3};
use skeuo::parts;
use skeuo::{Color, Flat, Lamp, Light, Look, Material, Mesh, Object, Scene};

use super::Control;
use crate::params::CanWeTalkParams;
use controls::interaction::{Edit, Interaction, Item, Menu, Target};

pub(super) const WIDTH: f32 = 500.0;
pub(super) const HEIGHT: f32 = 250.0;
/// the part of a window so many pixels wide and tall that the panel is drawn on, as its left,
/// top, width and height: the largest of the panel's own shape that fits, in the middle. a
/// window of that shape is all of it, and one of any other has room left over at two sides
pub(super) fn fitted(window: [u32; 2]) -> [u32; 4] {
  let [wide, tall] = window.map(u64::from);
  let (across, down) = (WIDTH as u64, HEIGHT as u64);
  let (width, height) = if wide * down <= tall * across {
    (wide, wide * down / across)
  } else {
    (tall * across / down, tall)
  };
  [(wide - width) / 2, (tall - height) / 2, width, height].map(|side| side as u32)
}

/// the point of the panel that is at a pixel of the window, with the panel drawn on `place`
pub(super) fn point_at(place: [u32; 4], pixel: Vec2) -> Vec2 {
  let [left, top, width, height] = place.map(|side| side as f32);
  (pixel - Vec2::new(left, top)) / Vec2::new(width, height) * Vec2::new(WIDTH, HEIGHT)
}

/// how far a point slides across the picture for each unit of its height: seen a little
/// from in front
const LEAN: Vec2 = Vec2::new(0.0, -0.3);
/// the knobs' centers down the panel
const KNOB_Y: f32 = 94.0;
/// the names under the knobs, and under those the windows their values show in
const NAME_Y: f32 = 152.0;
const WINDOW_Y: f32 = 176.0;
const WINDOW: Vec2 = Vec2::new(66.0, 19.0);
/// how high the glass of a window is, which what it shows is lit on
const GLASS_UP: f32 = 0.5;
/// the figures a window shows its value in: how tall, and how wide their strokes. a unit
/// that is a word comes after them in smaller letters standing on the same line, and one
/// that is a sign as tall as they are, both in finer strokes
const READOUT_TALL: f32 = 8.0;
const READOUT_WEIGHT: f32 = 1.5;
const UNIT_TALL: f32 = 5.6;
const UNIT_WEIGHT: f32 = 1.15;
/// how many letters as tall as the figures fit in a window with two points clear at each end
pub(super) const READOUT_LETTERS: usize = {
  let mut letters = 0;
  while parts::legend_width(letters + 1, READOUT_TALL) <= WINDOW.x - 4.0 {
    letters += 1;
  }
  letters
};
const READOUT_COLOR: Vec3 = Vec3::new(1.0, 0.66, 0.14);
/// the strip along the bottom that carries the plugin's name
const STRIP_Y: f32 = 221.0;
const STRIP_UP: f32 = 1.5;
/// the lamp at the end of the strip, and how wide it is
const LAMP: Vec2 = Vec2::new(WIDTH - 34.0, STRIP_Y);
const LAMP_SIZE: f32 = 9.0;
const LAMP_COLOR: Vec3 = Vec3::new(1.0, 0.45, 0.08);
/// the menu: a flat list drawn over the picture, with a row for each item
const MENU_ROW: Vec2 = Vec2::new(84.0, 13.0);
/// the edge around the rows, and the room between every two pairs of rows that a line
/// across divides them in
const MENU_EDGE: f32 = 3.0;
const MENU_DIVIDE: f32 = 5.0;
const MENU_SIZE: Vec2 = Vec2::new(
  MENU_ROW.x + 2.0 * MENU_EDGE,
  menu_row_down(Item::ALL.len() - 1) + MENU_ROW.y + 2.0 * MENU_EDGE,
);
/// the menu's letters: how tall, how wide their strokes, and how far in from a row's left
const MENU_LETTERS: f32 = 6.0;
const MENU_WEIGHT: f32 = 1.0;
const MENU_INSET: f32 = 6.0;
const MENU_GROUND: Color = [0.075, 0.075, 0.085, 0.97];
const MENU_LINE: Color = [0.4, 0.4, 0.43, 1.0];
const MENU_INK: Color = [0.9, 0.9, 0.9, 1.0];
const MENU_MARK: Color = [0.95, 0.62, 0.14, 1.0];
const MENU_MARKED_INK: Color = [0.06, 0.05, 0.04, 1.0];

/// the grip the window is pulled larger and smaller by, drawn flat in its far corner: how
/// wide and tall the corner it takes up is, and how wide its lines are. these are in the
/// screen's points and not the panel's, so that it is the same size however large the
/// window is
const GRIP: f32 = 13.0;
const GRIP_WEIGHT: f32 = 1.2;
const GRIP_INK: Color = [1.0, 1.0, 1.0, 0.5];

/// whether a point of a panel shown at so many times its size is on the grip
fn on_grip(at: Vec2, scale: f32) -> bool {
  let corner = Vec2::new(WIDTH, HEIGHT);
  at.cmpgt(corner - GRIP / scale).all() && at.cmple(corner).all()
}

/// the grip: three lines slanting across the corner, each shorter than the last toward it
fn grip_shapes(flat: &mut Flat, scale: f32) {
  let corner = Vec2::new(WIDTH, HEIGHT);
  for nth in 1..=3u8 {
    // from the foot of the window to its side, the same way in from the corner on both
    let inward = (f32::from(nth) * 3.5 + 0.5) / scale;
    let clear = 1.5 / scale;
    flat.line(
      (corner - Vec2::new(inward, clear)).into(),
      (corner - Vec2::new(clear, inward)).into(),
      GRIP_WEIGHT / scale,
      GRIP_INK,
    );
  }
}

/// how far down from the first row's top a row's top is
const fn menu_row_down(row: usize) -> f32 {
  row as f32 * MENU_ROW.y + (row / 2) as f32 * MENU_DIVIDE
}

/// whether a readout has a letter for something typed into it
pub(super) fn readout_shows(letter: char) -> bool {
  letter == ' ' || parts::lettered(letter)
}

fn label(item: Item) -> &'static str {
  match item {
    Item::Edit => "EDIT VALUE",
    Item::Reset => "RESET",
    Item::Copy => "COPY VALUE",
    Item::Paste => "PASTE VALUE",
  }
}

/// where the menu's top left corner goes when it is asked for at a point: at the point, or
/// with the menu to the other side of the point where it would run off the panel
fn menu_corner(at: Vec2) -> Vec2 {
  let fits = (at + MENU_SIZE).cmple(Vec2::new(WIDTH, HEIGHT));
  Vec2::select(fits, at, at - MENU_SIZE)
}

/// what of a menu with its corner at a place is at a point, if any of it is
fn menu_at(corner: Vec2, at: Vec2) -> Option<Target> {
  let within = at - corner;
  if !(within.cmpge(Vec2::ZERO).all() && within.cmplt(MENU_SIZE).all()) {
    return None;
  }
  let row = (Item::ALL.into_iter().zip(0..)).find(|&(_, row)| {
    let down = within.y - MENU_EDGE - menu_row_down(row);
    (0.0..MENU_ROW.y).contains(&down) && (MENU_EDGE..MENU_EDGE + MENU_ROW.x).contains(&within.x)
  });
  Some(row.map_or(Target::Menu, |(item, _)| Target::Item(item)))
}

/// the menu as flat shapes, with lines as wide as `pixel`, which is one of the window's
fn menu_shapes(flat: &mut Flat, menu: &Menu, pixel: f32) {
  let corner = menu_corner(Vec2::from(menu.at));
  flat.rect(corner.into(), (corner + MENU_SIZE).into(), MENU_LINE);
  let line = Vec2::splat(pixel);
  flat.rect(
    (corner + line).into(),
    (corner + MENU_SIZE - line).into(),
    MENU_GROUND,
  );
  for (item, row) in Item::ALL.into_iter().zip(0..) {
    let top_left = corner + Vec2::new(MENU_EDGE, MENU_EDGE + menu_row_down(row));
    let marked = menu.hovered == Some(item);
    if marked {
      flat.rect(top_left.into(), (top_left + MENU_ROW).into(), MENU_MARK);
    }
    let letters = top_left + Vec2::new(MENU_INSET, (MENU_ROW.y - MENU_LETTERS) / 2.0);
    flat.legend(
      label(item),
      // onto the middles of pixels, where the upright and level strokes are sharpest
      (((letters / pixel).floor() + 0.5) * pixel).into(),
      MENU_LETTERS,
      MENU_WEIGHT,
      if marked { MENU_MARKED_INK } else { MENU_INK },
    );
    // a line across the middle of the room before each pair of rows after the first
    if row > 0 && row % 2 == 0 {
      let across = top_left + Vec2::new(0.0, -MENU_DIVIDE / 2.0);
      flat.rect(
        across.into(),
        (across + Vec2::new(MENU_ROW.x, pixel)).into(),
        MENU_LINE,
      );
    }
  }
}

impl Control {
  /// the kind of knob it is: the two shifts match, and the mix is smaller and set apart
  fn knob(self) -> parts::Knob {
    match self {
      Self::Pitch | Self::Formant => parts::Knob::skirted(38.0, 30.0),
      Self::Mix => parts::Knob {
        skirt: 0.0,
        top: parts::Top::Flat,
        ..parts::Knob::knurled(27.0, 26.0)
      },
    }
  }

  /// what is printed under the knob
  fn name(self) -> &'static str {
    match self {
      Self::Pitch => "PITCH",
      Self::Formant => "FORMANT",
      Self::Mix => "MIX",
    }
  }

  /// the figures printed around the knob, spread evenly from one end of its turn to the other
  fn figures(self) -> &'static [&'static str] {
    match self {
      Self::Pitch | Self::Formant => &["-24", "-12", "0", "+12", "+24"],
      Self::Mix => &["0", "50", "100"],
    }
  }

  /// how many marks the knob's scale has, and how many apart the long ones are
  fn marks(self) -> (u32, u32) {
    match self {
      Self::Pitch | Self::Formant => (25, 6),
      Self::Mix => (11, 5),
    }
  }

  /// the knob's center across the panel
  fn x(self) -> f32 {
    match self {
      Self::Pitch => 96.0,
      Self::Formant => 236.0,
      Self::Mix => 384.0,
    }
  }
}

/// objects on their way into a scene, and for each of them what a press on it is on
#[derive(Clone, Default)]
struct Drawn {
  objects: Vec<Object>,
  targets: Vec<Option<Target>>,
}

impl Drawn {
  fn put(&mut self, mesh: &Arc<Mesh>, at: Vec3, material: Material, target: Option<Target>) {
    self.place(mesh, Mat4::from_translation(at), material, target);
  }

  fn place(
    &mut self,
    mesh: &Arc<Mesh>,
    transform: Mat4,
    material: Material,
    target: Option<Target>,
  ) {
    self.objects.push(Object {
      mesh: Arc::clone(mesh),
      transform,
      material,
    });
    self.targets.push(target);
  }
}

/// the panel as it is drawn: the scene with what each of its objects is, in the same order,
/// and flat over the scene the grip, and the menu if it is up
pub(super) struct Picture {
  pub scene: Scene,
  targets: Vec<Option<Target>>,
  /// what is drawn flat over the scene
  pub over: Flat,
  /// where the menu's corner is
  menu: Option<Vec2>,
  /// how many times the panel's size it is shown at
  scale: f32,
}

impl Picture {
  /// what is drawn at a point of the panel. the menu is in front of everything, and the
  /// grip in front of everything on the panel
  pub(super) fn target_at(&self, at: Vec2) -> Option<Target> {
    (self.menu)
      .and_then(|corner| menu_at(corner, at))
      .or_else(|| on_grip(at, self.scale).then_some(Target::Grip))
      .or_else(|| self.scene.pick(at).and_then(|object| self.targets[object]))
  }
}

/// what a window shows, as meshes
struct Lettering {
  /// the figures, the unit and the selection it was made from
  of: (String, &'static str, Range<usize>),
  /// the figures that are not selected, each in its place among them all
  figures: Arc<Mesh>,
  /// how far across from the window's middle the figures' own middle is
  figures_at: f32,
  /// the unit, and where its middle is from the window's
  unit: Option<(Arc<Mesh>, Vec2)>,
  /// the figures that are selected, the block behind them, and how far across from the
  /// figures' middle the block's is
  selected: Option<(Arc<Mesh>, Arc<Mesh>, f32)>,
}

impl Lettering {
  fn new(figures: &str, unit: &'static str, selected: Range<usize>) -> Self {
    let count = figures.chars().count();
    let wide = parts::legend_width(count, READOUT_TALL).max(0.0);
    // a unit written after a space is a word, which stands off from the figures
    let name = unit.trim_start();
    let (gap, unit_tall) = if name.len() < unit.len() {
      (4.5, UNIT_TALL)
    } else {
      (2.5, READOUT_TALL)
    };
    let unit_wide = parts::legend_width(name.chars().count(), unit_tall);
    let all = if name.is_empty() {
      wide
    } else {
      wide + gap + unit_wide
    };
    // the figures with a space for each that is or is not selected, which keeps the others
    // where they are
    let only = |within: bool| -> String {
      (figures.chars().enumerate())
        .map(|(place, figure)| {
          if selected.contains(&place) == within {
            figure
          } else {
            ' '
          }
        })
        .collect()
    };
    let lettered = |text: &str| Arc::new(parts::legend(text, READOUT_TALL, READOUT_WEIGHT));
    Self {
      figures: lettered(&only(false)),
      figures_at: (wide - all) / 2.0,
      unit: (!name.is_empty()).then(|| {
        (
          Arc::new(parts::legend(name, unit_tall, UNIT_WEIGHT)),
          // down the window by as much as stands it on the line the figures stand on
          Vec2::new((all - unit_wide) / 2.0, (READOUT_TALL - unit_tall) / 2.0),
        )
      }),
      selected: (!selected.is_empty()).then(|| {
        // from the gap before the first of them to the gap after the last
        let from = parts::legend_gap(count, selected.start, READOUT_TALL);
        let to = parts::legend_gap(count, selected.end, READOUT_TALL);
        let half = Vec3::new((to - from) / 2.0, READOUT_TALL / 2.0 + 2.0, 0.0);
        let block = Mesh::cuboid(-half, half + Vec3::Z * 0.15);
        (lettered(&only(true)), Arc::new(block), (from + to) / 2.0)
      }),
      of: (figures.to_owned(), unit, selected),
    }
  }
}

/// the meshes of the panel, each made once
pub(super) struct Panel {
  /// everything that never moves or changes
  fixed: Drawn,
  /// for each control its knob, the face let into it and the line painted on it
  knobs: [[Arc<Mesh>; 3]; 3],
  lamp: Arc<Mesh>,
  glass: Arc<Mesh>,
  /// the lettering of what each window shows, kept for as long as it shows that
  readouts: [Option<Lettering>; 3],
  caret: Arc<Mesh>,
}

impl Panel {
  pub(super) fn new() -> Self {
    let mut fixed = Drawn::default();
    let mut put = |mesh: Mesh, x: f32, y: f32, z: f32, material: Material, target| {
      fixed.put(&Arc::new(mesh), Vec3::new(x, y, z), material, target);
    };
    let face = Vec3::new(0.05, 0.30, 0.66);
    let ink = Material::matte(Vec3::splat(0.92));
    let black = Vec3::new(0.03, 0.03, 0.035);
    let hole = Material::matte(Vec3::splat(0.012));
    let (middle, across) = (WIDTH / 2.0, HEIGHT / 2.0);

    // a brushed and anodized plate in a dark rim
    put(
      parts::plate(WIDTH, HEIGHT, 6.0),
      0.0,
      0.0,
      0.0,
      // less of a mirror than bare metal, so that the dye shows
      Material {
        metal: 0.3,
        ..Material::brushed(face)
      },
      None,
    );
    put(
      parts::frame(WIDTH, HEIGHT, 7.0, 7.0, 10.0),
      middle,
      across,
      0.0,
      Material::glossy(black),
      None,
    );
    // a black strip along the bottom with the plugin's name on it, and what it is at the other
    // end
    put(
      parts::slab(WIDTH - 20.0, 34.0, STRIP_UP, 4.0),
      middle,
      STRIP_Y,
      0.0,
      Material::glossy(black),
      None,
    );
    put(
      parts::legend("CANWETALK", 13.0, 2.6),
      92.0,
      STRIP_Y,
      STRIP_UP,
      ink,
      None,
    );
    put(
      parts::legend("PITCH + FORMANT SHIFTER", 6.0, 1.1),
      WIDTH - 136.0,
      STRIP_Y,
      STRIP_UP,
      Material::matte(Vec3::splat(0.55)),
      None,
    );
    for control in Control::ALL {
      let (x, radius) = (control.x(), control.knob().radius);
      // each knob stands in a polished ring, with a scale of marks around it and figures
      // outside them
      let ring = [
        [radius + 6.5, 0.0],
        [radius + 6.5, 1.6],
        [radius + 5.2, 2.8],
        [radius + 2.0, 2.8],
        [radius + 0.9, 1.6],
        [radius + 0.9, 0.0],
      ];
      put(
        Mesh::lathe(&ring, 96, true),
        x,
        KNOB_Y,
        0.0,
        Material::chrome(Vec3::splat(0.72)),
        None,
      );
      let (marks, long_every) = control.marks();
      put(
        parts::ticks(
          marks,
          parts::KNOB_SWEEP,
          radius + 9.5,
          radius + 12.5,
          long_every,
        ),
        x,
        KNOB_Y,
        0.0,
        ink,
        None,
      );
      let figures = control.figures();
      for (index, figure) in figures.iter().enumerate() {
        let turn = index as f32 / (figures.len() - 1) as f32;
        let (sin, cos) = ((turn - 0.5) * parts::KNOB_SWEEP).sin_cos();
        let out = radius + 23.5;
        put(
          parts::legend(figure, 6.5, 0.95),
          x + out * sin,
          KNOB_Y - out * cos,
          0.0,
          ink,
          None,
        );
      }
      put(
        parts::legend(control.name(), 8.0, 1.7),
        x,
        NAME_Y,
        0.0,
        ink,
        None,
      );
      // the bezel of the window its value shows in. the glass set down in it changes with
      // what it shows
      put(
        parts::frame(WINDOW.x + 6.0, WINDOW.y + 6.0, 3.0, 2.2, 3.5),
        x,
        WINDOW_Y,
        0.0,
        Material::glossy(black),
        Some(Target::Readout(control as usize)),
      );
    }
    put(
      parts::led_seat(LAMP_SIZE),
      LAMP.x,
      LAMP.y,
      STRIP_UP,
      hole,
      None,
    );
    // a screw in a dark seat in each corner of the plate above the strip, each with its slot
    // left at another angle
    for (x, y, turn) in [
      (20.0, 20.0, 0.5),
      (WIDTH - 20.0, 20.0, 2.1),
      (20.0, 186.0, 1.2),
      (WIDTH - 20.0, 186.0, 2.8),
    ] {
      put(parts::screw_seat(4.5, 1.6), x, y, 0.0, hole, None);
      put(
        parts::screw(4.5).transformed(Mat4::from_rotation_z(turn)),
        x,
        y,
        0.0,
        Material::chrome(Vec3::splat(0.6)),
        None,
      );
    }

    let knobs = Control::ALL.map(|control| {
      let knob = control.knob();
      [knob.body(), knob.cap(0.74), knob.line()].map(Arc::new)
    });
    Self {
      fixed,
      knobs,
      lamp: Arc::new(parts::led(LAMP_SIZE)),
      glass: Arc::new(Mesh::cuboid(
        (-WINDOW / 2.0).extend(0.0),
        (WINDOW / 2.0).extend(GLASS_UP),
      )),
      readouts: [None, None, None],
      caret: Arc::new(Mesh::cuboid(
        Vec3::new(-0.6, -READOUT_TALL / 2.0 - 2.0, 0.0),
        Vec3::new(0.6, READOUT_TALL / 2.0 + 2.0, 0.3),
      )),
    }
  }

  /// the panel with each control at its value, in the order of [`Control::ALL`], and
  /// whatever is being typed or chosen
  pub(super) fn picture(
    &mut self,
    params: &CanWeTalkParams,
    values: [f32; 3],
    interaction: &Interaction,
    pixel: f32,
    scale: f32,
  ) -> Picture {
    // the scene owns this frame's objects, so the ones that never move are copied into it
    let mut drawn = self.fixed.clone();
    for (index, control) in Control::ALL.into_iter().enumerate() {
      let param = control.param(params);
      let turn = values[index];
      let transform = parts::knob_transform(control.x(), KNOB_Y, turn, parts::KNOB_SWEEP);
      let materials = [
        // black plastic light enough for its shape to show under the light
        Material {
          specular: 0.5,
          shininess: 28.0,
          reflect: 0.35,
          ..Material::matte(Vec3::new(0.075, 0.075, 0.08))
        },
        Material {
          specular: 1.0,
          ..Material::spun(Vec3::new(0.5, 0.51, 0.54))
        },
        Material::matte(Vec3::splat(0.92)),
      ];
      for (mesh, material) in self.knobs[index].iter().zip(materials) {
        drawn.place(mesh, transform, material, Some(Target::Knob(index)));
      }
      let edit = interaction.edit().filter(|edit| edit.control == index);
      let value = param.normalized_value_to_string(turn, false);
      self.readout(&mut drawn, index, control, (&value, param.unit()), edit);
    }
    // the lamp is as bright as the mix is wet, and dark when nothing of the shift is heard
    let wet = values[Control::Mix as usize];
    drawn.put(
      &self.lamp,
      LAMP.extend(STRIP_UP),
      parts::led_material(LAMP_COLOR, wet, LAMP_SIZE),
      None,
    );
    let menu = interaction.menu();
    let mut over = Flat::default();
    grip_shapes(&mut over, scale);
    if let Some(menu) = menu {
      menu_shapes(&mut over, menu, pixel);
    }
    Picture {
      over,
      scale,
      menu: menu.map(|menu| menu_corner(Vec2::from(menu.at))),
      scene: Scene {
        size: Vec2::new(WIDTH, HEIGHT),
        heights: [-8.0, 48.0],
        lean: LEAN,
        // under a wide soft light
        light: Light {
          ambient: Vec3::splat(0.26),
          softness: 3.0,
          ..Light::default()
        },
        look: Look {
          soften: 0.25,
          ..Look::default()
        },
        objects: drawn.objects,
        lamps: vec![Lamp {
          at: LAMP.extend(STRIP_UP + LAMP_SIZE),
          color: LAMP_COLOR * wet,
          reach: LAMP_SIZE * 4.0,
        }],
      },
      targets: drawn.targets,
    }
  }

  /// a control's window: dark glass with its value and unit lit on it. while the value is
  /// typed the glass is lit too and shows what has been typed, with a caret in it or, for
  /// as long as all of it is selected, dark on a bright block
  fn readout(
    &mut self,
    drawn: &mut Drawn,
    index: usize,
    control: Control,
    value: (&str, &'static str),
    edit: Option<&Edit>,
  ) {
    let target = Some(Target::Readout(index));
    let at = Vec3::new(control.x(), WINDOW_Y, 0.0);
    let glass = Material {
      reflect: 0.2,
      // sized as a much larger pane, which keeps the sheen to one corner
      ..Material::pane(Vec3::new(0.012, 0.01, 0.008), WINDOW.x * 3.5)
    };
    let glass = match edit {
      Some(_) => glass.lit(READOUT_COLOR * 0.5, 0.45, 0.0),
      None => glass,
    };
    drawn.put(&self.glass, at, glass, target);

    let (figures, unit) = edit.map_or(value, |edit| (&edit.text, ""));
    let selected = edit.map_or(0..0, Edit::selected);
    let lettering = (self.readouts[index].take())
      .filter(|shown| shown.of.0 == figures && shown.of.1 == unit && shown.of.2 == selected)
      .unwrap_or_else(|| Lettering::new(figures, unit, selected));
    let lit = Material::lamp(READOUT_COLOR);
    let on_glass = at + Vec3::Z * GLASS_UP;
    let figures_at = on_glass + Vec3::X * lettering.figures_at;
    drawn.put(&lettering.figures, figures_at, lit, target);
    if let Some((figures, block, across)) = &lettering.selected {
      drawn.put(block, figures_at + Vec3::X * *across, lit, target);
      let ink = Material::matte(Vec3::ZERO);
      drawn.put(figures, figures_at + Vec3::Z * 0.15, ink, target);
    }
    if let Some((unit, unit_at)) = &lettering.unit {
      drawn.put(unit, on_glass + unit_at.extend(0.0), lit, target);
    }
    if let Some(edit) = edit {
      let across = parts::legend_gap(edit.text.chars().count(), edit.caret, READOUT_TALL);
      drawn.put(&self.caret, figures_at + Vec3::X * across, lit, target);
    }
    self.readouts[index] = Some(lettering);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_menu_opens_at_the_pointer_and_turns_from_the_panels_edges() {
    let panel = Vec2::new(WIDTH, HEIGHT);
    // on open panel, and over a knob or a readout all the same, its corner is the pointer
    for at in [
      Vec2::new(150.0, 150.0),
      Vec2::new(Control::Pitch.x(), KNOB_Y),
      Vec2::new(Control::Mix.x(), WINDOW_Y - 20.0),
      Vec2::ZERO,
      panel - MENU_SIZE,
    ] {
      assert_eq!(menu_corner(at), at);
    }
    // where it would run off the right or the foot it lies to the other side of the pointer
    let right = Vec2::new(WIDTH - 10.0, 100.0);
    assert_eq!(menu_corner(right), right - Vec2::X * MENU_SIZE.x);
    let foot = Vec2::new(100.0, HEIGHT - 10.0);
    assert_eq!(menu_corner(foot), foot - Vec2::Y * MENU_SIZE.y);
    assert_eq!(menu_corner(panel), panel - MENU_SIZE);
    // and wherever it is asked for all of it is on the panel
    for x in 0..=WIDTH as u16 {
      for y in 0..=HEIGHT as u16 {
        let corner = menu_corner(Vec2::new(f32::from(x), f32::from(y)));
        assert!(
          corner.cmpge(Vec2::ZERO).all() && (corner + MENU_SIZE).cmple(panel).all(),
          "asked for at {x} {y}, the menu at {corner} is off the panel"
        );
      }
    }
  }

  #[test]
  fn a_point_of_the_menu_is_a_row_the_plate_or_not_the_menu() {
    let corner = Vec2::new(100.0, 50.0);
    let row = |row: usize| corner + Vec2::new(MENU_SIZE.x / 2.0, MENU_EDGE + menu_row_down(row));
    for (item, at) in Item::ALL.into_iter().zip(0..) {
      // from its first line of points to its last
      assert_eq!(menu_at(corner, row(at)), Some(Target::Item(item)));
      let last = row(at) + Vec2::Y * (MENU_ROW.y - 0.01);
      assert_eq!(menu_at(corner, last), Some(Target::Item(item)));
    }
    // the edge around the rows and the room between the pairs are the menu and no row
    assert_eq!(menu_at(corner, corner + 1.0), Some(Target::Menu));
    assert_eq!(menu_at(corner, row(2) - Vec2::Y * 2.0), Some(Target::Menu));
    assert_eq!(
      menu_at(corner, row(0) - Vec2::X * (MENU_ROW.x / 2.0 + 1.0)),
      Some(Target::Menu)
    );
    // and past its sides is not the menu
    assert_eq!(menu_at(corner, corner - 0.5), None);
    assert_eq!(menu_at(corner, corner + MENU_SIZE), None);
    assert_eq!(
      menu_at(corner, corner + MENU_SIZE - 0.01),
      Some(Target::Menu)
    );
  }

  #[test]
  fn the_grip_is_the_windows_far_corner_and_as_large_on_the_screen_at_any_size() {
    let corner = Vec2::new(WIDTH, HEIGHT);
    for scale in [0.75, 1.0, 3.0] {
      // so many of the screen's points in from the corner, whatever that is of the panel's
      let inside = corner - (GRIP - 0.5) / scale;
      let outside = corner - (GRIP + 0.5) / scale;
      assert!(on_grip(inside, scale) && on_grip(corner, scale), "{scale}");
      assert!(!on_grip(outside, scale), "{scale}");
      assert!(!on_grip(Vec2::new(inside.x, outside.y), scale), "{scale}");
      assert!(!on_grip(Vec2::new(outside.x, inside.y), scale), "{scale}");
    }
  }

  #[test]
  fn the_panel_is_fitted_into_a_window_of_any_shape_at_its_own() {
    // a window of the panel's shape is all the panel's
    assert_eq!(fitted([750, 375]), [0, 0, 750, 375]);
    assert_eq!(fitted([500, 250]), [0, 0, 500, 250]);
    // one too wide has the room left over at its sides, and one too tall above and below
    assert_eq!(fitted([1100, 375]), [175, 0, 750, 375]);
    assert_eq!(fitted([750, 640]), [0, 132, 750, 375]);
    assert_eq!(fitted([1908, 1077]), [0, 61, 1908, 954]);
    // odd sizes round down, and what is left over is never more than a pixel off the middle
    assert_eq!(fitted([751, 375]), [0, 0, 750, 375]);
    assert_eq!(fitted([750, 376]), [0, 0, 750, 375]);
    assert_eq!(fitted([3, 100]), [0, 49, 3, 1]);
    for window in [[640, 480], [333, 777], [2250, 1125], [1, 1], [0, 0]] {
      let [left, top, width, height] = fitted(window);
      assert!(
        left + width <= window[0] && top + height <= window[1],
        "{window:?}"
      );
      // the panel's shape, to within the pixel that rounding takes
      assert!(width.abs_diff(height * 2) <= 1, "{window:?}");
      // and as large as fits: one of the two reaches from side to side
      assert!(width == window[0] || height == window[1], "{window:?}");
    }
  }

  #[test]
  fn the_pointer_is_on_the_panel_where_the_panel_is_drawn() {
    // a wide window with the panel in its middle at one and a half pixels to a point
    let place = fitted([1100, 375]);
    assert_eq!(point_at(place, Vec2::new(175.0, 0.0)), Vec2::ZERO);
    assert_eq!(
      point_at(place, Vec2::new(925.0, 375.0)),
      Vec2::new(WIDTH, HEIGHT)
    );
    assert_eq!(
      point_at(place, Vec2::new(550.0, 187.5)),
      Vec2::new(250.0, 125.0)
    );
    // on the room left over it is off the panel, where nothing is picked
    assert!(point_at(place, Vec2::new(100.0, 100.0)).x < 0.0);
    assert!(point_at(place, Vec2::new(1000.0, 100.0)).x > WIDTH);
    // and in a tall one
    let place = fitted([750, 640]);
    assert_eq!(
      point_at(place, Vec2::new(375.0, 132.0)),
      Vec2::new(250.0, 0.0)
    );
    assert!(point_at(place, Vec2::new(375.0, 100.0)).y < 0.0);
  }

  #[test]
  fn the_longest_value_fits_its_window() {
    // the widest a shift is written, and the most letters that are let be typed
    let widest = Lettering::new("-24.00", " st", 0..0);
    let (_, unit_at) = widest.unit.as_ref().expect("a unit");
    let right = unit_at.x + parts::legend_width(2, UNIT_TALL) / 2.0;
    assert!(right <= WINDOW.x / 2.0 - 1.0, "{right}");
    assert!(parts::legend_width(READOUT_LETTERS, READOUT_TALL) <= WINDOW.x - 4.0);
    assert!(READOUT_LETTERS >= "-12.5 st".len());
  }
}
