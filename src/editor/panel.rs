//! what the panel is made of and where everything on it goes

use std::f32::consts::{FRAC_PI_2, PI};
use std::ops::Range;
use std::sync::Arc;

use controls::flat::Flat;
use controls::interaction::{Edit, Reading, Target};
use controls_window::{Gpu, Skin, View, wgpu};
use skeuo::glam::{Mat4, Vec2, Vec3};
use skeuo::parts;
use skeuo::{Lamp, Light, Look, Material, Mesh, Object, Place, Renderer, Scene};

use super::Control;

pub(super) const WIDTH: f32 = 500.0;
pub(super) const HEIGHT: f32 = 250.0;
/// how far a point slides across the picture for each unit of its height: seen a little
/// from in front
const LEAN: Vec2 = Vec2::new(0.0, -0.3);
/// how wide the rack ear at each end of the plate is
const EAR: f32 = 32.0;
/// the names of the groups of knobs, along the top
const HEADING_Y: f32 = 13.0;
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
const LAMP: Vec2 = Vec2::new(WIDTH - 54.0, STRIP_Y);
const LAMP_SIZE: f32 = 11.0;
const LAMP_COLOR: Vec3 = Vec3::new(1.0, 0.45, 0.08);
impl Control {
  /// the kind of knob it is: the two shifts match, and the mix is smaller and set apart
  fn knob(self) -> parts::Knob {
    match self {
      Self::Pitch | Self::Formant => parts::Knob {
        skirt: 0.2,
        ..parts::Knob::skirted(36.0, 30.0)
      },
      Self::Mix => parts::Knob {
        skirt: 0.0,
        top: parts::Top::Flat,
        ..parts::Knob::knurled(26.0, 26.0)
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
      Self::Pitch => 102.0,
      Self::Formant => 240.0,
      Self::Mix => 374.0,
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

/// a flat mesh leaned over to the right the way slanted letters are: `slope` further across
/// for each point further up the panel
fn slanted(mut mesh: Mesh, slope: f32) -> Mesh {
  for vertex in &mut mesh.vertices {
    vertex.position[0] -= vertex.position[1] * slope;
  }
  mesh
}

/// what a window shows, as meshes
struct Lettering {
  /// the figures, the unit and the selection it was made from
  of: (String, String, Range<usize>),
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
  fn new(figures: &str, unit: &str, selected: Range<usize>) -> Self {
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
      of: (figures.to_owned(), unit.to_owned(), selected),
    }
  }
}

/// the panel as skeuo draws it: its meshes, each made once, and the scene of the frame last
/// laid out
pub struct Panel {
  renderer: Renderer,
  /// the scene last laid out, which is what the pointer is on. none until the first is
  scene: Option<Scene>,
  /// what each of the scene's objects is, in the same order
  targets: Vec<Option<Target>>,
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
  pub(super) fn new(gpu: &Gpu) -> Self {
    let mut fixed = Drawn::default();
    let mut put = |mesh: Mesh, x: f32, y: f32, z: f32, material: Material, target| {
      fixed.put(&Arc::new(mesh), Vec3::new(x, y, z), material, target);
    };
    let face = Vec3::new(0.05, 0.30, 0.66);
    // less of a mirror than bare metal and with less of a shine, so that the dye shows
    let anodized = Material {
      metal: 0.3,
      specular: 0.28,
      wear: 0.7,
      ..Material::brushed(face)
    };
    let ink = Material::matte(Vec3::splat(0.92));
    let faint = Material::matte(Vec3::splat(0.55));
    let red = Material::matte(Vec3::new(0.85, 0.07, 0.05));
    let black = Vec3::new(0.03, 0.03, 0.035);
    let hole = Material::matte(Vec3::splat(0.012));
    let chrome = Material::chrome(Vec3::splat(0.8));
    // the far side of anything cut into the plate, which the light reaches
    let lip = Material::matte(face.lerp(Vec3::ONE, 0.5));
    let (middle, across) = (WIDTH / 2.0, HEIGHT / 2.0);

    // a brushed and anodized plate, with a low rim along each edge that slopes down to it
    put(
      parts::plate(WIDTH, HEIGHT, 6.0),
      0.0,
      0.0,
      0.0,
      anodized,
      None,
    );
    let rim = |half: f32| {
      let outline = [[0.0, 0.0], [0.0, 1.3], [0.9, 1.3], [2.6, 0.0]].map(Vec2::from);
      Mesh::extruded(&outline, |_| half, 1.0)
    };
    for (half, x, y, turn) in [
      (middle, middle, 0.0, 0.0),
      (middle, middle, HEIGHT, PI),
      (across, 0.0, across, -FRAC_PI_2),
      (across, WIDTH, across, FRAC_PI_2),
    ] {
      let turned = rim(half).transformed(Mat4::from_rotation_z(turn));
      put(turned, x, y, 0.0, anodized, None);
    }

    // a rack ear at each end, marked off by a groove, with a screw through a long hole near
    // its top and its bottom, each screw's cross left at another angle
    for x in [EAR, WIDTH - EAR] {
      for (from, to, up, material) in [(0.5, 1.2, 0.1, lip), (-0.5, 0.5, 0.2, hole)] {
        let groove = Mesh::cuboid(Vec3::new(from, 5.0, 0.0), Vec3::new(to, HEIGHT - 5.0, up));
        put(groove, x, 0.0, 0.0, material, None);
      }
    }
    for (x, y, turn) in [
      (EAR / 2.0, 30.0, 0.4),
      (EAR / 2.0, HEIGHT - 30.0, 1.3),
      (WIDTH - EAR / 2.0, 30.0, 2.3),
      (WIDTH - EAR / 2.0, HEIGHT - 30.0, 0.9),
    ] {
      put(parts::slot(16.0, 9.0), x + 0.5, y + 0.5, 0.0, lip, None);
      put(parts::slot(16.0, 9.0), x, y, 0.05, hole, None);
      let washer = Material::satin(Vec3::splat(0.05));
      put(parts::bezel(2.0, 6.8, 1.3), x, y, 0.0, washer, None);
      let turned = Mat4::from_rotation_z(turn);
      put(parts::screw_head(4.6), x, y, 0.5, chrome, None);
      let cross = parts::screw_cross(4.6).transformed(turned);
      put(cross, x, y, 0.5, hole, None);
    }

    // each group of knobs has its name over it, between two rules that turn down at their
    // far ends
    for (name, from, to) in [("SHIFT", 42.0, 300.0), ("OUTPUT", 324.0, 424.0)] {
      let at = (from + to) / 2.0;
      put(parts::legend(name, 5.5, 1.0), at, HEADING_Y, 0.0, ink, None);
      let clear = parts::legend_width(name.len(), 5.5) / 2.0 + 6.0;
      for (near, far) in [(at - clear, from), (at + clear, to)] {
        let rule = Mesh::cuboid(
          Vec3::new(near.min(far), HEADING_Y - 0.4, 0.0),
          Vec3::new(near.max(far), HEADING_Y + 0.4, 0.25),
        );
        put(rule, 0.0, 0.0, 0.0, ink, None);
        let drop = Mesh::cuboid(
          Vec3::new(far - 0.4, HEADING_Y - 0.4, 0.0),
          Vec3::new(far + 0.4, HEADING_Y + 6.0, 0.25),
        );
        put(drop, 0.0, 0.0, 0.0, ink, None);
      }
    }
    // slots that let the air out, beside the last knob
    for slot in 0..5 {
      let x = WIDTH - EAR - 28.0 + slot as f32 * 5.0;
      put(
        parts::slot(2.6, 76.0),
        x + 0.4,
        KNOB_Y + 0.4,
        0.0,
        lip,
        None,
      );
      put(parts::slot(2.6, 76.0), x, KNOB_Y, 0.05, hole, None);
    }

    for control in Control::ALL {
      let (x, radius) = (control.x(), control.knob().radius);
      // each knob stands in a polished ring, with a scale of marks around it and figures
      // outside them
      put(
        parts::bezel(radius + 0.9, radius + 8.5, 3.4),
        x,
        KNOB_Y,
        0.0,
        chrome,
        None,
      );
      let (marks, long_every) = control.marks();
      put(
        parts::ticks(
          marks,
          parts::KNOB_SWEEP,
          radius + 11.5,
          radius + 14.5,
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
        let out = radius + 25.0;
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

    // a black strip between the ears along the bottom. the plugin's name is on it in
    // slanted letters over a red stroke, then what it is, a maker's plate riveted on, and
    // the lamp in a polished ring
    put(
      parts::slab(WIDTH - 2.0 * EAR - 8.0, 34.0, STRIP_UP, 4.0),
      middle,
      STRIP_Y,
      0.0,
      Material::glossy(black),
      None,
    );
    let name = slanted(parts::legend("CANWETALK", 13.0, 2.6), 0.2);
    put(name, 106.0, STRIP_Y - 1.5, STRIP_UP, ink, None);
    let stroke = Mesh::cuboid(Vec3::new(-58.0, -0.7, 0.0), Vec3::new(58.0, 0.7, 0.25));
    put(stroke, 103.5, STRIP_Y + 10.0, STRIP_UP, red, None);
    for (line, y, tall) in [
      ("PROFESSIONAL", -4.5, 4.4),
      ("PITCH + FORMANT SHIFTER", 4.0, 5.0),
    ] {
      let weight = tall * 0.19;
      put(
        parts::legend(line, tall, weight),
        230.0,
        STRIP_Y + y,
        STRIP_UP,
        faint,
        None,
      );
    }
    let plate = Vec3::new(338.0, STRIP_Y, STRIP_UP);
    put(
      parts::slab(76.0, 20.0, 0.9, 1.5),
      plate.x,
      plate.y,
      plate.z,
      Material::brushed(Vec3::splat(0.72)),
      None,
    );
    for (line, y) in [("MODEL CWT-3", -4.2), ("SERIAL NO 004217", 4.2)] {
      let stamped = Material::matte(Vec3::splat(0.06));
      put(
        parts::legend(line, 3.8, 0.75),
        plate.x,
        plate.y + y,
        plate.z + 0.9,
        stamped,
        None,
      );
    }
    for side in [-1.0, 1.0] {
      put(
        parts::screw_head(1.3),
        plate.x + side * 34.0,
        plate.y,
        plate.z + 0.9,
        chrome,
        None,
      );
    }
    put(
      parts::legend("EFFECT", 4.2, 0.8),
      LAMP.x - 30.0,
      STRIP_Y,
      STRIP_UP,
      faint,
      None,
    );
    put(
      parts::led_seat(LAMP_SIZE),
      LAMP.x,
      LAMP.y,
      STRIP_UP,
      hole,
      None,
    );
    put(
      parts::bezel(LAMP_SIZE * 0.56 + 0.4, LAMP_SIZE * 0.56 + 3.2, 2.0),
      LAMP.x,
      LAMP.y,
      STRIP_UP,
      chrome,
      None,
    );

    let knobs = Control::ALL.map(|control| {
      let knob = control.knob();
      [knob.body(), knob.cap(0.74), knob.line()].map(Arc::new)
    });
    Self {
      renderer: Renderer::new(gpu.device, gpu.format),
      scene: None,
      targets: Vec::new(),
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

  /// the scene of the panel with each control at its value, in the order of
  /// [`Control::ALL`], and whatever is being typed, with what each of its objects is
  fn scene(&mut self, controls: &[Reading], edit: Option<&Edit>) -> (Scene, Vec<Option<Target>>) {
    // the scene owns this frame's objects, so the ones that never move are copied into it
    let mut drawn = self.fixed.clone();
    for (index, control) in Control::ALL.into_iter().enumerate() {
      let turn = controls[index].value;
      let transform = parts::knob_transform(control.x(), KNOB_Y, turn, parts::KNOB_SWEEP);
      let materials = [
        Material {
          reflect: 0.9,
          ..Material::glossy(Vec3::new(0.06, 0.06, 0.065))
        },
        Material {
          specular: 1.0,
          ..Material::spun(Vec3::new(0.5, 0.51, 0.54))
        },
        Material::matte(Vec3::splat(0.92)),
      ];
      for (mesh, material) in self.knobs[index].iter().zip(materials) {
        drawn.place(mesh, transform, material, Some(Target::Control(index)));
      }
      let edit = edit.filter(|edit| edit.control == index);
      let Reading { text, unit, .. } = &controls[index];
      self.readout(&mut drawn, index, control, (text, unit), edit);
    }
    // the lamp is as bright as the mix is wet, and dark when nothing of the shift is heard
    let wet = controls[Control::Mix as usize].value;
    drawn.put(
      &self.lamp,
      LAMP.extend(STRIP_UP),
      parts::led_material(LAMP_COLOR, wet, LAMP_SIZE),
      None,
    );
    let scene = Scene {
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
        vignette: 0.4,
        ..Look::default()
      },
      objects: drawn.objects,
      lamps: vec![Lamp {
        at: LAMP.extend(STRIP_UP + LAMP_SIZE),
        // strong enough to show on the black strip, which gives little of any light back
        color: LAMP_COLOR * wet * 2.5,
        reach: LAMP_SIZE * 4.0,
      }],
    };
    (scene, drawn.targets)
  }

  /// a control's window: dark glass with its value and unit lit on it. while the value is
  /// typed the glass is lit too and shows what has been typed, with a caret in it or, for
  /// as long as all of it is selected, dark on a bright block
  fn readout(
    &mut self,
    drawn: &mut Drawn,
    index: usize,
    control: Control,
    (figures, unit): (&str, &str),
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

impl Skin for Panel {
  fn prepare(&mut self, gpu: &Gpu, view: &View, _over: &mut Flat) {
    let (scene, targets) = self.scene(view.controls, view.edit);
    (self.renderer).prepare(gpu.device, gpu.queue, &scene, view.pixels);
    (self.scene, self.targets) = (Some(scene), targets);
  }

  fn draw(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, place: [u32; 4]) {
    let [x, y, width, height] = place.map(|side| side as f32);
    let on = Place {
      x,
      y,
      width,
      height,
    };
    self.renderer.render(encoder, target, on, place);
  }

  fn target_at(&self, at: [f32; 2]) -> Option<Target> {
    let object = self.scene.as_ref()?.pick(at.into())?;
    self.targets[object]
  }
}

#[cfg(test)]
mod tests {
  use super::*;

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
