//! what the panel is made of and where everything on it goes

use std::f32::consts::{FRAC_PI_2, PI};
use std::ops::Range;
use std::sync::Arc;

use potpot::glam::{Mat4, Vec2, Vec3};
use potpot::parts::{self, Handle, LedArc, LedRing, LedStops, PushSwitch, Screen, Vent};
use potpot::{Light, Look, Material, Mesh, Object, Place, Renderer, Scene};
use potpot_controls::axis::Axis;
use potpot_controls::flat::{Color, Flat};
use potpot_controls::furniture::{GripLook, MenuLook};
use potpot_controls::interaction::{Edit, Push, Reading, Target};
use potpot_controls::layout::{Pack, middle, pack};
use potpot_controls::stroke;
use potpot_window::{Gpu, Skin, View, wgpu};

use super::{Control, Kind, note};
use crate::params::SHIFT_SEMITONES;
use crate::spectrum::{
  BANDS, FLOOR_DB, FOLLOW_SLOT, HERTZ, HERTZ_ENDS, INPUT_BANDS, OUTPUT_BANDS, PITCH_SLOT,
  VOICE_SLOTS,
};

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
/// the light built into the hardware
const CYAN: Vec3 = Vec3::new(0.2, 0.85, 1.0);
/// the color of the caret and the selection while a value is typed
const AMBER: Vec3 = Vec3::new(1.0, 0.66, 0.14);
/// the dark metal of the panel, and the darker metal of the strip along its bottom
const GUNMETAL: Vec3 = Vec3::new(0.12, 0.13, 0.14);
const STRIP_METAL: Vec3 = Vec3::new(0.06, 0.065, 0.07);
/// how much further apart lit letters are than the face sets them, in units of the grid a
/// letter is drawn on, and how wide their strokes are for each point of their height
const TRACKING: f32 = 1.0;
const LIT_WEIGHT: f32 = 0.13;
/// how brightly the names of the groups are lit, and the rest of the lettering on the panel
const HEADING_LIT: f32 = 1.0;
const LABEL_LIT: f32 = 0.85;
/// the windows the controls' values show in
const WINDOW: Vec2 = Vec2::new(66.0, 17.0);
/// how much wider and taller the rim around a window is than the window, and how high it
/// stands over the glass
const WINDOW_FRAME: f32 = 6.0;
const WINDOW_RIM: f32 = 1.8;
/// how high the glass of a window is, which what it shows is lit on
const GLASS_UP: f32 = 0.5;
/// the figures a window shows its value in: how tall, and how wide their strokes. a unit
/// that is a word comes after them in smaller letters standing on the same line, and one
/// that is a sign as tall as they are, both in finer strokes
const READOUT_TALL: f32 = 7.0;
const READOUT_WEIGHT: f32 = 1.2;
const UNIT_TALL: f32 = 5.0;
const UNIT_WEIGHT: f32 = 0.9;
/// how many letters as tall as the figures fit in a window with two points clear at each end
pub(super) const READOUT_LETTERS: usize = {
  let mut letters = 0;
  while stroke::width(letters + 1, READOUT_TALL) <= WINDOW.x - 4.0 {
    letters += 1;
  }
  letters
};
/// the knobs: the radius and height of a shift's and of the others'
const SHIFT_RADIUS: f32 = 27.0;
const SHIFT_HEIGHT: f32 = 24.0;
const SMALL_RADIUS: f32 = 14.0;
const SMALL_HEIGHT: f32 = 15.0;
/// how far the chrome skirt under a knob reaches past its edge, for each point of its radius
const SKIRT: f32 = 1.22;
/// how far out from a shift's middle the figures of its scale are, and how tall
const FIGURES_OUT: f32 = 53.0;
const FIGURES_TALL: f32 = 6.0;
/// how far out from a selector's middle the light at each stop is and how wide, and how far
/// out the name of each choice is printed and how tall
const PIP_OUT: f32 = 22.0;
const PIP: f32 = 4.0;
const CHOICE_OUT: f32 = 31.0;
const CHOICE_TALL: f32 = 5.0;
/// how bright a light is that is off: a selector's at a stop it is not at, and a switch's
const DARK: f32 = 0.1;
/// how bright a shift's ring is over what formant follow adds to the shift
const FOLLOW_LIT: f32 = 0.35;
/// the field each group of controls is set in: how wide its rim is, how high and the radius
/// of its corners, and the room inside it over the group's name and under its windows
const FIELD_BORDER: f32 = 3.0;
const FIELD_UP: f32 = 1.6;
const FIELD_CORNER: f32 = 6.0;
const FIELD_PAD: f32 = 6.0;
/// how tall the letters of a group's name are, how far the lines of light beside it turn down
/// at their far ends, and how far in from the ends of its field those ends are
const HEADING_TALL: f32 = 5.5;
const RULE_DROP: f32 = 6.0;
const RULE_IN: f32 = 10.0;
/// the strip along the bottom that carries the plugin's name: its middle down the panel, how
/// high it stands and how tall it is
const STRIP_Y: f32 = HEIGHT - 29.0;
const STRIP_UP: f32 = 2.0;
const STRIP_TALL: f32 = 34.0;
/// the light on the strip that shows how much of the shift is heard, and how wide it is
const EFFECT: Vec2 = Vec2::new(WIDTH - 66.0, STRIP_Y);
const EFFECT_SIZE: f32 = 7.0;
/// the screen at the top right, by the middle and the size of its face
const SCREEN_AT: Vec2 = Vec2::new(RIGHT - SCREEN_CLEAR - SCREEN_SIZE.x / 2.0, 106.0);
const SCREEN_SIZE: Vec2 = Vec2::new(352.0, 136.0);
/// the room kept clear across the panel at each side of the screen's face, in its field
const SCREEN_CLEAR: f32 = 14.0;
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
/// how tall the input's note is written, and the notes the voices make
const NOTE_TALL: f32 = 13.0;
const MADE_TALL: f32 = 10.0;
/// how many of the voices' notes are written, the latest first. the rest are counted
const MADE_NAMES: usize = 3;
/// the levels the spectrum is drawn over, in decibels, and the ones a line is drawn across at
const LEVELS: Axis = Axis::linear([FLOOR_DB, 0.0]);
const LEVEL_LINES: [f32; 4] = [-18.0, -36.0, -54.0, -72.0];
/// the frequencies a line is drawn up at, those named under them
const HERTZ_LINES: [f32; 8] = [50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10_000.0];
const HERTZ_NAMES: [(f32, &str); 3] = [(100.0, "100"), (1000.0, "1K"), (10_000.0, "10K")];
/// the colors on the screen: its smoked face, its lines, their names, the input spectrum's
/// fill under its edge, the edge and the detected pitch, then the output spectrum's edge, the
/// pitches the voices make and the names of the output's lines
const FACE: Vec3 = Vec3::new(0.008, 0.014, 0.018);
const GRID: Color = [0.035, 0.11, 0.14, 1.0];
const LABELS: Color = [0.12, 0.42, 0.52, 1.0];
const FILL: Color = [0.02, 0.12, 0.16, 1.0];
const EDGE: Color = [CYAN.x, CYAN.y, CYAN.z, 1.0];
const PITCH: Color = [0.92, 1.0, 1.0, 1.0];
const MADE: Color = [0.78, 0.96, 1.0, 1.0];
const MADE_PITCH: Color = [0.38, 0.5, 0.56, 1.0];
const MADE_LABELS: Color = [0.28, 0.4, 0.46, 1.0];

/// a row of controls across the panel, by where its parts are down it: the top and the foot
/// of the fields its groups are set in, the names of its groups, the middles of its controls,
/// the names under them and the windows their values show in
#[derive(Clone, Copy, Debug)]
struct Row {
  field: [f32; 2],
  heading: f32,
  middle: f32,
  name: f32,
  window: f32,
}

/// the room over the first row, at the top of the panel, and under the last, over the strip
const TOP_MARGIN: f32 = 10.0;
const STRIP_CLEAR: f32 = 8.0;
/// the room in a row between the drops at the ends of its rules and the tops of its controls,
/// between the controls and the names under them, and between the names and the windows
const OVER_GAP: f32 = 5.5;
const UNDER_GAP: f32 = 10.0;
const NAME_GAP: f32 = 6.0;

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

/// the knob the shifts are turned by
fn shift_knob() -> parts::Knob {
  parts::Knob {
    skirt: 0.0,
    ..parts::Knob::round(SHIFT_RADIUS, SHIFT_HEIGHT)
  }
}

/// the knob the other controls that turn are turned by
fn small_knob() -> parts::Knob {
  parts::Knob {
    skirt: 0.0,
    ..parts::Knob::round(SMALL_RADIUS, SMALL_HEIGHT)
  }
}

/// the chrome skirt a knob stands on, which does not turn with it
fn skirt(knob: parts::Knob) -> Mesh {
  parts::bezel(knob.radius * 0.8, knob.radius * SKIRT, knob.height * 0.18)
}

/// the lights around a shift's knob, which show its value either way from nothing
fn shift_ring() -> LedRing {
  LedRing::new(24, SHIFT_RADIUS * SKIRT + 6.0, 5.0)
}

/// the unbroken light around the knob of a control that turns smoothly
fn knob_arc() -> LedArc {
  LedArc::new(SMALL_RADIUS * SKIRT + 5.0, 2.4)
}

/// the lights at the stops of a selector with these choices
fn selector_stops(choices: &[&str]) -> LedStops {
  LedStops {
    dim: DARK,
    ..LedStops::new(
      u32::try_from(choices.len()).expect("a handful of choices"),
      PIP_OUT,
      PIP,
    )
  }
}

/// how far down from a knob's middle a point `out` from it is at either end of its turn
fn down_at_ends(out: f32) -> f32 {
  -out * (parts::KNOB_SWEEP / 2.0).cos()
}

impl Kind {
  /// how wide a part of this kind is laid out, with the room it keeps clear around it
  fn slot(self) -> f32 {
    match self {
      Self::Shift => 140.0,
      // room for the names printed out to its sides
      Self::Selector { printed: true, .. } => 94.0,
      Self::Knob | Self::Selector { printed: false, .. } => 76.0,
      Self::Switch => 70.0,
    }
  }

  /// how far a part of this kind reaches up the panel from its middle, and down, flat on the
  /// panel
  fn reach(self) -> [f32; 2] {
    match self {
      // over the figure at the top of its scale, and under those at the ends of its turn or
      // its skirt, whichever is lower
      Self::Shift => {
        let skirt = SHIFT_RADIUS * SKIRT;
        let ends = down_at_ends(FIGURES_OUT) + FIGURES_TALL / 2.0;
        [FIGURES_OUT + FIGURES_TALL / 2.0, ends.max(skirt)]
      }
      // over the light around it, and under its ends or the skirt
      Self::Knob => {
        let arc = knob_arc();
        let out = arc.radius + arc.width / 2.0 + arc.margin;
        [out, down_at_ends(out).max(SMALL_RADIUS * SKIRT)]
      }
      // over the name of the choice at the top, and under those at the ends of its turn
      Self::Selector { printed: true, .. } => [
        CHOICE_OUT + CHOICE_TALL / 2.0,
        down_at_ends(CHOICE_OUT) + CHOICE_TALL / 2.0,
      ],
      // over and under its lights, or under the skirt if that is lower
      Self::Selector {
        choices,
        printed: false,
      } => {
        let [up, down] = selector_stops(choices).reach();
        [up, down.max(SMALL_RADIUS * SKIRT)]
      }
      // over and under the collar
      Self::Switch => {
        let switch = switch();
        let half = switch.plunger.length / 2.0 + switch.collar;
        [half, half]
      }
    }
  }

  /// whether a control of this kind has a window its value shows in, which it is typed into
  fn has_window(self) -> bool {
    match self {
      Self::Shift | Self::Knob => true,
      Self::Selector { printed, .. } => !printed,
      Self::Switch => false,
    }
  }

  /// how tall the name under a control of this kind is
  fn name_tall(self) -> f32 {
    match self {
      Self::Shift => 7.5,
      Self::Knob | Self::Selector { .. } => 5.5,
      Self::Switch => 5.0,
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

  /// the place of its turn the light around a control is lit from: the middle for one that
  /// shifts either way, the start for the rest
  fn rest(self) -> f32 {
    match self {
      Self::Pitch | Self::Formant | Self::SlideFormant => 0.5,
      Self::FormantFollow
      | Self::Smooth
      | Self::Detune
      | Self::Smear
      | Self::Stereo
      | Self::VibratoDepth
      | Self::VibratoRate
      | Self::VibratoSync
      | Self::VibratoDivision
      | Self::Mode
      | Self::Latch
      | Self::Glide
      | Self::GlideNotesOnly
      | Self::Attack
      | Self::Decay
      | Self::Sustain
      | Self::Release
      | Self::PressureLevel
      | Self::BendRange
      | Self::Detect
      | Self::Anchor
      | Self::Mix => 0.0,
    }
  }
}

/// the push switch every switch on the panel is
fn switch() -> PushSwitch {
  PushSwitch::backlit(30.0, 18.0, 6.0)
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

/// a field set into the panel, `across` and `down` it, with a name over what is in it. `y` is
/// how far down the name is
#[derive(Clone, Debug)]
struct Field {
  name: &'static str,
  across: [f32; 2],
  down: [f32; 2],
  y: f32,
}

/// where each control is, in the order of [`Control::ALL`], and the fields the groups of
/// controls and the screen are set in. `border` is the width of the screen's bezel
fn layout(border: f32) -> ([Spot; Control::ALL.len()], Vec<Field>) {
  let mut spots = [None; Control::ALL.len()];
  let mut fields = Vec::new();
  // each row's parts down the panel, from the top of its fields to their foot
  let bands = GROUPS.map(|groups| {
    let kinds =
      || (groups.iter()).flat_map(|(_, controls)| controls.iter().map(|control| control.kind()));
    let largest = |of: fn(Kind) -> f32| kinds().map(of).fold(0.0, f32::max);
    let (name_gap, window) = if kinds().any(Kind::has_window) {
      (NAME_GAP, WINDOW.y + WINDOW_FRAME)
    } else {
      (0.0, 0.0)
    };
    [
      FIELD_PAD,
      HEADING_TALL,
      // the drop starts at the middle of the letters
      RULE_DROP - HEADING_TALL / 2.0,
      OVER_GAP,
      largest(|kind| kind.reach()[0]),
      largest(|kind| kind.reach()[1]),
      UNDER_GAP,
      largest(Kind::name_tall),
      name_gap,
      window,
      FIELD_PAD,
    ]
  });
  // the rows spread from the top margin to the strip, the first reaching at least past the
  // foot of the screen's bezel
  let mut tall = bands.map(|bands| bands.iter().sum::<f32>());
  let screen_foot = SCREEN_AT.y + SCREEN_SIZE.y / 2.0 + border + FIELD_PAD;
  tall[0] = tall[0].max(screen_foot - TOP_MARGIN);
  let down = [TOP_MARGIN, STRIP_Y - STRIP_TALL / 2.0 - STRIP_CLEAR];
  let rows: Vec<Row> = (pack(down, &tall, Pack::Spread).into_iter().zip(bands))
    .map(|(span, bands)| {
      let placed = <[_; 11]>::try_from(pack(span, &bands, Pack::Start));
      let [_, letters, _, _, over, _, _, name, _, window, _] =
        placed.expect("a place for each band");
      Row {
        field: span,
        heading: middle(letters),
        middle: over[1],
        name: middle(name),
        window: middle(window),
      }
    })
    .collect();
  let slots = |controls: &[Control]| -> Vec<f32> {
    controls
      .iter()
      .map(|control| control.kind().slot())
      .collect()
  };
  // the room each row's groups share, and how. the screen's field reaches to the right end
  let screen = [RIGHT - SCREEN_SIZE.x - 2.0 * SCREEN_CLEAR, RIGHT];
  let rooms = [
    ([LEFT, screen[0] - FIELD_PAD * 2.0], Pack::Start),
    ([LEFT, RIGHT], Pack::Spread),
    ([LEFT, RIGHT], Pack::Spread),
  ];
  for ((&row, groups), (room, how)) in rows.iter().zip(GROUPS).zip(rooms) {
    let widths: Vec<f32> = (groups.iter())
      .map(|(_, controls)| slots(controls).iter().sum())
      .collect();
    for (&(name, controls), span) in groups.iter().zip(pack(room, &widths, how)) {
      fields.push(Field {
        name,
        across: span,
        down: row.field,
        y: row.heading,
      });
      for (&control, slot) in controls
        .iter()
        .zip(pack(span, &slots(controls), Pack::Start))
      {
        spots[control as usize] = Some(Spot {
          x: middle(slot),
          row,
        });
      }
    }
  }
  fields.push(Field {
    name: "INPUT",
    across: screen,
    down: rows[0].field,
    y: rows[0].heading,
  });
  (
    spots.map(|spot| spot.expect("every control is in a group")),
    fields,
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

/// lettering lit from inside the panel, in fine strokes spread wider than the face sets them
fn lit_legend(text: &str, tall: f32) -> Mesh {
  parts::tracked_legend(text, tall, tall * LIT_WEIGHT, TRACKING)
}

/// how wide [`lit_legend`] draws `text`
fn lit_width(text: &str, tall: f32) -> f32 {
  stroke::tracked_width(text.chars().count(), tall, TRACKING)
}

/// what lit lettering and lines are made of, at `level` of their full light
fn lit(level: f32) -> Material {
  Material::lamp(CYAN * level)
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

/// the point `out` from a knob's middle where the knob points at `turn` of its sweep
fn around(middle: Vec2, turn: f32, out: f32) -> Vec2 {
  let (sin, cos) = ((turn - 0.5) * parts::KNOB_SWEEP).sin_cos();
  middle + Vec2::new(sin, -cos) * out
}

/// the middle of the name of a choice printed at `turn` of the sweep of a selector around
/// `middle`. each name is pushed to the side its stop leans to, by as much of its half width as
/// the stop leans, which keeps a name beside the knob clear of it
fn choice_at(middle: Vec2, turn: f32, name: &str) -> Vec2 {
  let side = ((turn - 0.5) * parts::KNOB_SWEEP).sin();
  let half = lit_width(name, CHOICE_TALL) / 2.0;
  around(middle, turn, CHOICE_OUT) + Vec2::X * side * half
}

/// meshes and what each is made of
type Parts = Vec<(Arc<Mesh>, Material)>;

/// the meshes of a knob that turn with it: its glossy black body and the lit line on it
fn knob_parts(knob: parts::Knob) -> Parts {
  // black plastic with a tight, faint highlight and little of the room in it, which keeps a
  // knob's top black where the near light's mirror image falls on it
  let body = Material {
    specular: 0.3,
    shininess: 600.0,
    reflect: 0.1,
    ..Material::glossy(Vec3::splat(0.008))
  };
  vec![
    (Arc::new(knob.body()), body),
    (Arc::new(knob.line()), Material::lamp(CYAN)),
  ]
}

/// objects made for what they show, kept for as long as it is the same
struct Kept<T> {
  of: T,
  objects: Vec<Object>,
}

/// everything the panel is drawn from, apart from the renderer that draws it
struct Front {
  spots: [Spot; Control::ALL.len()],
  /// everything that never moves or changes
  fixed: Drawn,
  /// the parts of a shift's knob and of the others
  shift_knob: Parts,
  small_knob: Parts,
  switch: PushSwitch,
  plunger: Arc<Mesh>,
  ring: LedRing,
  /// one light of a shift's ring
  ring_light: Arc<Mesh>,
  arc: LedArc,
  /// the dark gap an arc is set in and its lens
  arc_meshes: [Arc<Mesh>; 2],
  /// the lit arc around each control, kept for as long as it is lit over the same run
  arcs: Vec<Option<Kept<[f32; 2]>>>,
  /// the light at each stop of a selector
  stop_light: Arc<Mesh>,
  effect: Arc<Mesh>,
  glass: Arc<Mesh>,
  /// the lettering of what each window shows, kept for as long as it shows that
  readouts: Vec<Option<Lettering>>,
  caret: Arc<Mesh>,
  screen: Screen,
  /// how many objects the screen's lines and their names are, which what it shows lies over
  grid: usize,
  shown: Option<Kept<Vec<f32>>>,
}

impl Front {
  fn new() -> Self {
    let screen = Screen::new(SCREEN_SIZE.x, SCREEN_SIZE.y);
    let (spots, fields) = layout(screen.border);
    let mut fixed = Drawn::default();
    let mut put = |mesh: Mesh, at: Vec3, material: Material, target| {
      fixed.put(&Arc::new(mesh), at, material, target);
    };
    let flat = |x: f32, y: f32| Vec3::new(x, y, 0.0);
    // bead blasted more than brushed
    let metal = Material {
      brushed: 0.35,
      grain: 0.3,
      ..Material::brushed(GUNMETAL)
    };
    let hole = Material::matte(Vec3::splat(0.01));
    let chrome = Material::chrome(Vec3::splat(0.75));
    // polished on a lathe, which catches the light in bright wedges around the knob
    let skirt_chrome = Material {
      spun: 1.0,
      ..Material::chrome(Vec3::splat(0.95))
    };
    // polished steel brushed along the bar, which stands out from the darker metal of the panel
    let steel = Material {
      brushed: 0.6,
      ..Material::chrome(Vec3::splat(0.85))
    };
    // the far side of anything cut into the plate, which the light reaches
    let lip = Material::matte(GUNMETAL.lerp(Vec3::ONE, 0.3));
    let (half_wide, half_tall) = (WIDTH / 2.0, HEIGHT / 2.0);

    // the plate, with a low rim along each edge that slopes down to it
    put(parts::plate(WIDTH, HEIGHT, 6.0), Vec3::ZERO, metal, None);
    let rim = |half: f32| {
      let outline = [[0.0, 0.0], [0.0, 2.0], [1.4, 2.0], [4.4, 0.0]].map(Vec2::from);
      Mesh::extruded(&outline, |_| half, 1.0)
    };
    for (half, x, y, turn) in [
      (half_wide, half_wide, 0.0, 0.0),
      (half_wide, half_wide, HEIGHT, PI),
      (half_tall, 0.0, half_tall, -FRAC_PI_2),
      (half_tall, WIDTH, half_tall, FRAC_PI_2),
    ] {
      let turned = rim(half).transformed(Mat4::from_rotation_z(turn));
      put(turned, flat(x, y), metal, None);
    }

    // a rack ear at each end, marked off by a groove, with a handle on two blocks down its
    // middle and a socket screw through a long hole near its top and its bottom, each
    // socket at another angle
    let handle = Handle::new(260.0, 22.0, 8.0);
    for x in [EAR, WIDTH - EAR] {
      for (from, to, up, material) in [(0.5, 1.2, 0.1, lip), (-0.5, 0.5, 0.2, hole)] {
        let groove = Mesh::cuboid(Vec3::new(from, 5.0, 0.0), Vec3::new(to, HEIGHT - 5.0, up));
        put(groove, flat(x, 0.0), material, None);
      }
    }
    for x in [EAR / 2.0, WIDTH - EAR / 2.0] {
      put(
        handle.mounts(),
        flat(x, half_tall),
        Material::brushed(Vec3::splat(0.4)),
        None,
      );
      put(handle.bar(), flat(x, half_tall), steel, None);
    }
    for (x, y, turn) in [
      (EAR / 2.0, 30.0, 0.4),
      (EAR / 2.0, HEIGHT - 30.0, 1.3),
      (WIDTH - EAR / 2.0, 30.0, 2.3),
      (WIDTH - EAR / 2.0, HEIGHT - 30.0, 0.9),
    ] {
      put(parts::slot(16.0, 9.0), flat(x + 0.5, y + 0.5), lip, None);
      put(parts::slot(16.0, 9.0), Vec3::new(x, y, 0.05), hole, None);
      let turned = Mat4::from_rotation_z(turn);
      put(
        parts::cap_screw(4.4).transformed(turned),
        flat(x, y),
        chrome,
        None,
      );
      let socket = parts::hex_socket(4.4).transformed(turned);
      put(socket, flat(x, y), hole, None);
    }

    // each group of controls and the screen are set in a field with a cut rim, with the name
    // over them between two lines of light that turn down at their far ends
    for Field {
      name,
      across: [left, right],
      down: [top, foot],
      y,
    } in fields
    {
      let field = parts::inset(
        right - left,
        foot - top,
        FIELD_BORDER,
        FIELD_UP,
        FIELD_CORNER,
      );
      let at = middle([left, right]);
      put(field, flat(at, middle([top, foot])), metal, None);
      put(
        lit_legend(name, HEADING_TALL),
        flat(at, y),
        lit(HEADING_LIT),
        None,
      );
      let clear = lit_width(name, HEADING_TALL) / 2.0 + 6.0;
      for (near, far) in [(at - clear, left + RULE_IN), (at + clear, right - RULE_IN)] {
        let path = [
          Vec2::new(near, y),
          Vec2::new(far, y),
          Vec2::new(far, y + RULE_DROP),
        ];
        put(
          parts::light_line_seat(&path, 0.9, 0.5),
          Vec3::ZERO,
          hole,
          None,
        );
        put(parts::light_line(&path, 0.9), Vec3::ZERO, lit(0.6), None);
      }
    }

    let switch = switch();
    let ring = shift_ring();
    let ring_seat = Arc::new(ring.seat());
    // every selector's lights are the same size, whatever its count of stops
    let stop = selector_stops(&[]);
    let stop_seat = Arc::new(stop.seat());
    for (index, (control, spot)) in Control::ALL.into_iter().zip(spots).enumerate() {
      let kind = control.kind();
      let Vec2 { x, y } = spot.middle();
      match kind {
        Kind::Shift => {
          fixed.put(
            &Arc::new(skirt(shift_knob())),
            flat(x, y),
            skirt_chrome,
            None,
          );
          fixed.extend(ring.seats(&ring_seat, x, y));
          // figures outside the ring, each where the knob points at that shift
          for share in [-1.0, -0.5, 0.0, 0.5, 1.0_f32] {
            let figure = match share * SHIFT_SEMITONES {
              0.0 => "0".to_owned(),
              semitones => format!("{semitones:+}"),
            };
            let at = around(Vec2::new(x, y), (share + 1.0) / 2.0, FIGURES_OUT);
            let figure = Arc::new(lit_legend(&figure, FIGURES_TALL));
            fixed.put(&figure, at.extend(0.0), lit(LABEL_LIT), None);
          }
        }
        Kind::Knob => {
          let skirt = Arc::new(skirt(small_knob()));
          fixed.put(&skirt, flat(x, y), skirt_chrome, None);
        }
        Kind::Selector { choices, printed } => {
          let skirt = Arc::new(skirt(small_knob()));
          fixed.put(&skirt, flat(x, y), skirt_chrome, None);
          let stops = selector_stops(choices);
          fixed.extend(stops.seats(&stop_seat, x, y));
          if printed {
            for (stop, name) in (0..).zip(choices) {
              let legend = Arc::new(lit_legend(name, CHOICE_TALL));
              let at = choice_at(Vec2::new(x, y), stops.turn(stop), name);
              fixed.put(&legend, at.extend(0.0), lit(LABEL_LIT), None);
            }
          }
        }
        Kind::Switch => {
          let collar = Arc::new(switch.collar());
          fixed.put(
            &collar,
            flat(x, y),
            Material::satin(Vec3::splat(0.03)),
            None,
          );
        }
      }
      let name = Arc::new(lit_legend(control.name(), kind.name_tall()));
      fixed.put(&name, flat(x, spot.row.name), lit(LABEL_LIT), None);
      // the rim the window its value shows in is sunk behind. the glass in it changes with
      // what it shows
      if kind.has_window() {
        let frame = WINDOW + WINDOW_FRAME;
        let rim = parts::inset(frame.x, frame.y, WINDOW_FRAME / 2.0, WINDOW_RIM, 3.5);
        let target = Some(Target::Readout(index));
        fixed.put(&Arc::new(rim), spot.window(), metal, target);
      }
    }

    // the screen in its bezel, with lines across and up it that the spectrum is drawn over
    let mut put = |mesh: Mesh, at: Vec3, material: Material| {
      fixed.put(&Arc::new(mesh), at, material, None);
    };
    let glass = screen_glass();
    let Vec2 { x, y } = SCREEN_AT;
    put(screen.face(), flat(x, y), glass);
    put(screen.bezel(), flat(x, y), Material::satin(STRIP_METAL));
    let grid = parts::picture(&lines(), x, y, glass, 0);
    let grid_objects = grid.len();
    fixed.extend(grid);

    // a strip of darker metal between the ears along the bottom. the plugin's name is lit on
    // it over a line of light, between two vents with light coming through them. what it is
    // is to the left, and to the right a maker's plate and the light that shows the effect
    let mut put = |mesh: Mesh, x: f32, y: f32, z: f32, material: Material| {
      fixed.put(&Arc::new(mesh), Vec3::new(x, y, z), material, None);
    };
    let strip = Material::brushed(STRIP_METAL);
    let strip_wide = WIDTH - 2.0 * EAR - 8.0;
    let slab = parts::chamfered_slab(strip_wide, STRIP_TALL, STRIP_UP, 1.2, 4.0);
    put(slab, half_wide, STRIP_Y, 0.0, strip);
    let name = slanted(lit_legend("CANWETALK", 13.0), 0.2);
    put(name, half_wide, STRIP_Y - 2.0, STRIP_UP, lit(1.0));
    let logo_half = lit_width("CANWETALK", 13.0) / 2.0;
    let underline = [
      Vec2::new(half_wide - logo_half, STRIP_Y + 9.5),
      Vec2::new(half_wide + logo_half - 4.0, STRIP_Y + 9.5),
    ];
    let seat = parts::light_line_seat(&underline, 0.9, 0.5);
    put(seat, 0.0, 0.0, STRIP_UP, hole);
    put(
      parts::light_line(&underline, 0.9),
      0.0,
      0.0,
      STRIP_UP,
      lit(0.7),
    );
    // holes coarse enough to show at the panel's size
    let vent = Vent {
      hole: 1.2,
      hole_pitch: 1.9,
      ..Vent::new(7, 5.0, 22.0)
    };
    let vent_half = vent.pitch * (vent.slots - 1) as f32 / 2.0 + vent.width / 2.0;
    for side in [-1.0, 1.0] {
      let x = half_wide + side * (logo_half + 22.0 + vent_half);
      put(vent.slots(), x, STRIP_Y, STRIP_UP, hole);
      put(vent.holes(), x, STRIP_Y, STRIP_UP, lit(0.55));
    }
    for (line, y, tall) in [
      ("PROFESSIONAL", -4.5, 4.4),
      ("PITCH + FORMANT SHIFTER", 4.0, 5.0),
    ] {
      put(
        lit_legend(line, tall),
        170.0,
        STRIP_Y + y,
        STRIP_UP,
        lit(LABEL_LIT),
      );
    }
    let plate = Vec3::new(WIDTH - 190.0, STRIP_Y, STRIP_UP);
    let plate_mesh = parts::chamfered_slab(96.0, 22.0, 1.2, 0.5, 1.5);
    put(
      plate_mesh,
      plate.x,
      plate.y,
      plate.z,
      Material::brushed(Vec3::splat(0.3)),
    );
    for (line, y) in [("MODEL CWT-3", -4.2), ("SERIAL NO 004217", 4.2)] {
      let engraved = lit_legend(line, 3.6);
      put(
        engraved,
        plate.x,
        plate.y + y,
        plate.z + 1.2,
        lit(LABEL_LIT),
      );
    }
    for side in [-1.0, 1.0] {
      let x = plate.x + side * 43.0;
      put(parts::cap_screw(1.6), x, plate.y, plate.z + 1.2, chrome);
      put(parts::hex_socket(1.6), x, plate.y, plate.z + 1.2, hole);
    }
    let label = lit_legend("EFFECT", 4.2);
    put(label, EFFECT.x - 30.0, STRIP_Y, STRIP_UP, lit(LABEL_LIT));
    put(
      parts::led_seat(EFFECT_SIZE),
      EFFECT.x,
      EFFECT.y,
      STRIP_UP,
      hole,
    );

    let arc = knob_arc();
    Self {
      spots,
      fixed,
      shift_knob: knob_parts(shift_knob()),
      small_knob: knob_parts(small_knob()),
      plunger: Arc::new(switch.plunger()),
      switch,
      ring_light: Arc::new(ring.segment()),
      ring,
      arc_meshes: [Arc::new(arc.seat()), Arc::new(arc.lens())],
      arc,
      arcs: Control::ALL.map(|_| None).into(),
      stop_light: Arc::new(stop.light()),
      effect: Arc::new(parts::led(EFFECT_SIZE)),
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
    for (index, (control, spot)) in Control::ALL.into_iter().zip(self.spots).enumerate() {
      let Vec2 { x, y } = spot.middle();
      let reading = &controls[index];
      let target = Some(Target::Control(index));
      let knob = match control.kind() {
        Kind::Shift => {
          // lit from nothing to the knob's shift, and for the formant more dimly on from there
          // by what formant follow adds to the formants of the voice that started last. a run
          // past either end of the scale stops there, which is as far as the ring shows,
          // though the shifter moves the formants further
          let followed = match control {
            Control::Formant => feed[FOLLOW_SLOT] / (2.0 * SHIFT_SEMITONES),
            _ => 0.0,
          };
          let shift = self.ring.levels([control.rest(), reading.value]);
          let follow = (self.ring).levels([reading.value, reading.value + followed]);
          let levels = shift
            .zip(follow)
            .map(|(shift, follow)| shift.max(follow * FOLLOW_LIT));
          drawn.extend((self.ring).lit(&self.ring_light, x, y, levels, CYAN));
          &self.shift_knob
        }
        Kind::Knob => {
          let run = [control.rest(), reading.value];
          let kept = (self.arcs[index].take())
            .filter(|kept| kept.of == run)
            .unwrap_or_else(|| Kept {
              of: run,
              objects: (self.arc.lights(&self.arc_meshes, x, y, run, CYAN)).collect(),
            });
          drawn.extend(kept.objects.iter().cloned());
          self.arcs[index] = Some(kept);
          &self.small_knob
        }
        Kind::Selector { choices, .. } => {
          let stops = selector_stops(choices);
          drawn.extend(stops.lights(&self.stop_light, x, y, reading.value, CYAN));
          &self.small_knob
        }
        Kind::Switch => {
          // a switch that is on stays in and is lit from under it, and throws its light
          // around it. one that is off is lit barely
          let on = reading.value > 0.5;
          let depth = Push::Latching.depth(on, held == Some(index));
          let level = if on { 1.0 } else { DARK };
          drawn.place(
            &self.plunger,
            self.switch.transform(x, y, depth),
            self.switch.backlight(CYAN, level),
            target,
          );
          if on {
            lamps.push(self.switch.plunger.lamp(x, y, CYAN, 0.6));
          }
          continue;
        }
      };
      let transform = parts::knob_transform(x, y, reading.value, parts::KNOB_SWEEP);
      for (mesh, material) in knob {
        drawn.place(mesh, transform, *material, target);
      }
      if control.kind().has_window() {
        let edit = edit.filter(|edit| edit.control == index);
        let Reading { text, unit, .. } = reading;
        self.readout(&mut drawn, index, spot.window(), (text, unit), edit);
      }
    }

    let Vec2 { x, y } = SCREEN_AT;
    let glass = screen_glass();
    let shown = (self.shown.take())
      .filter(|shown| shown.of == feed)
      .unwrap_or_else(|| Kept {
        of: feed.to_vec(),
        objects: parts::picture(&spectrum(feed), x, y, glass, self.grid),
      });
    drawn.extend(shown.objects.iter().cloned());
    self.shown = Some(shown);
    lamps.extend(self.screen.lamps(x, y, CYAN, 0.3));

    // the light is as bright as the mix is wet, and dark when nothing of the shift is heard
    let wet = controls[Control::Mix as usize].value;
    drawn.put(
      &self.effect,
      EFFECT.extend(STRIP_UP),
      parts::led_material(CYAN, wet, EFFECT_SIZE),
      None,
    );
    let over = EFFECT.extend(STRIP_UP + EFFECT_SIZE);
    lamps.push(parts::led_lamp(
      over,
      CYAN,
      wet,
      EFFECT_SIZE,
      EFFECT_SIZE * 5.0,
    ));
    let scene = Scene {
      size: Vec2::new(WIDTH, HEIGHT),
      heights: [-8.0, 48.0],
      lean: LEAN,
      // under a dim wide light, which leaves what glows to show
      light: Light {
        color: Vec3::splat(0.6),
        ambient: Vec3::splat(0.2),
        softness: 3.0,
        ..Light::default()
      },
      look: Look {
        soften: 0.2,
        vignette: 0.4,
        bloom: 0.6,
        bloom_radius: 10.0,
        ..Look::default()
      },
      objects: drawn.objects,
      lamps,
    };
    (scene, drawn.targets)
  }

  /// a control's window: smoked glass sunk in the panel, with its value and unit lit on it.
  /// while the value is typed the glass is lit too and shows what has been typed, with an
  /// amber caret in it or, for as long as all of it is selected, dark on an amber block
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
      // the key light is near and its mirror image falls on the windows at the top left,
      // which a pane's full shine would grey over
      specular: 0.08,
      // sized as a much larger pane, which keeps the sheen to one corner
      ..Material::pane(FACE, WINDOW.x * 3.5)
    };
    let glass = match edit {
      Some(_) => glass.lit(CYAN * 0.4, 0.3, 0.0),
      None => glass,
    };
    drawn.put(&self.glass, at, glass, target);

    let selected = edit.map_or(0..0, Edit::selected);
    let lettering = (self.readouts[index].take())
      .filter(|shown| shown.of.0 == figures && shown.of.1 == unit && shown.of.2 == selected)
      .unwrap_or_else(|| Lettering::new(figures, unit, selected));
    let figure_light = Material::lamp(CYAN);
    let typing = Material::lamp(AMBER);
    let on_glass = at + Vec3::Z * GLASS_UP;
    let figures_at = on_glass + Vec3::X * lettering.figures_at;
    drawn.put(&lettering.figures, figures_at, figure_light, target);
    if let Some((figures, block, across)) = &lettering.selected {
      drawn.put(block, figures_at + Vec3::X * *across, typing, target);
      let ink = Material::matte(Vec3::ZERO);
      drawn.put(figures, figures_at + Vec3::Z * 0.15, ink, target);
    }
    if let Some((unit, unit_at)) = &lettering.unit {
      drawn.put(unit, on_glass + unit_at.extend(0.0), figure_light, target);
    }
    if let Some(edit) = edit {
      let across = stroke::gap(edit.text.chars().count(), edit.caret, READOUT_TALL);
      drawn.put(&self.caret, figures_at + Vec3::X * across, typing, target);
    }
    self.readouts[index] = Some(lettering);
  }
}

/// the menu and the grip of the window, in the panel's colors
pub(super) fn furniture() -> (MenuLook, GripLook) {
  let mut menu = MenuLook::default();
  menu.ground = [0.05, 0.055, 0.06, 0.97];
  menu.line = [0.1, 0.4, 0.48, 1.0];
  menu.ink = [0.7, 0.93, 1.0, 1.0];
  menu.mark = EDGE;
  menu.marked_ink = [0.02, 0.04, 0.05, 1.0];
  let mut grip = GripLook::default();
  grip.ink = [CYAN.x, CYAN.y, CYAN.z, 0.5];
  (menu, grip)
}

/// the face of the screen, which what it shows is lit on
fn screen_glass() -> Material {
  Material {
    // smoked glass, which mirrors less of the room than clear
    reflect: 0.15,
    // sized as a much larger pane, which keeps the sheen to the corner over the top of the plot
    ..Material::screen(FACE, SCREEN_SIZE.length() * 5.5)
  }
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

/// what the screen shows of the feed: the level of each band of the input's spectrum as a
/// curve over a fill, the output's as a curve over that, and a line up at the detected pitch
/// and at each pitch a voice makes. beside them are the note nearest the detected pitch, how
/// many cents off it the pitch is and the pitch itself, and under those the notes the latest
/// voices make, with a count of the others. a note there is none of is two dashes
fn spectrum(feed: &[f32]) -> Flat {
  let mut flat = Flat::default();
  let curve = |first: usize| -> Vec<[f32; 2]> {
    (feed[first..first + BANDS].iter().enumerate())
      .map(|(band, level)| {
        let across = (band as f32 + 0.5) / BANDS as f32;
        plotted([across, LEVELS.turn(*level)])
      })
      .collect()
  };
  let heard = curve(INPUT_BANDS);
  flat.area(&heard, PLOT[1].y, FILL);
  flat.curve(&heard, 1.5, EDGE);
  // the output is an edge alone over the input's, which leaves the input's fill to show
  flat.curve(&curve(OUTPUT_BANDS), 1.2, MADE);
  // a pitch past either end of the plot has no line, which the axis would draw at that end,
  // and is still named beside it
  let up = |flat: &mut Flat, hz: f32, color: Color| {
    let [lowest, highest] = HERTZ_ENDS;
    if (lowest..=highest).contains(&hz) {
      let across = HERTZ.turn(hz);
      flat.line(plotted([across, 0.0]), plotted([across, 1.0]), 1.0, color);
    }
  };
  let made: Vec<f32> = (feed[VOICE_SLOTS].iter().copied())
    .filter(|&hz| hz > 0.0)
    .collect();
  for &hz in &made {
    up(&mut flat, hz, MADE_PITCH);
  }

  // the column beside the plot: the input's note over the notes the voices make
  flat.centered("IN", [NOTE_X, SCREEN_AT.y - 56.0], 5.0, 1.0, LABELS);
  let hz = feed[PITCH_SLOT];
  if hz > 0.0 {
    up(&mut flat, hz, PITCH);
    let (name, cents) = note::nearest(hz);
    flat.centered(&name, [NOTE_X, SCREEN_AT.y - 42.0], NOTE_TALL, 2.2, EDGE);
    let cents = format!("{cents:+} CT");
    flat.centered(&cents, [NOTE_X, SCREEN_AT.y - 28.0], 5.5, 1.1, EDGE);
    let hertz = format!("{hz:.1} HZ");
    flat.centered(&hertz, [NOTE_X, SCREEN_AT.y - 19.0], 5.0, 1.0, LABELS);
  } else {
    flat.centered("--", [NOTE_X, SCREEN_AT.y - 42.0], NOTE_TALL, 2.2, LABELS);
  }
  flat.centered("OUT", [NOTE_X, SCREEN_AT.y - 4.0], 5.0, 1.0, MADE_LABELS);
  if made.is_empty() {
    flat.centered(
      "--",
      [NOTE_X, SCREEN_AT.y + 10.0],
      MADE_TALL,
      1.8,
      MADE_LABELS,
    );
  }
  for (row, &hz) in made.iter().take(MADE_NAMES).enumerate() {
    let (name, _) = note::nearest(hz);
    let y = SCREEN_AT.y + 10.0 + 14.0 * row as f32;
    flat.centered(&name, [NOTE_X, y], MADE_TALL, 1.8, MADE);
  }
  if let Some(more) = made.len().checked_sub(MADE_NAMES).filter(|&more| more > 0) {
    let more = format!("+{more}");
    flat.centered(&more, [NOTE_X, SCREEN_AT.y + 51.0], 5.5, 1.1, MADE_LABELS);
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
  use potpot_controls::flat::Shape;
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

  /// a voice made under the lowest frequency the plot shows, or over its highest, has no line.
  /// the axis puts such a pitch at the end it is past, where a line would say it is that end's
  #[test]
  fn a_pitch_past_the_plot_draws_no_line_off_it() {
    let mut feed = vec![FLOOR_DB; SLOTS];
    feed[PITCH_SLOT] = 220.0;
    // key 28 shifted down two octaves, and one past the highest
    let made = [10.3, 440.0, 21_000.0];
    for (slot, hz) in VOICE_SLOTS.zip(made) {
      feed[slot] = hz;
    }
    let flat = spectrum(&feed);
    let edge = SCREEN_AT.x + SCREEN_SIZE.x / 2.0;
    let mut pitch_lines = 0;
    for shape in flat.shapes() {
      if let Shape::Line { from, to, .. } = shape {
        for [x, _] in [from, to] {
          assert!((PLOT[0].x..=edge).contains(x), "a line at {x}");
        }
        pitch_lines += usize::from(from[1] == PLOT[1].y && to[1] == PLOT[0].y);
      }
    }
    // the input's and the one voice inside the plot's, and none for the others
    assert_eq!(pitch_lines, 2);
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
      // the middle of a knob or a switch and a little off it each way, which its top is seen
      // leaned from
      for off in [Vec2::ZERO, Vec2::X * 6.0, -Vec2::X * 6.0, Vec2::Y * 6.0] {
        let at = spot.middle() + off;
        assert_eq!(
          picked(at),
          Some(Target::Control(index)),
          "{control:?} at {at}"
        );
      }
      // the window, its glass and its rim
      if control.kind().has_window() {
        let at = spot.window().truncate();
        let rim = WINDOW.x / 2.0 + WINDOW_FRAME / 4.0;
        for off in [0.0, WINDOW.x / 2.0 - 2.0, rim, -rim] {
          assert_eq!(
            picked(at + Vec2::X * off),
            Some(Target::Readout(index)),
            "{control:?} {off} across its window"
          );
        }
      }
    }
  }

  #[test]
  fn every_control_fits_in_its_field_and_no_two_fields_meet() {
    let screen = Screen::new(SCREEN_SIZE.x, SCREEN_SIZE.y);
    let (spots, fields) = layout(screen.border);
    // each control is in one group alone, which with every one placed is once each
    let groups = GROUPS.iter().flat_map(|row| row.iter());
    let grouped: usize = groups.clone().map(|(_, controls)| controls.len()).sum();
    assert_eq!(grouped, Control::ALL.len());

    // between the ears and over the strip, and clear of each other
    for field in &fields {
      let [left, right] = field.across;
      let [top, foot] = field.down;
      assert!(left >= LEFT && right <= RIGHT, "{field:?}");
      assert!(
        top >= TOP_MARGIN && foot <= STRIP_Y - STRIP_TALL / 2.0,
        "{field:?}"
      );
      // the lines of light beside its name reach out from it
      let clear = lit_width(field.name, HEADING_TALL) / 2.0 + 6.0;
      assert!(clear < (right - left) / 2.0 - RULE_IN, "{field:?}");
    }
    for (index, one) in fields.iter().enumerate() {
      for other in &fields[index + 1..] {
        let apart = |side: fn(&Field) -> [f32; 2]| {
          let ([a_from, a_to], [b_from, b_to]) = (side(one), side(other));
          a_to <= b_from || b_to <= a_from
        };
        assert!(
          apart(|field| field.across) || apart(|field| field.down),
          "{one:?} meets {other:?}"
        );
      }
    }

    // each control lies inside its group's field, all it reaches between the drops of the
    // lines beside the name and the rim at the foot, and its name no wider than its slot
    for (group, controls) in groups {
      let field = (fields.iter())
        .find(|field| field.name == *group)
        .expect("a field for each group");
      let foot = field.down[1] - FIELD_BORDER;
      for &control in *controls {
        let (kind, spot) = (control.kind(), spots[control as usize]);
        let [up, down] = kind.reach();
        let half = kind.slot() / 2.0;
        assert!(
          spot.x - half >= field.across[0] && spot.x + half <= field.across[1],
          "{control:?}"
        );
        assert!(spot.row.middle - up > field.y + RULE_DROP, "{control:?}");
        assert!(spot.row.middle + down < spot.row.name, "{control:?}");
        // and the names on it clear of the rim of the field at either end of a row, by a few
        // points
        let room = kind.slot() / 2.0 - FIELD_BORDER - 4.0;
        assert!(
          lit_width(control.name(), kind.name_tall()) / 2.0 <= room,
          "{control:?}"
        );
        if let Kind::Selector {
          choices,
          printed: true,
        } = kind
        {
          for (stop, name) in (0..).zip(choices) {
            let at = choice_at(Vec2::ZERO, selector_stops(choices).turn(stop), name);
            let half = lit_width(name, CHOICE_TALL) / 2.0;
            assert!(at.x.abs() + half <= room, "{control:?} {name}");
          }
        }
        let lowest = if kind.has_window() {
          spot.row.window + (WINDOW.y + WINDOW_FRAME) / 2.0
        } else {
          spot.row.name + kind.name_tall() / 2.0
        };
        assert!(lowest <= foot, "{control:?}");
      }
    }

    // the screen and its bezel lie in their field, under its name
    let input = (fields.iter())
      .find(|field| field.name == "INPUT")
      .expect("a field for the screen");
    let half = SCREEN_SIZE / 2.0 + screen.border;
    let [left, right] = input.across;
    assert!(SCREEN_AT.x - half.x >= left + FIELD_BORDER);
    assert!(SCREEN_AT.x + half.x <= right - FIELD_BORDER);
    assert!(SCREEN_AT.y - half.y > input.y + RULE_DROP);
    assert!(SCREEN_AT.y + half.y <= input.down[1] - FIELD_BORDER);
  }
}
