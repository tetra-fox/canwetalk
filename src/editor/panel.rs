//! what the panel is made of and where everything on it goes

use std::f32::consts::{FRAC_PI_2, PI};
use std::ops::Range;
use std::sync::Arc;

use potpot::glam::{Mat4, Vec2, Vec3};
use potpot::parts::{self, LedRing, PushSwitch, Screen};
use potpot::{Lamp, Light, Look, Material, Mesh, Object, Place, Renderer, Scene};
use potpot_controls::axis::Axis;
use potpot_controls::flat::{Color, Flat};
use potpot_controls::interaction::{Edit, Push, Reading, Target};
use potpot_controls::stroke;
use potpot_window::{Gpu, Skin, View, wgpu};

use super::{Control, Kind, note};
use crate::params::SHIFT_SEMITONES;
use crate::spectrum::{BANDS, FLOOR_DB, FOLLOW_SLOT, HERTZ, PITCH_SLOT};

pub(super) const WIDTH: f32 = 920.0;
pub(super) const HEIGHT: f32 = 500.0;
/// how far a point slides across the picture for each unit of its height: seen a little
/// from in front
const LEAN: Vec2 = Vec2::new(0.0, -0.3);
/// how wide the rack ear at each end of the plate is
const EAR: f32 = 32.0;
/// the ends of the room the controls have across the panel, clear of the ears
const LEFT: f32 = EAR + 14.0;
const RIGHT: f32 = WIDTH - EAR - 14.0;
/// the windows the controls' values show in
const WINDOW: Vec2 = Vec2::new(66.0, 17.0);
/// how high the glass of a window is, which what it shows is lit on
const GLASS_UP: f32 = 0.5;
/// the figures a window shows its value in: how tall, and how wide their strokes. a unit
/// that is a word comes after them in smaller letters standing on the same line, and one
/// that is a sign as tall as they are, both in finer strokes
const READOUT_TALL: f32 = 7.0;
const READOUT_WEIGHT: f32 = 1.35;
const UNIT_TALL: f32 = 5.0;
const UNIT_WEIGHT: f32 = 1.0;
/// how many letters as tall as the figures fit in a window with two points clear at each end
pub(super) const READOUT_LETTERS: usize = {
  let mut letters = 0;
  while stroke::width(letters + 1, READOUT_TALL) <= WINDOW.x - 4.0 {
    letters += 1;
  }
  letters
};
const READOUT_COLOR: Vec3 = Vec3::new(1.0, 0.66, 0.14);
/// the strip along the bottom that carries the plugin's name
const STRIP_Y: f32 = HEIGHT - 29.0;
const STRIP_UP: f32 = 1.5;
/// the lamp at the end of the strip, and how wide it is
const LAMP: Vec2 = Vec2::new(WIDTH - 54.0, STRIP_Y);
const LAMP_SIZE: f32 = 11.0;
const LAMP_COLOR: Vec3 = Vec3::new(1.0, 0.45, 0.08);
/// the screen at the top right, by the middle and the size of its face
const SCREEN_AT: Vec2 = Vec2::new(RIGHT - 7.0 - SCREEN_SIZE.x / 2.0, 104.0);
const SCREEN_SIZE: Vec2 = Vec2::new(352.0, 136.0);
/// the room on the screen the spectrum is drawn in, from its top left corner to its bottom
/// right, and the middle of the column beside it that the note is written in
const PLOT: [Vec2; 2] = [
  Vec2::new(
    SCREEN_AT.x - SCREEN_SIZE.x / 2.0 + 8.0,
    SCREEN_AT.y - SCREEN_SIZE.y / 2.0 + 8.0,
  ),
  Vec2::new(
    SCREEN_AT.x + SCREEN_SIZE.x / 2.0 - 84.0,
    SCREEN_AT.y + SCREEN_SIZE.y / 2.0 - 14.0,
  ),
];
const NOTE_X: f32 = SCREEN_AT.x + SCREEN_SIZE.x / 2.0 - 40.0;
/// where across the panel the line is that sets the note's column apart
const NOTE_LINE: f32 = PLOT[1].x + 6.0;
/// how tall the note's name is written
const NOTE_TALL: f32 = 18.0;
/// the levels the spectrum is drawn over, in decibels, and the ones a line is drawn across at
const LEVELS: Axis = Axis::linear([FLOOR_DB, 0.0]);
const LEVEL_LINES: [f32; 4] = [-18.0, -36.0, -54.0, -72.0];
/// the frequencies a line is drawn up at, those named under them
const HERTZ_LINES: [f32; 8] = [50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10_000.0];
const HERTZ_NAMES: [(f32, &str); 3] = [(100.0, "100"), (1000.0, "1K"), (10_000.0, "10K")];
/// the colors on the screen: its face, its lines, their names, the spectrum's fill under its
/// edge, the edge and the detected pitch
const FACE: Vec3 = Vec3::new(0.014, 0.011, 0.008);
const GRID: Color = [0.12, 0.075, 0.03, 1.0];
const LABELS: Color = [0.42, 0.27, 0.08, 1.0];
const FILL: Color = [0.3, 0.16, 0.03, 1.0];
const EDGE: Color = [1.0, 0.66, 0.14, 1.0];
const PITCH: Color = [1.0, 0.92, 0.75, 1.0];

/// a row of controls across the panel, by where its parts are down it: the names of its
/// groups, the middles of its controls, the names under them and the windows their values
/// show in
#[derive(Clone, Copy, Debug)]
struct Row {
  heading: f32,
  middle: f32,
  name: f32,
  window: f32,
}

const ROWS: [Row; 3] = [
  Row {
    heading: 13.0,
    middle: 92.0,
    name: 150.0,
    window: 174.0,
  },
  Row {
    heading: 206.0,
    middle: 245.0,
    name: 286.0,
    window: 305.0,
  },
  Row {
    heading: 335.0,
    middle: 374.0,
    name: 415.0,
    window: 434.0,
  },
];

/// the groups of controls in each row, left to right, each under its name. the one group of
/// the first row keeps to the left, beside the screen, and the groups of the others are
/// spread from end to end
const GROUPS: [&[(&str, &[Control])]; 3] = [
  &[(
    "SHIFT",
    &[
      Control::Pitch,
      Control::Formant,
      Control::FormantFollow,
      Control::Smooth,
    ],
  )],
  &[
    (
      "EFFECTS",
      &[Control::Detune, Control::Smear, Control::Stereo],
    ),
    (
      "VIBRATO",
      &[
        Control::VibratoDepth,
        Control::VibratoRate,
        Control::VibratoSync,
        Control::VibratoDivision,
      ],
    ),
    (
      "NOTES",
      &[
        Control::Mode,
        Control::Latch,
        Control::Glide,
        Control::GlideNotesOnly,
      ],
    ),
  ],
  &[
    (
      "ENVELOPE",
      &[
        Control::Attack,
        Control::Decay,
        Control::Sustain,
        Control::Release,
      ],
    ),
    (
      "EXPRESSION",
      &[
        Control::PressureLevel,
        Control::SlideFormant,
        Control::BendRange,
      ],
    ),
    ("DETECTION", &[Control::Detect, Control::Anchor]),
    ("OUTPUT", &[Control::Mix]),
  ],
];

impl Kind {
  /// how wide a part of this kind is laid out, with the room it keeps clear around it
  fn slot(self) -> f32 {
    match self {
      Self::Shift => 140.0,
      Self::Knob | Self::Selector { .. } => 76.0,
      Self::Switch => 60.0,
    }
  }

  /// the knob a control of this kind is turned by, if it has one. the shifts' are skirted and
  /// large, the others' small, and a selector's points
  fn knob(self) -> Option<parts::Knob> {
    match self {
      Self::Shift => Some(parts::Knob {
        skirt: 0.2,
        ..parts::Knob::skirted(36.0, 30.0)
      }),
      Self::Knob => Some(parts::Knob {
        top: parts::Top::Flat,
        ..parts::Knob::knurled(16.0, 18.0)
      }),
      Self::Selector { .. } => Some(parts::Knob::chicken_head(15.0, 16.0)),
      Self::Switch => None,
    }
  }

  /// how tall the name under a control of this kind is, and how wide its strokes
  fn name_size(self) -> (f32, f32) {
    match self {
      Self::Shift => (8.0, 1.7),
      Self::Knob | Self::Selector { .. } => (6.0, 1.25),
      Self::Switch => (5.5, 1.15),
    }
  }
}

impl Control {
  /// what is printed under the control
  fn name(self) -> &'static str {
    match self {
      Self::Pitch => "PITCH",
      Self::Formant => "FORMANT",
      Self::FormantFollow => "F. FOLLOW",
      Self::Smooth => "SMOOTH",
      Self::Detune => "DETUNE",
      Self::Smear => "SMEAR",
      Self::Stereo => "STEREO",
      Self::VibratoDepth => "DEPTH",
      Self::VibratoRate => "RATE",
      Self::VibratoSync => "SYNC",
      Self::VibratoDivision => "DIVISION",
      Self::Mode => "MIDI",
      Self::Latch => "LATCH",
      Self::Glide => "GLIDE",
      Self::GlideNotesOnly => "NOTES ONLY",
      Self::Attack => "ATTACK",
      Self::Decay => "DECAY",
      Self::Sustain => "SUSTAIN",
      Self::Release => "RELEASE",
      Self::PressureLevel => "PRESSURE",
      Self::SlideFormant => "SLIDE",
      Self::BendRange => "BEND",
      Self::Detect => "RANGE",
      Self::Anchor => "ANCHOR",
      Self::Mix => "MIX",
    }
  }
}

/// where a control is: across the panel, and its row
#[derive(Clone, Copy, Debug)]
struct Spot {
  x: f32,
  row: Row,
}

impl Spot {
  fn middle(self) -> Vec2 {
    Vec2::new(self.x, self.row.middle)
  }

  fn window(self) -> Vec3 {
    Vec3::new(self.x, self.row.window, 0.0)
  }
}

/// the name of a group of controls over them, between rules that reach `ends` across the
/// panel
struct Heading {
  name: &'static str,
  y: f32,
  ends: [f32; 2],
}

/// where each control is, in the order of [`Control::ALL`], and the names of the groups
fn layout() -> ([Spot; Control::ALL.len()], Vec<Heading>) {
  let mut spots = [None; Control::ALL.len()];
  let mut headings = Vec::new();
  let wide = |controls: &[Control]| -> f32 { controls.iter().map(|c| c.kind().slot()).sum() };
  for (row, groups) in ROWS.into_iter().zip(GROUPS) {
    let taken: f32 = groups.iter().map(|(_, controls)| wide(controls)).sum();
    let gap = match groups.len() {
      1 => 0.0,
      count => (RIGHT - LEFT - taken) / (count - 1) as f32,
    };
    let mut x = LEFT;
    for &(name, controls) in groups {
      let ends = [x + 6.0, x + wide(controls) - 6.0];
      headings.push(Heading {
        name,
        y: row.heading,
        ends,
      });
      for &control in controls {
        let slot = control.kind().slot();
        spots[control as usize] = Some(Spot {
          x: x + slot / 2.0,
          row,
        });
        x += slot;
      }
      x += gap;
    }
  }
  (
    spots.map(|spot| spot.expect("every control is in a group")),
    headings,
  )
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

  /// adds objects no press is on
  fn extend(&mut self, objects: impl IntoIterator<Item = Object>) {
    self.objects.extend(objects);
    self.targets.resize(self.objects.len(), None);
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
    let wide = stroke::width(count, READOUT_TALL).max(0.0);
    // a unit written after a space is a word, which stands off from the figures
    let name = unit.trim_start();
    let (gap, unit_tall) = if name.len() < unit.len() {
      (4.0, UNIT_TALL)
    } else {
      (2.2, READOUT_TALL)
    };
    let unit_wide = stroke::width(name.chars().count(), unit_tall);
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
        let from = stroke::gap(count, selected.start, READOUT_TALL);
        let to = stroke::gap(count, selected.end, READOUT_TALL);
        let half = Vec3::new((to - from) / 2.0, READOUT_TALL / 2.0 + 2.0, 0.0);
        let block = Mesh::cuboid(-half, half + Vec3::Z * 0.15);
        (lettered(&only(true)), Arc::new(block), (from + to) / 2.0)
      }),
      of: (figures.to_owned(), unit.to_owned(), selected),
    }
  }
}

/// the meshes of the knob a control of `kind` is turned by and what each is made of: its body,
/// the face let into it unless it is a selector's, and the line painted on it
fn knob_parts(kind: Kind) -> Vec<(Arc<Mesh>, Material)> {
  let knob = kind.knob().expect("it is turned by a knob");
  let body = Material {
    reflect: 0.9,
    ..Material::glossy(Vec3::new(0.06, 0.06, 0.065))
  };
  // tight enough that its bright wedges stay apart under the light, where it glances off the
  // whole face at nearly one angle
  let spun = Material {
    specular: 1.0,
    shininess: 160.0,
    ..Material::spun(Vec3::new(0.42, 0.43, 0.46))
  };
  let line = Material::matte(Vec3::splat(0.92));
  let mut parts = vec![(Arc::new(knob.body()), body)];
  if !matches!(kind, Kind::Selector { .. }) {
    parts.push((Arc::new(knob.cap(0.74)), spun));
  }
  parts.push((Arc::new(knob.line()), line));
  parts
}

/// what the screen shows of a feed, kept for as long as the feed holds the same numbers
struct Shown {
  feed: Vec<f32>,
  objects: Vec<Object>,
}

/// everything the panel is drawn from, apart from the renderer that draws it
struct Front {
  spots: [Spot; Control::ALL.len()],
  /// everything that never moves or changes
  fixed: Drawn,
  /// the knobs of the shifts, of the other knobs and of the selectors
  knobs: [Vec<(Arc<Mesh>, Material)>; 3],
  switch: PushSwitch,
  plunger: Arc<Mesh>,
  /// one light of the ring around the formant's knob
  ring_light: Arc<Mesh>,
  lamp: Arc<Mesh>,
  glass: Arc<Mesh>,
  /// the lettering of what each window shows, kept for as long as it shows that
  readouts: Vec<Option<Lettering>>,
  caret: Arc<Mesh>,
  screen: Screen,
  /// how many objects the screen's lines and their names are, which what it shows lies over
  grid: usize,
  shown: Option<Shown>,
}

impl Front {
  fn new() -> Self {
    let (spots, headings) = layout();
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
      let outline = [[0.0, 0.0], [0.0, 2.0], [1.4, 2.0], [4.4, 0.0]].map(Vec2::from);
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

    // each group of controls has its name over it, between two rules that turn down at their
    // far ends, and so does the screen
    let screen = Screen::new(SCREEN_SIZE.x, SCREEN_SIZE.y);
    let over_screen = SCREEN_SIZE.x / 2.0 + screen.border - 6.0;
    let screen_heading = Heading {
      name: "INPUT",
      y: ROWS[0].heading,
      ends: [SCREEN_AT.x - over_screen, SCREEN_AT.x + over_screen],
    };
    for Heading { name, y, ends } in headings.into_iter().chain([screen_heading]) {
      let [from, to] = ends;
      let at = (from + to) / 2.0;
      put(parts::legend(name, 5.5, 1.0), at, y, 0.0, ink, None);
      let clear = stroke::width(name.len(), 5.5) / 2.0 + 6.0;
      for (near, far) in [(at - clear, from), (at + clear, to)] {
        let rule = Mesh::cuboid(
          Vec3::new(near.min(far), y - 0.4, 0.0),
          Vec3::new(near.max(far), y + 0.4, 0.25),
        );
        put(rule, 0.0, 0.0, 0.0, ink, None);
        let drop = Mesh::cuboid(
          Vec3::new(far - 0.4, y - 0.4, 0.0),
          Vec3::new(far + 0.4, y + 6.0, 0.25),
        );
        put(drop, 0.0, 0.0, 0.0, ink, None);
      }
    }

    let switch = PushSwitch::square(22.0, 12.0);
    for (index, (control, spot)) in Control::ALL.into_iter().zip(spots).enumerate() {
      let kind = control.kind();
      let Vec2 { x, y } = spot.middle();
      match kind {
        Kind::Shift | Kind::Knob => {
          let radius = kind.knob().expect("it is turned by a knob").radius;
          // each knob stands in a polished ring, with a scale of marks around it
          let (ring, marks, long_every, ticks) = match kind {
            Kind::Shift => (7.6, 25, 6, [11.5, 14.5]),
            _ => (4.6, 11, 5, [7.5, 10.5]),
          };
          // the formant's ring of lights stands on a low black ring where the others' polished
          // one does
          let (bezel, up) = match control {
            Control::Formant => (hole, 0.3),
            _ => (chrome, ring * 0.45),
          };
          let bezel_mesh = parts::bezel(radius + 0.9, radius + ring, up);
          put(bezel_mesh, x, y, 0.0, bezel, None);
          let [inner, outer] = ticks.map(|out| radius + out);
          let scale = parts::ticks(marks, parts::KNOB_SWEEP, inner, outer, long_every);
          put(scale, x, y, 0.0, ink, None);
          // the shifts have figures outside their marks, spread evenly from one end of the turn
          // to the other
          if kind == Kind::Shift {
            let figures = ["-24", "-12", "0", "+12", "+24"];
            for (place, figure) in figures.iter().enumerate() {
              let turn = place as f32 / (figures.len() - 1) as f32;
              let (sin, cos) = ((turn - 0.5) * parts::KNOB_SWEEP).sin_cos();
              let out = radius + 25.0;
              put(
                parts::legend(figure, 6.5, 0.95),
                x + out * sin,
                y - out * cos,
                0.0,
                ink,
                None,
              );
            }
          }
        }
        Kind::Selector { choices } => {
          // a long mark at each choice, past the pointer's tip
          let radius = kind.knob().expect("it is turned by a knob").radius;
          let marks = parts::ticks(
            choices as u32,
            parts::KNOB_SWEEP,
            radius + 6.0,
            radius + 11.0,
            1,
          );
          put(marks, x, y, 0.0, ink, None);
        }
        Kind::Switch => {
          put(switch.plunger.seat(1.2), x, y, 0.0, hole, None);
          put(switch.collar(), x, y, 0.0, chrome, None);
        }
      }
      let (tall, weight) = kind.name_size();
      let name = parts::legend(control.name(), tall, weight);
      put(name, x, spot.row.name, 0.0, ink, None);
      // the bezel of the window its value shows in. the glass set down in it changes with
      // what it shows
      if kind != Kind::Switch {
        put(
          parts::frame(WINDOW.x + 6.0, WINDOW.y + 6.0, 3.0, 2.2, 3.5),
          x,
          spot.row.window,
          0.0,
          Material::glossy(black),
          Some(Target::Readout(index)),
        );
      }
    }

    // the screen in its bezel, with lines across and up it that the spectrum is drawn over
    let glass = screen_glass();
    let Vec2 { x, y } = SCREEN_AT;
    put(screen.face(), x, y, 0.0, glass, None);
    put(screen.bezel(), x, y, 0.0, Material::glossy(black), None);
    let grid = parts::picture(&lines(), x, y, glass, 0);
    let grid_objects = grid.len();
    fixed.extend(grid);
    let ring = formant_ring();

    // a black strip between the ears along the bottom. the plugin's name is on it in
    // slanted letters over a red stroke, then what it is, a maker's plate riveted on, and
    // the lamp in a polished ring
    let mut put = |mesh: Mesh, x: f32, y: f32, z: f32, material: Material, target| {
      fixed.put(&Arc::new(mesh), Vec3::new(x, y, z), material, target);
    };
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
    let plate = Vec3::new(WIDTH - 162.0, STRIP_Y, STRIP_UP);
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

    Self {
      spots,
      fixed,
      // made for one control of each of those kinds
      knobs: [Control::Pitch, Control::Mix, Control::Detect].map(|one| knob_parts(one.kind())),
      plunger: Arc::new(switch.plunger()),
      ring_light: Arc::new(ring.segment()),
      switch,
      lamp: Arc::new(parts::led(LAMP_SIZE)),
      glass: Arc::new(Mesh::cuboid(
        (-WINDOW / 2.0).extend(0.0),
        (WINDOW / 2.0).extend(GLASS_UP),
      )),
      readouts: Control::ALL.map(|_| None).into(),
      caret: Arc::new(Mesh::cuboid(
        Vec3::new(-0.6, -READOUT_TALL / 2.0 - 2.0, 0.0),
        Vec3::new(0.6, READOUT_TALL / 2.0 + 2.0, 0.3),
      )),
      screen,
      grid: grid_objects,
      shown: None,
    }
  }

  /// the scene of the panel with each control at its value, in the order of
  /// [`Control::ALL`], and whatever is being typed, with what each of its objects is.
  /// `held` is the control held down, and `feed` is what the audio thread last wrote
  fn scene(
    &mut self,
    controls: &[Reading],
    edit: Option<&Edit>,
    held: Option<usize>,
    feed: &[f32],
  ) -> (Scene, Vec<Option<Target>>) {
    // the scene owns this frame's objects, so the ones that never move are copied into it
    let mut drawn = self.fixed.clone();
    let mut lamps = Vec::new();
    let rubber = Material::silicone(Vec3::splat(0.1));
    for (index, (control, spot)) in Control::ALL.into_iter().zip(self.spots).enumerate() {
      let Vec2 { x, y } = spot.middle();
      let reading = &controls[index];
      let target = Some(Target::Control(index));
      let knob = match control.kind() {
        Kind::Shift => &self.knobs[0],
        Kind::Knob => &self.knobs[1],
        Kind::Selector { .. } => &self.knobs[2],
        Kind::Switch => {
          // a switch that is on stays in and is lit, and throws its light around it
          let on = reading.value > 0.5;
          let depth = Push::Latching.depth(on, held == Some(index));
          let plunger = self.switch.plunger;
          let level = f32::from(u8::from(on));
          drawn.place(
            &self.plunger,
            self.switch.transform(x, y, depth),
            rubber.lit(READOUT_COLOR, level, plunger.glow_radius()),
            target,
          );
          if on {
            lamps.push(plunger.lamp(x, y, READOUT_COLOR, 0.5));
          }
          continue;
        }
      };
      let transform = parts::knob_transform(x, y, reading.value, parts::KNOB_SWEEP);
      for (mesh, material) in knob {
        drawn.place(mesh, transform, *material, target);
      }
      if control == Control::Formant {
        // lit from where the knob points on by what formant follow adds to the formants of
        // the voice that started last. a run past either end of the scale stops there, which
        // is as far as the ring shows, though the shifter moves the formants further
        let followed = feed[FOLLOW_SLOT] / (2.0 * SHIFT_SEMITONES);
        let run = [reading.value, reading.value + followed];
        let ring = formant_ring();
        let lights = ring.lights(&self.ring_light, x, y, run, LAMP_COLOR);
        // stood on the black ring
        drawn.extend(lights.map(|light| Object {
          transform: Mat4::from_translation(Vec3::Z * 0.3) * light.transform,
          ..light
        }));
      }
      let edit = edit.filter(|edit| edit.control == index);
      let Reading { text, unit, .. } = reading;
      self.readout(&mut drawn, index, spot.window(), (text, unit), edit);
    }

    let Vec2 { x, y } = SCREEN_AT;
    let glass = screen_glass();
    let shown = (self.shown.take())
      .filter(|shown| shown.feed == feed)
      .unwrap_or_else(|| Shown {
        feed: feed.to_vec(),
        objects: parts::picture(&spectrum(feed), x, y, glass, self.grid),
      });
    drawn.extend(shown.objects.iter().cloned());
    self.shown = Some(shown);
    lamps.extend(self.screen.lamps(x, y, READOUT_COLOR, 0.35));

    // the lamp is as bright as the mix is wet, and dark when nothing of the shift is heard
    let wet = controls[Control::Mix as usize].value;
    drawn.put(
      &self.lamp,
      LAMP.extend(STRIP_UP),
      parts::led_material(LAMP_COLOR, wet, LAMP_SIZE),
      None,
    );
    lamps.push(Lamp {
      at: LAMP.extend(STRIP_UP + LAMP_SIZE),
      // strong enough to show on the black strip, which gives little of any light back
      color: LAMP_COLOR * wet * 2.5,
      reach: LAMP_SIZE * 4.0,
      radius: LAMP_SIZE / 2.0,
    });
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
      lamps,
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
    at: Vec3,
    (figures, unit): (&str, &str),
    edit: Option<&Edit>,
  ) {
    let target = Some(Target::Readout(index));
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
      let across = stroke::gap(edit.text.chars().count(), edit.caret, READOUT_TALL);
      drawn.put(&self.caret, figures_at + Vec3::X * across, lit, target);
    }
    self.readouts[index] = Some(lettering);
  }
}

/// the lights around the formant's knob, one between each two of its marks
fn formant_ring() -> LedRing {
  let radius = Kind::Shift.knob().expect("it is turned by a knob").radius;
  LedRing::new(24, radius + 4.3, 4.0)
}

/// the face of the screen, which what it shows is lit on
fn screen_glass() -> Material {
  Material::screen(FACE, (SCREEN_SIZE.x + SCREEN_SIZE.y) * 3.0)
}

/// the point of the panel at a place in the plot: a part of the way across it and a part of
/// the way up
fn plotted([across, up]: [f32; 2]) -> [f32; 2] {
  let [min, max] = PLOT;
  [
    min.x + across * (max.x - min.x),
    max.y - up * (max.y - min.y),
  ]
}

/// the lines the screen's spectrum is drawn over, the names of the frequencies under them, and
/// the line that sets the note's column apart
fn lines() -> Flat {
  let mut flat = Flat::default();
  for hz in HERTZ_LINES {
    let across = HERTZ.turn(hz);
    flat.line(plotted([across, 0.0]), plotted([across, 1.0]), 1.0, GRID);
  }
  for level in LEVEL_LINES {
    let up = LEVELS.turn(level);
    flat.line(plotted([0.0, up]), plotted([1.0, up]), 1.0, GRID);
  }
  for (hz, name) in HERTZ_NAMES {
    let [x, y] = plotted([HERTZ.turn(hz), 0.0]);
    flat.centered(name, [x, y + 7.0], 5.0, 1.0, LABELS);
  }
  let half = SCREEN_SIZE.y / 2.0 - 10.0;
  let top = [NOTE_LINE, SCREEN_AT.y - half];
  flat.line(top, [NOTE_LINE, SCREEN_AT.y + half], 1.0, GRID);
  flat
}

/// what the screen shows of the feed: the level of each band of the spectrum as a curve over
/// a fill, and beside it the note nearest the detected pitch, how many cents off it the pitch
/// is and the pitch itself. the pitch is marked on the spectrum too. with none detected the
/// note is two dashes
fn spectrum(feed: &[f32]) -> Flat {
  let mut flat = Flat::default();
  let curve: Vec<[f32; 2]> = (feed[..BANDS].iter().enumerate())
    .map(|(band, level)| {
      let across = (band as f32 + 0.5) / BANDS as f32;
      plotted([across, LEVELS.turn(*level)])
    })
    .collect();
  flat.area(&curve, PLOT[1].y, FILL);
  flat.curve(&curve, 1.5, EDGE);
  let hz = feed[PITCH_SLOT];
  if hz > 0.0 {
    let across = HERTZ.turn(hz);
    flat.line(plotted([across, 0.0]), plotted([across, 1.0]), 1.0, PITCH);
    let (name, cents) = note::nearest(hz);
    flat.centered(&name, [NOTE_X, SCREEN_AT.y - 24.0], NOTE_TALL, 2.6, EDGE);
    let cents = format!("{cents:+} CT");
    flat.centered(&cents, [NOTE_X, SCREEN_AT.y + 8.0], 7.0, 1.3, EDGE);
    let hertz = format!("{hz:.1} HZ");
    flat.centered(&hertz, [NOTE_X, SCREEN_AT.y + 24.0], 5.5, 1.1, LABELS);
  } else {
    flat.centered("--", [NOTE_X, SCREEN_AT.y - 24.0], NOTE_TALL, 2.6, LABELS);
  }
  flat
}

/// the panel as potpot draws it, and what each object of the scene last laid out is
pub struct Panel {
  renderer: Renderer,
  /// what each object of the scene last laid out is, in the scene's order
  targets: Vec<Option<Target>>,
  front: Front,
}

impl Panel {
  pub(super) fn new(gpu: &Gpu) -> Self {
    Self {
      renderer: Renderer::new(gpu.device, gpu.format),
      targets: Vec::new(),
      front: Front::new(),
    }
  }
}

impl Skin for Panel {
  fn prepare(&mut self, gpu: &Gpu, view: &View, _over: &mut Flat) {
    let (scene, targets) = (self.front).scene(view.controls, view.edit, view.held, view.feed);
    (self.renderer).prepare(gpu.device, gpu.queue, &scene, view.pixels);
    self.targets = targets;
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

  fn changed(&self) -> Option<&[[u32; 4]]> {
    self.renderer.changed()
  }

  fn target_at(&self, at: [f32; 2]) -> Option<Target> {
    let object = self.renderer.pick(at.into())?;
    self.targets[object]
  }
}

#[cfg(test)]
mod tests {
  use potpot_controls::interaction::{Control as Handled, Dial, Interaction, Readout};

  use super::*;
  use crate::spectrum::SLOTS;

  #[test]
  fn the_longest_value_fits_its_window() {
    // the widest each kind of value is written, and the most letters that are let be typed
    for (figures, unit) in [
      ("-24.00", " st"),
      ("10000", " ms"),
      ("15.00", " Hz"),
      ("200", " ct"),
      ("100", "%"),
      ("96", " st"),
      ("Mono Gate", ""),
      ("1/16T", ""),
      ("-12.5", ""),
    ] {
      let lettering = Lettering::new(figures, unit, 0..0);
      // the furthest any stroke reaches from the window's middle
      let reach = |mesh: &Mesh, at: f32| -> f32 {
        (mesh.vertices.iter())
          .map(|vertex| (vertex.position[0] + at).abs())
          .fold(0.0, f32::max)
      };
      let unit_reach = (lettering.unit.as_ref()).map_or(0.0, |(mesh, at)| reach(mesh, at.x));
      let widest = reach(&lettering.figures, lettering.figures_at).max(unit_reach);
      assert!(
        widest <= WINDOW.x / 2.0 - 1.0,
        "{figures}{unit} reaches {widest}"
      );
    }
    assert!(stroke::width(READOUT_LETTERS, READOUT_TALL) <= WINDOW.x - 4.0);
    assert!(READOUT_LETTERS >= "-12.5 st".len());
    // the widest note name, a sharp in the octave under the lowest, fits between the line
    // that sets its column apart and the edge of the screen's face
    let half = stroke::width("C#-1".len(), NOTE_TALL) / 2.0;
    let edge = SCREEN_AT.x + SCREEN_SIZE.x / 2.0;
    assert!(NOTE_X - half > NOTE_LINE && NOTE_X + half < edge, "{half}");
  }

  /// the scene with every control halfway, nothing typed and a pitch on the screen
  fn scene(front: &mut Front) -> (Scene, Vec<Option<Target>>) {
    let count = Control::ALL.len();
    let dials = (0..count)
      .map(|_| Dial::linear([0.0, 1.0], 0.5, None, 2, ""))
      .collect();
    let handled = (0..count).map(|control| Handled::new(control, Readout::new(4)));
    let panel = Interaction::new(dials, handled.collect(), [WIDTH, HEIGHT], [1.0, 1.0]);
    let mut feed = vec![FLOOR_DB; SLOTS];
    feed[PITCH_SLOT] = 220.0;
    front.scene(&panel.readings(&vec![0.5; count]), None, None, &feed)
  }

  #[test]
  fn every_control_and_window_is_picked_where_it_is_drawn() {
    let mut front = Front::new();
    let (scene, targets) = scene(&mut front);
    let picked = |at: Vec2| scene.pick(at).and_then(|object| targets[object]);
    for (index, (control, spot)) in Control::ALL.into_iter().zip(front.spots).enumerate() {
      // the middle of a knob and a little off it each way, which its top is seen leaned from
      for off in [Vec2::ZERO, Vec2::X * 6.0, -Vec2::X * 6.0, Vec2::Y * 6.0] {
        let at = spot.middle() + off;
        assert_eq!(
          picked(at),
          Some(Target::Control(index)),
          "{control:?} at {at}"
        );
      }
      if control.kind() != Kind::Switch {
        let at = spot.window().truncate();
        for off in [Vec2::ZERO, Vec2::X * (WINDOW.x / 2.0 - 2.0)] {
          assert_eq!(
            picked(at + off),
            Some(Target::Readout(index)),
            "{control:?}"
          );
        }
      }
    }
  }

  #[test]
  fn no_two_controls_take_the_same_room() {
    // each control is in one group alone, which with every one placed is once each
    let grouped: usize = (GROUPS.iter().flat_map(|row| row.iter()))
      .map(|(_, controls)| controls.len())
      .sum();
    assert_eq!(grouped, Control::ALL.len());
    let (spots, _) = layout();
    for (index, one) in spots.iter().enumerate() {
      let kind = Control::ALL[index].kind();
      for (other, two) in spots.iter().enumerate().skip(index + 1) {
        let reach = (kind.slot() + Control::ALL[other].kind().slot()) / 2.0;
        let apart = one.middle().distance(two.middle());
        assert!(apart >= reach, "{index} and {other} are {apart} apart");
      }
      // and every one is clear of the screen and the ears
      let half = kind.slot() / 2.0;
      assert!(one.x - half >= LEFT && one.x + half <= RIGHT, "{index}");
      let screen = SCREEN_AT.x - SCREEN_SIZE.x / 2.0 - 7.0;
      assert!(
        one.row.middle > ROWS[0].window || one.x + half <= screen,
        "{index}"
      );
    }
  }
}
