//! what the panel is made of and where everything on it goes

use std::ops::Range;
use std::sync::Arc;

use controls::flat::Flat;
use controls::interaction::{Edit, Target};
use controls_window::{Gpu, Skin, View, wgpu};
use nice_plug::prelude::Param;
use skeuo::glam::{Mat4, Vec2, Vec3};
use skeuo::parts;
use skeuo::{Lamp, Light, Look, Material, Mesh, Object, Place, Renderer, Scene};

use super::Control;
use crate::params::CanWeTalkParams;

pub(super) const WIDTH: f32 = 500.0;
pub(super) const HEIGHT: f32 = 250.0;
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
/// whether a readout has a letter for something typed into it
pub(super) fn readout_shows(letter: char) -> bool {
  letter == ' ' || parts::lettered(letter)
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

/// the panel as skeuo draws it: its meshes, each made once, and the scene of the frame last
/// laid out
pub(super) struct Panel {
  params: Arc<CanWeTalkParams>,
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
  pub(super) fn new(gpu: &Gpu, params: Arc<CanWeTalkParams>) -> Self {
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
      params,
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
  fn scene(&mut self, values: &[f32], edit: Option<&Edit>) -> (Scene, Vec<Option<Target>>) {
    let params = Arc::clone(&self.params);
    // the scene owns this frame's objects, so the ones that never move are copied into it
    let mut drawn = self.fixed.clone();
    for (index, control) in Control::ALL.into_iter().enumerate() {
      let param = control.param(&params);
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
        drawn.place(mesh, transform, material, Some(Target::Control(index)));
      }
      let edit = edit.filter(|edit| edit.control == index);
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
        ..Look::default()
      },
      objects: drawn.objects,
      lamps: vec![Lamp {
        at: LAMP.extend(STRIP_UP + LAMP_SIZE),
        color: LAMP_COLOR * wet,
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

impl Skin for Panel {
  fn prepare(&mut self, gpu: &Gpu, view: &View, _over: &mut Flat) {
    let (scene, targets) = self.scene(view.values, view.edit);
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
