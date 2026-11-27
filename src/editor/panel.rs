//! what the panel is made of and where everything on it goes

use std::f32::consts::{FRAC_PI_2, PI};
use std::ops::Range;
use std::sync::Arc;

use potpot::glam::{Mat4, Vec2, Vec3};
use potpot::parts::{self, Handle, LedArc, LedRing, LedStops, Meter, PushSwitch, Screen, Vent};
use potpot::{Anisotropy, Light, Look, Material, Mesh, Object, Place, Renderer, Scene};
use potpot_controls::axis::Axis;
use potpot_controls::flat::{Color, Flat};
use potpot_controls::furniture::{GripLook, MenuLook};
use potpot_controls::interaction::{Edit, Push, Reading, Target};
use potpot_controls::layout::{Pack, middle, pack};
use potpot_controls::stroke;
use potpot_window::{Gpu, Skin, View, wgpu};

use nice_plug::prelude::Enum;

use super::{Control, Kind, note};
use crate::notes::Mode;
use crate::params::SHIFT_SEMITONES;
use crate::pitch::Range as Detected;
use crate::spectrum::{
  BANDS, FLOOR_DB, FOLLOW_SLOT, HERTZ, HERTZ_ENDS, INPUT_BANDS, INPUT_LEVEL, OUTPUT_BANDS,
  OUTPUT_LEVEL, PITCH_SLOT, VOICE_SLOTS,
};

pub(super) const WIDTH: f32 = 920.0;
pub(super) const HEIGHT: f32 = 525.0;
/// how far a point slides across the picture for each unit of its height: seen a little
/// from in front
const LEAN: Vec2 = Vec2::new(0.0, -0.3);
/// how wide the rack ear at each end of the plate is
const EAR: f32 = 32.0;
/// the ends of the room the controls have across the panel, clear of the ears
const LEFT: f32 = EAR + 8.0;
const RIGHT: f32 = WIDTH - EAR - 8.0;
/// the light built into the hardware
const CYAN: Vec3 = Vec3::new(0.2, 0.85, 1.0);
/// the color of the caret and the selection while a value is typed
const AMBER: Vec3 = Vec3::new(1.0, 0.66, 0.14);
/// the one color for a warning, which a level meter turns near full scale
const ORANGE: Vec3 = Vec3::new(1.0, 0.38, 0.05);
/// the dark metal of the panel, and the darker metal of the strip along its bottom
const GUNMETAL: Vec3 = Vec3::new(0.3, 0.31, 0.32);
const STRIP_METAL: Vec3 = Vec3::new(0.2, 0.205, 0.21);
/// how much further apart lit letters are than the face sets them, in units of the grid a
/// letter is drawn on, and how wide their strokes are for each point of their height
const TRACKING: f32 = 1.0;
const LIT_WEIGHT: f32 = 0.13;
/// how brightly the names of the groups are lit, and the rest of the lettering on the panel
const HEADING_LIT: f32 = 1.0;
const LABEL_LIT: f32 = 0.85;
/// the windows the controls' values show in
const WINDOW: Vec2 = Vec2::new(64.0, 17.0);
/// how much wider and taller the rim around a window is than the window, and how high it
/// stands over the glass
const WINDOW_FRAME: f32 = 4.0;
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
/// the knobs: the radius and height of a shift's, of a selector's to the tip of its point,
/// and of the others'
const SHIFT_RADIUS: f32 = 33.0;
const SHIFT_HEIGHT: f32 = 26.0;
const SELECTOR_RADIUS: f32 = 16.0;
const SELECTOR_HEIGHT: f32 = 15.0;
const SMALL_RADIUS: f32 = 17.0;
const SMALL_HEIGHT: f32 = 15.0;
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
/// how brightly the envelope's lettering, lights and screen are lit, for each part of their
/// full light, while the mode plays no envelope
const DIMMED: f32 = 0.3;
/// how wide the envelope's screen draws the sustain held, in turns of a time's knob
const HOLD: f32 = 0.6;
/// how bright a shift's ring is over what formant follow adds to the shift
const FOLLOW_LIT: f32 = 0.35;
/// the field each group of controls is set in: how wide its rim is, how high and the radius
/// of its corners, and the room inside it over the group's name, under its windows and at
/// either end of its controls
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
/// how wide the light on the strip is that is as bright as the output is loud
const LOUD_LIGHT_SIZE: f32 = 7.0;
/// how tall the names on the strip are
const STRIP_LETTERS: f32 = 4.2;
/// the small knob on the strip: its radius and height
const TRIM_RADIUS: f32 = 8.0;
const TRIM_HEIGHT: f32 = 9.0;
/// the level meters on the strip: how long each is and how many lights it has, the quietest
/// level they show in decibels, and the level they turn orange from
const METER_LONG: f32 = 120.0;
const METER_LIGHTS: u32 = 24;
const METER_FLOOR_DB: f32 = -60.0;
const METER_WARN_DB: f32 = -3.0;
/// the name over the screen, which shows both spectra and the notes
const SCREEN_NAME: &str = "SPECTRUM";
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
/// the lines within the range of pitches detection looks for, and at its ends
const RANGE_GRID: Color = [0.06, 0.19, 0.24, 1.0];
const RANGE_EDGE: Color = [0.1, 0.36, 0.45, 1.0];
const LABELS: Color = [0.12, 0.42, 0.52, 1.0];
const FILL: Color = [0.02, 0.12, 0.16, 1.0];
const EDGE: Color = [CYAN.x, CYAN.y, CYAN.z, 1.0];
const PITCH: Color = [0.92, 1.0, 1.0, 1.0];
const MADE: Color = [0.78, 0.96, 1.0, 1.0];
const MADE_PITCH: Color = [0.38, 0.5, 0.56, 1.0];
const MADE_LABELS: Color = [0.28, 0.4, 0.46, 1.0];

/// a group by where its parts are down the panel: its name, the middle of the envelope's
/// screen, the middles of its controls, the names under them and the windows their values show
/// in
#[derive(Clone, Copy, Debug)]
struct Row {
  heading: f32,
  display: f32,
  middle: f32,
  name: f32,
  window: f32,
}

/// the screen the envelope's shape is drawn on, over the envelope's controls: the height of its
/// face, the width of its bezel, and the room across between the bezel and the field's rim
const DISPLAY_TALL: f32 = 40.0;
const DISPLAY_BORDER: f32 = 2.0;
const DISPLAY_CLEAR: f32 = 4.0;
/// the room between two fields side by side or one over the other
const FIELD_GAP: f32 = 6.0;

/// the room over the first row, at the top of the panel, and under the last, over the strip
const TOP_MARGIN: f32 = 10.0;
const STRIP_CLEAR: f32 = 8.0;
/// the room in a row between the drops at the ends of its rules and the tops of its controls,
/// between the controls and the names under them, and between the names and the windows
const OVER_GAP: f32 = 5.5;
const UNDER_GAP: f32 = 10.0;
const NAME_GAP: f32 = 6.0;

/// a group of controls, set in a field under its name
struct Group {
  name: &'static str,
  controls: &'static [Control],
  /// whether it is the envelope's: it shows the envelope's shape on a screen over its
  /// controls, and is dimmed while the mode plays no envelope
  envelope: bool,
}

impl Group {
  const fn new(name: &'static str, controls: &'static [Control]) -> Self {
    Self {
      name,
      controls,
      envelope: false,
    }
  }
}

/// the rows of the panel, top first, each a run of columns left to right and each column groups
/// top to bottom. the first row's one column keeps to the left, beside the screen, and the
/// columns of the second are spread from end to end, the notes, then the sound, then what the
/// shifter listens to
const ROWS: [&[&[Group]]; 2] = [
  &[&[Group::new(
    "SHIFT",
    &[
      Control::Pitch,
      Control::Formant,
      Control::FormantFollow,
      Control::Smooth,
    ],
  )]],
  &[
    &[
      Group::new(
        "NOTES",
        &[
          Control::Mode,
          Control::Latch,
          Control::Glide,
          Control::GlideNotesOnly,
        ],
      ),
      Group {
        envelope: true,
        ..Group::new(
          "ENVELOPE",
          &[
            Control::Attack,
            Control::Decay,
            Control::Sustain,
            Control::Release,
          ],
        )
      },
    ],
    &[
      Group::new(
        "EFFECTS",
        &[Control::Detune, Control::Smear, Control::Stereo],
      ),
      Group::new(
        "VIBRATO",
        &[
          Control::VibratoDepth,
          Control::VibratoRate,
          Control::VibratoSync,
          Control::VibratoDivision,
        ],
      ),
    ],
    &[
      Group::new(
        "EXPRESSION",
        &[
          Control::PressureLevel,
          Control::SlideFormant,
          Control::BendRange,
        ],
      ),
      Group::new("DETECTION", &[Control::Detect, Control::Anchor]),
    ],
  ],
];

/// the knob the shifts are turned by
fn shift_knob() -> parts::Knob {
  parts::Knob::skirted(SHIFT_RADIUS, SHIFT_HEIGHT)
}

/// the knob on the strip, ribbed for a fingertip with a capped face
fn trim_knob() -> parts::Knob {
  parts::Knob::ribbed(TRIM_RADIUS, TRIM_HEIGHT)
}

/// the knob a selector is turned by, which points
fn selector_knob() -> parts::Knob {
  parts::Knob::chicken_head(SELECTOR_RADIUS, SELECTOR_HEIGHT)
}

/// the knob the other controls that turn are turned by, knurled with a flat face for a cap
fn small_knob() -> parts::Knob {
  parts::Knob {
    top: parts::Top::Flat,
    ..parts::Knob::knurled(SMALL_RADIUS, SMALL_HEIGHT)
  }
}

/// the lights around a shift's knob, which show its value either way from nothing
fn shift_ring() -> LedRing {
  LedRing::new(24, SHIFT_RADIUS + 6.0, 5.0)
}

/// the unbroken light around the knob of a control that turns smoothly
fn knob_arc() -> LedArc {
  LedArc::new(SMALL_RADIUS + 5.0, 2.4)
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
      Self::Knob | Self::Selector { printed: false, .. } => 72.0,
      Self::Switch => 70.0,
      Self::Trim => 2.0 * TRIM_RADIUS,
    }
  }

  /// how far a part of this kind reaches up the panel from its middle, and down, flat on the
  /// panel
  fn reach(self) -> [f32; 2] {
    match self {
      // over the figure at the top of its scale, and under those at the ends of its turn or
      // its skirt, whichever is lower
      Self::Shift => {
        let ends = down_at_ends(FIGURES_OUT) + FIGURES_TALL / 2.0;
        [FIGURES_OUT + FIGURES_TALL / 2.0, ends.max(SHIFT_RADIUS)]
      }
      // over the light around it, and under its ends or the knob
      Self::Knob => {
        let arc = knob_arc();
        let out = arc.radius + arc.width / 2.0 + arc.margin;
        [out, down_at_ends(out).max(SMALL_RADIUS)]
      }
      // over the name of the choice at the top, and under those at the ends of its turn
      Self::Selector { printed: true, .. } => [
        CHOICE_OUT + CHOICE_TALL / 2.0,
        down_at_ends(CHOICE_OUT) + CHOICE_TALL / 2.0,
      ],
      // over and under its lights, or under its point if that is lower
      Self::Selector {
        choices,
        printed: false,
      } => {
        let [up, down] = selector_stops(choices).reach();
        [up, down.max(SELECTOR_RADIUS)]
      }
      // over and under the collar
      Self::Trim => [TRIM_RADIUS, TRIM_RADIUS],
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
      Self::Switch | Self::Trim => false,
    }
  }

  /// how tall the name under a control of this kind is
  fn name_tall(self) -> f32 {
    match self {
      Self::Shift => 7.5,
      Self::Knob | Self::Selector { .. } => 5.5,
      Self::Switch => 5.0,
      Self::Trim => STRIP_LETTERS,
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

/// where a control is: across the panel, and its row. `envelope` is whether it is one of the
/// envelope's
#[derive(Clone, Copy, Debug)]
struct Spot {
  x: f32,
  row: Row,
  envelope: bool,
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
/// how far down the name is. the envelope's has the screen its shape is drawn on
#[derive(Clone, Debug)]
struct Field {
  name: &'static str,
  across: [f32; 2],
  down: [f32; 2],
  y: f32,
  display: Option<Display>,
}

/// the screen the envelope's shape is drawn on, by the middle and the size of its face
#[derive(Clone, Copy, Debug)]
struct Display {
  at: Vec2,
  size: Vec2,
}

impl Display {
  fn screen(self) -> Screen {
    Screen {
      border: DISPLAY_BORDER,
      ..Screen::new(self.size.x, self.size.y)
    }
  }
}

/// the parts of a group down the panel, from the top of its field to its foot: the name and the
/// drops of the lines beside it, the envelope's screen if it is the envelope's, its controls,
/// their names and their windows if they have any
fn bands(group: &Group) -> [f32; 13] {
  let kinds = || group.controls.iter().map(|control| control.kind());
  let largest = |of: fn(Kind) -> f32| kinds().map(of).fold(0.0, f32::max);
  let [name_gap, window] = if kinds().any(Kind::has_window) {
    [NAME_GAP, WINDOW.y + WINDOW_FRAME]
  } else {
    [0.0, 0.0]
  };
  let [display_gap, display] = if group.envelope {
    [OVER_GAP, DISPLAY_TALL + 2.0 * DISPLAY_BORDER]
  } else {
    [0.0, 0.0]
  };
  [
    FIELD_PAD,
    HEADING_TALL,
    // the drop starts at the middle of the letters
    RULE_DROP - HEADING_TALL / 2.0,
    display_gap,
    display,
    OVER_GAP,
    largest(|kind| kind.reach()[0]),
    largest(|kind| kind.reach()[1]),
    UNDER_GAP,
    largest(Kind::name_tall),
    name_gap,
    window,
    FIELD_PAD,
  ]
}

/// how tall a group's field is at least
fn group_tall(group: &Group) -> f32 {
  bands(group).iter().sum()
}

/// how wide a group's field is at least, with its controls side by side
fn group_wide(group: &Group) -> f32 {
  let slots: f32 = (group.controls.iter())
    .map(|control| control.kind().slot())
    .sum();
  slots + 2.0 * FIELD_PAD
}

/// how tall a column of groups is at least, with a gap between each two
fn column_tall(column: &[Group]) -> f32 {
  let fields: f32 = column.iter().map(group_tall).sum();
  fields + FIELD_GAP * column.len().saturating_sub(1) as f32
}

/// sets a group in a field `across` and `down` the panel, with its controls spread across it and
/// set halfway down the room under its name: a field, and a spot for each control. returns where
/// its parts are
fn place_group(
  group: &Group,
  across: [f32; 2],
  down: [f32; 2],
  spots: &mut [Option<Spot>; Control::ALL.len()],
  fields: &mut Vec<Field>,
) -> Row {
  let mut bands = bands(group);
  let spare = down[1] - down[0] - bands.iter().sum::<f32>();
  // the room over the controls and under the windows
  bands[5] += spare / 2.0;
  bands[12] += spare / 2.0;
  let placed = <[_; 13]>::try_from(pack(down, &bands, Pack::Start));
  let [_, letters, _, _, display, _, over, _, _, name, _, window, _] =
    placed.expect("a place for each band");
  let row = Row {
    heading: middle(letters),
    display: middle(display),
    middle: over[1],
    name: middle(name),
    window: middle(window),
  };
  let inside = [across[0] + FIELD_PAD, across[1] - FIELD_PAD];
  fields.push(Field {
    name: group.name,
    across,
    down,
    y: row.heading,
    display: group.envelope.then(|| Display {
      at: Vec2::new(middle(across), row.display),
      size: Vec2::new(
        inside[1] - inside[0] - 2.0 * (DISPLAY_BORDER + DISPLAY_CLEAR),
        DISPLAY_TALL,
      ),
    }),
  });
  let slots: Vec<f32> = (group.controls.iter())
    .map(|control| control.kind().slot())
    .collect();
  let slotted = pack(inside, &slots, Pack::Spread);
  for (&control, slot) in group.controls.iter().zip(slotted) {
    spots[control as usize] = Some(Spot {
      x: middle(slot),
      row,
      envelope: group.envelope,
    });
  }
  row
}

/// where each control is, in the order of [`Control::ALL`], and the fields the groups of
/// controls and the screen are set in. `border` is the width of the screen's bezel
fn layout(border: f32) -> ([Spot; Control::ALL.len()], Vec<Field>) {
  let mut spots = [None; Control::ALL.len()];
  let mut fields = Vec::new();
  // each row as tall as its tallest column, and the first at least as tall as the screen in its
  // bezel
  let screen_foot = SCREEN_AT.y + SCREEN_SIZE.y / 2.0 + border + FIELD_PAD;
  let mut tall = ROWS.map(|row| {
    row
      .iter()
      .map(|column| column_tall(column))
      .fold(0.0, f32::max)
  });
  tall[0] = tall[0].max(screen_foot - TOP_MARGIN);
  // the rows spread from the top margin to the strip
  let down = [TOP_MARGIN, STRIP_Y - STRIP_TALL / 2.0 - STRIP_CLEAR];
  let rows = pack(down, &tall, Pack::Spread);
  // the room each row's columns share, and how. the screen's field reaches to the right end
  let screen = [RIGHT - SCREEN_SIZE.x - 2.0 * SCREEN_CLEAR, RIGHT];
  let rooms = [
    ([LEFT, screen[0] - FIELD_GAP], Pack::Start),
    ([LEFT, RIGHT], Pack::Spread),
  ];
  let mut headings = Vec::new();
  for ((row, &span), (room, how)) in ROWS.iter().zip(&rows).zip(rooms) {
    let widths: Vec<f32> = (row.iter())
      .map(|column| column.iter().map(group_wide).fold(0.0, f32::max))
      .collect();
    for (column, across) in row.iter().zip(pack(room, &widths, how)) {
      // a column shorter than its row shares what it has spare between its groups' fields
      let spare = (span[1] - span[0] - column_tall(column)) / column.len() as f32;
      let heights: Vec<f32> = (column.iter())
        .map(|group| group_tall(group) + spare)
        .collect();
      for (group, down) in column.iter().zip(pack(span, &heights, Pack::Spread)) {
        let lines = place_group(group, across, down, &mut spots, &mut fields);
        headings.push(lines.heading);
      }
    }
  }
  // the mix's knob is on the strip, which has no rows: every part of it is on its middle
  let on_strip = Row {
    heading: STRIP_Y,
    display: STRIP_Y,
    middle: STRIP_Y,
    name: STRIP_Y,
    window: STRIP_Y,
  };
  spots[Control::Mix as usize] = Some(Spot {
    x: middle(strip().mix),
    row: on_strip,
    envelope: false,
  });
  fields.push(Field {
    name: SCREEN_NAME,
    across: screen,
    down: rows[0],
    y: headings[0],
    display: None,
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

/// the vents either side of the plugin's name, with holes coarse enough to show at the panel's
/// size
fn vent() -> Vent {
  Vent {
    hole: 1.2,
    hole_pitch: 1.9,
    ..Vent::new(7, 5.0, 22.0)
  }
}

/// where each part of the strip along the bottom is across the panel
struct Strip {
  in_meter: [f32; 2],
  in_name: [f32; 2],
  in_vent: [f32; 2],
  logo: [f32; 2],
  out_vent: [f32; 2],
  out_name: [f32; 2],
  out_meter: [f32; 2],
  mix_name: [f32; 2],
  mix: [f32; 2],
  loud: [f32; 2],
}

/// the names of the meters on the strip, the input's and the output's
const METER_NAMES: [&str; 2] = ["IN", "OUT"];

/// the parts of the strip: in its middle the plugin's name between two vents, past each a
/// meter's name and the meter, and at the right end the mix's name, its knob and the light
/// that shows how loud the output is
fn strip() -> Strip {
  let vent = vent();
  let vent_wide = vent.pitch * (vent.slots - 1) as f32 + vent.width;
  // both meters' names take the room of the wider, which keeps the plugin's name in the middle
  let name_wide = (METER_NAMES.iter())
    .map(|name| lit_width(name, STRIP_LETTERS))
    .fold(0.0, f32::max);
  let logo_wide = lit_width("CANWETALK", 13.0);
  // each part, with the gap after it
  let middle_parts = [
    METER_LONG, 8.0, name_wide, 18.0, vent_wide, 22.0, logo_wide, 22.0, vent_wide, 18.0, name_wide,
    8.0, METER_LONG,
  ];
  let [
    in_meter,
    _,
    in_name,
    _,
    in_vent,
    _,
    logo,
    _,
    out_vent,
    _,
    out_name,
    _,
    out_meter,
  ] = <[_; 13]>::try_from(pack([EAR, WIDTH - EAR], &middle_parts, Pack::Center))
    .expect("a place for each part of the middle of the strip");
  let end_parts = [
    lit_width(Control::Mix.name(), STRIP_LETTERS),
    6.0,
    2.0 * TRIM_RADIUS,
    10.0,
    LOUD_LIGHT_SIZE,
  ];
  // up to a little way in from the strip's right end
  let end = WIDTH - EAR - 18.0;
  let start = end - end_parts.iter().sum::<f32>();
  let [mix_name, _, mix, _, loud] = <[_; 5]>::try_from(pack([start, end], &end_parts, Pack::Start))
    .expect("a place for each part of the end of the strip");
  Strip {
    in_meter,
    in_name,
    in_vent,
    logo,
    out_vent,
    out_name,
    out_meter,
    mix_name,
    mix,
    loud,
  }
}

/// a level meter on the strip, lying on its side once turned a quarter
fn meter() -> Meter {
  let warn = (METER_WARN_DB - METER_FLOOR_DB) / -METER_FLOOR_DB;
  Meter {
    colors: [CYAN, CYAN, ORANGE],
    yellow_from: warn,
    red_from: warn,
    ..Meter::ladder(METER_LIGHTS, 5.0, METER_LONG)
  }
}

/// how much of a level meter a level in decibels lights, from none at its floor to all of it
/// at full scale
fn meter_level(db: f32) -> f32 {
  ((db - METER_FLOOR_DB) / -METER_FLOOR_DB).clamp(0.0, 1.0)
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

/// the meshes of a knob that turn with it: its glossy black body, a face of spun aluminium if
/// it is `capped`, and the lit line on it
fn knob_parts(knob: parts::Knob, capped: bool) -> Parts {
  // black plastic with a tight highlight
  let body = Material {
    roughness: 0.12,
    ..Material::glossy(Vec3::splat(0.008))
  };
  let mut parts = vec![(Arc::new(knob.body()), body)];
  if capped {
    parts.push((
      Arc::new(knob.cap(0.86)),
      Material::spun(Vec3::new(0.78, 0.79, 0.81)),
    ));
  }
  parts.push((Arc::new(knob.line()), Material::lamp(CYAN)));
  parts
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
  /// the lit lettering and lines of the envelope's group, each with how brightly it is lit
  /// while the envelope plays
  dimmed: Vec<(Arc<Mesh>, Vec3, f32)>,
  /// the envelope's screen, and its shape as last drawn, kept for as long as its controls and
  /// its light are the same
  display: Display,
  shape: Option<Kept<[f32; 5]>>,
  /// the parts of a shift's knob and of the others
  shift_knob: Parts,
  selector_knob: Parts,
  small_knob: Parts,
  trim_knob: Parts,
  switch: PushSwitch,
  plunger: Arc<Mesh>,
  ring: LedRing,
  /// one light of a shift's ring
  ring_light: Arc<Mesh>,
  arc: LedArc,
  /// the dark gap an arc is set in and its lens
  arc_meshes: [Arc<Mesh>; 2],
  /// the lit arc around each control, kept for as long as it is lit over the same run as
  /// brightly
  arcs: Vec<Option<Kept<[f32; 3]>>>,
  /// the light at each stop of a selector
  stop_light: Arc<Mesh>,
  /// the light that shows how loud the output is, and its middle
  loud_light: Arc<Mesh>,
  loud: Vec2,
  meter: Meter,
  meter_light: Arc<Mesh>,
  /// where the input's meter and the output's lie, each from its quiet end
  meters: [Mat4; 2],
  glass: Arc<Mesh>,
  /// the lettering of what each window shows, kept for as long as it shows that
  readouts: Vec<Option<Lettering>>,
  caret: Arc<Mesh>,
  screen: Screen,
  /// how many objects the screen's lines and their names are, which what it shows lies over
  grid: usize,
  shown: Option<Kept<(Vec<f32>, [f32; 2])>>,
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
      anisotropy: Anisotropy::Brushed(0.35),
      grain: 0.3,
      ..Material::brushed(GUNMETAL)
    };
    let hole = Material::matte(Vec3::splat(0.01));
    let chrome = Material::chrome(Vec3::splat(0.75));
    // polished steel brushed along the bar, which stands out from the darker metal of the panel
    let steel = Material {
      anisotropy: Anisotropy::Brushed(0.6),
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
    let mut dimmed = Vec::new();
    let mut envelope_display = None;
    for Field {
      name,
      across: [left, right],
      down: [top, foot],
      y,
      display: shown_at,
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
      // the name and the lines are lit, and the envelope's are dimmed with it
      let mut lights = vec![(lit_legend(name, HEADING_TALL), flat(at, y), HEADING_LIT)];
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
        lights.push((parts::light_line(&path, 0.9), Vec3::ZERO, 0.6));
      }
      // only the envelope's field has its screen
      if let Some(display) = shown_at {
        let (screen, at) = (display.screen(), display.at.extend(0.0));
        put(screen.face(), at, screen_glass(), None);
        put(screen.bezel(), at, Material::satin(STRIP_METAL), None);
        envelope_display = Some(display);
        dimmed.extend((lights.into_iter()).map(|(mesh, at, level)| (Arc::new(mesh), at, level)));
      } else {
        for (mesh, at, level) in lights {
          put(mesh, at, lit(level), None);
        }
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
        // a knob's light and the gap it is set in change with its value, so are drawn with it
        Kind::Knob | Kind::Trim => {}
        Kind::Selector { choices, printed } => {
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
      // the strip's knob has its name drawn with the strip
      if kind != Kind::Trim {
        let name = Arc::new(lit_legend(control.name(), kind.name_tall()));
        let at = flat(x, spot.row.name);
        if spot.envelope {
          dimmed.push((name, at, LABEL_LIT));
        } else {
          fixed.put(&name, at, lit(LABEL_LIT), None);
        }
      }
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

    // a strip of darker metal between the ears along the bottom, with the parts `strip` places.
    // the plugin's name is lit over a line of light, light comes through the vents, and the
    // meters run left to right
    let mut put = |mesh: Mesh, x: f32, y: f32, z: f32, material: Material| {
      fixed.put(&Arc::new(mesh), Vec3::new(x, y, z), material, None);
    };
    let strip_metal = Material::brushed(STRIP_METAL);
    let strip_wide = WIDTH - 2.0 * EAR - 8.0;
    let slab = parts::chamfered_slab(strip_wide, STRIP_TALL, STRIP_UP, 1.2, 4.0);
    put(slab, half_wide, STRIP_Y, 0.0, strip_metal);
    let vent = vent();
    let Strip {
      in_meter,
      in_name,
      in_vent,
      logo,
      out_vent,
      out_name,
      out_meter,
      mix_name,
      loud,
      ..
    } = strip();
    let name = slanted(lit_legend("CANWETALK", 13.0), 0.2);
    put(name, middle(logo), STRIP_Y - 2.0, STRIP_UP, lit(1.0));
    let underline = [
      Vec2::new(logo[0], STRIP_Y + 9.5),
      Vec2::new(logo[1] - 4.0, STRIP_Y + 9.5),
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
    for span in [in_vent, out_vent] {
      put(vent.slots(), middle(span), STRIP_Y, STRIP_UP, hole);
      put(vent.holes(), middle(span), STRIP_Y, STRIP_UP, lit(0.55));
    }
    let meter = meter();
    let meter_seat = Arc::new(meter.seat());
    let meters = [in_meter, out_meter].map(|span| {
      Mat4::from_translation(Vec3::new(span[0], STRIP_Y, STRIP_UP))
        * Mat4::from_rotation_z(FRAC_PI_2)
    });
    let names = METER_NAMES.into_iter().zip([in_name, out_name]);
    for (name, span) in names.chain([(Control::Mix.name(), mix_name)]) {
      put(
        lit_legend(name, STRIP_LETTERS),
        middle(span),
        STRIP_Y,
        STRIP_UP,
        lit(LABEL_LIT),
      );
    }
    let meter_seats: Vec<Object> = (meters.iter())
      .flat_map(|place| {
        meter.seats(&meter_seat, 0.0, 0.0).map(|seat| Object {
          transform: *place * seat.transform,
          ..seat
        })
      })
      .collect();
    put(
      parts::led_seat(LOUD_LIGHT_SIZE),
      middle(loud),
      STRIP_Y,
      STRIP_UP,
      hole,
    );
    fixed.extend(meter_seats);

    let arc = knob_arc();
    Self {
      spots,
      fixed,
      shift_knob: knob_parts(shift_knob(), true),
      selector_knob: knob_parts(selector_knob(), false),
      small_knob: knob_parts(small_knob(), true),
      trim_knob: knob_parts(trim_knob(), true),
      plunger: Arc::new(switch.plunger()),
      switch,
      ring_light: Arc::new(ring.segment()),
      ring,
      arc_meshes: [Arc::new(arc.seat()), Arc::new(arc.lens())],
      arc,
      arcs: Control::ALL.map(|_| None).into(),
      stop_light: Arc::new(stop.light()),
      loud_light: Arc::new(parts::led(LOUD_LIGHT_SIZE)),
      loud: Vec2::new(middle(loud), STRIP_Y),
      meter_light: Arc::new(meter.segment()),
      meter,
      meters,
      glass: Arc::new(Mesh::cuboid(
        (-WINDOW / 2.0).extend(0.0),
        (WINDOW / 2.0).extend(GLASS_UP),
      )),
      readouts: Control::ALL.map(|_| None).into(),
      dimmed,
      display: envelope_display.expect("a field for the envelope"),
      shape: None,
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
    let envelope_light = if plays_envelope(&controls[Control::Mode as usize]) {
      1.0
    } else {
      DIMMED
    };
    for (mesh, at, level) in &self.dimmed {
      drawn.put(mesh, *at, lit(level * envelope_light), None);
    }
    for (index, (control, spot)) in Control::ALL.into_iter().zip(self.spots).enumerate() {
      let Vec2 { x, y } = spot.middle();
      let reading = &controls[index];
      let target = Some(Target::Control(index));
      let light = if spot.envelope { envelope_light } else { 1.0 };
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
            .filter(|kept| kept.of == [run[0], run[1], light])
            .unwrap_or_else(|| Kept {
              of: [run[0], run[1], light],
              objects: (self.arc.lights(&self.arc_meshes, x, y, run, CYAN * light)).collect(),
            });
          drawn.extend(kept.objects.iter().cloned());
          self.arcs[index] = Some(kept);
          &self.small_knob
        }
        Kind::Selector { choices, .. } => {
          let stops = selector_stops(choices);
          drawn.extend(stops.lights(&self.stop_light, x, y, reading.value, CYAN));
          &self.selector_knob
        }
        Kind::Trim => &self.trim_knob,
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
      // the strip's knob stands on the strip
      let lift = if control.kind() == Kind::Trim {
        STRIP_UP
      } else {
        0.0
      };
      let transform = Mat4::from_translation(Vec3::Z * lift)
        * parts::knob_transform(x, y, reading.value, parts::KNOB_SWEEP);
      for (mesh, material) in knob {
        drawn.place(mesh, transform, *material, target);
      }
      if control.kind().has_window() {
        let edit = edit.filter(|edit| edit.control == index);
        let Reading { text, unit, .. } = reading;
        self.readout(&mut drawn, index, spot.window(), (text, unit), edit, light);
      }
    }

    let Vec2 { x, y } = SCREEN_AT;
    let glass = screen_glass();
    let looked = chosen::<Detected>(&controls[Control::Detect as usize]).hz();
    let shown = (self.shown.take())
      .filter(|shown| shown.of.0 == feed && shown.of.1 == looked)
      .unwrap_or_else(|| Kept {
        of: (feed.to_vec(), looked),
        objects: parts::picture(&spectrum(feed, looked), x, y, glass, self.grid),
      });
    drawn.extend(shown.objects.iter().cloned());
    self.shown = Some(shown);
    lamps.extend(self.screen.lamps(x, y, CYAN, 0.3));

    let times = [
      Control::Attack,
      Control::Decay,
      Control::Sustain,
      Control::Release,
    ];
    let [attack, decay, sustain, release] = times.map(|control| controls[control as usize].value);
    let of = [attack, decay, sustain, release, envelope_light];
    let display = self.display;
    let Vec2 { x, y } = display.at;
    let shape = (self.shape.take())
      .filter(|shape| shape.of == of)
      .unwrap_or_else(|| Kept {
        of,
        objects: parts::picture(
          &envelope_shape(display, [attack, decay, sustain, release], envelope_light),
          x,
          y,
          screen_glass(),
          0,
        ),
      });
    drawn.extend(shape.objects.iter().cloned());
    self.shape = Some(shape);
    lamps.extend((display.screen()).lamps(x, y, CYAN, 0.2 * envelope_light));

    let levels = [INPUT_LEVEL, OUTPUT_LEVEL].map(|slot| meter_level(feed[slot]));
    for (place, level) in self.meters.iter().zip(levels) {
      let lights = (self.meter).lights(&self.meter_light, 0.0, 0.0, level, None);
      drawn.extend(lights.map(|light| Object {
        transform: *place * light.transform,
        ..light
      }));
    }
    // the light is as bright as the output is loud
    let [_, loud] = levels;
    drawn.put(
      &self.loud_light,
      self.loud.extend(STRIP_UP),
      parts::led_material(CYAN, loud, LOUD_LIGHT_SIZE),
      None,
    );
    let over = self.loud.extend(STRIP_UP + LOUD_LIGHT_SIZE);
    lamps.push(parts::led_lamp(
      over,
      CYAN,
      loud,
      LOUD_LIGHT_SIZE,
      LOUD_LIGHT_SIZE * 5.0,
    ));
    let scene = Scene {
      size: Vec2::new(WIDTH, HEIGHT),
      heights: [-8.0, 48.0],
      lean: LEAN,
      // under a dim wide light, which leaves what glows to show
      light: Light {
        color: Vec3::splat(0.6),
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

  /// a control's window: smoked glass sunk in the panel, with its value and unit lit on it at
  /// `light` of their full light.
  /// while the value is typed the glass is lit too and shows what has been typed, with an
  /// amber caret in it or, for as long as all of it is selected, dark on an amber block
  fn readout(
    &mut self,
    drawn: &mut Drawn,
    index: usize,
    at: Vec3,
    (figures, unit): (&str, &str),
    edit: Option<&Edit>,
    light: f32,
  ) {
    let target = Some(Target::Readout(index));
    // smoked glass, a dimmer mirror than clear
    let glass = Material {
      clearcoat: 0.4,
      ..Material::pane(FACE)
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
    let figure_light = Material::lamp(CYAN * light);
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

/// the face of a screen, which what it shows is lit on
fn screen_glass() -> Material {
  Material {
    // smoked glass, which mirrors less of the room than clear
    clearcoat: 0.3,
    ..Material::display_glass(FACE, 1.5)
  }
}

/// whether the mode a reading of the mode's control is at plays notes through the envelope
fn plays_envelope(mode: &Reading) -> bool {
  chosen::<Mode>(mode).is_gated()
}

/// the choice a reading of a selector is at
fn chosen<E: Enum>(reading: &Reading) -> E {
  let last = E::variants().len() - 1;
  E::from_index((reading.value * last as f32).round() as usize)
}

/// the envelope's shape on its screen, lit at `light` of its full light.
/// the attack, the decay and the release each run across as far as their knobs are turned,
/// from none at the start of a knob's turn to a third of the screen at its end, with the
/// sustain held at its level between the decay and the release
fn envelope_shape(
  Display { at, size }: Display,
  [attack, decay, sustain, release]: [f32; 4],
  light: f32,
) -> Flat {
  let inside = size / 2.0 - 5.0;
  let [left, right] = [at.x - inside.x, at.x + inside.x];
  let [top, foot] = [at.y - inside.y, at.y + inside.y];
  // one turn of a time's knob, so that all three at the ends of their turns and the hold fill
  // the screen
  let turn = (right - left) / (3.0 + HOLD);
  let held = foot - sustain * (foot - top);
  let mut x = left;
  let mut points = vec![[x, foot]];
  for (run, y) in [(attack, top), (decay, held), (HOLD, held), (release, foot)] {
    x += run * turn;
    points.push([x, y]);
  }
  let dim = |[red, green, blue, cover]: Color| [red * light, green * light, blue * light, cover];
  let mut flat = Flat::default();
  flat.line([left, foot], [right, foot], 1.0, dim(GRID));
  flat.area(&points, foot, dim(FILL));
  flat.curve(&points, 1.5, dim(EDGE));
  flat
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

/// what the screen shows of the feed: over the range of pitches detection looks for, the level
/// of each band of the input's spectrum as a curve over a fill, the output's as a curve over that, and a line up at the detected pitch
/// and at each pitch a voice makes. beside them are the note nearest the detected pitch, how
/// many cents off it the pitch is and the pitch itself, and under those the notes the latest
/// voices make, with a count of the others. a note there is none of is two dashes
fn spectrum(feed: &[f32], [lowest, highest]: [f32; 2]) -> Flat {
  let mut flat = Flat::default();
  // the range of pitches detection looks for, from `lowest` to `highest` hertz: the lines within
  // it lit brighter over those under, and a line at each end
  let [low, high] = [HERTZ.turn(lowest), HERTZ.turn(highest)];
  for hz in HERTZ_LINES {
    let across = HERTZ.turn(hz);
    if (low..=high).contains(&across) {
      flat.line(
        plotted([across, 0.0]),
        plotted([across, 1.0]),
        1.0,
        RANGE_GRID,
      );
    }
  }
  for level in LEVEL_LINES {
    let up = LEVELS.turn(level);
    flat.line(plotted([low, up]), plotted([high, up]), 1.0, RANGE_GRID);
  }
  for end in [low, high] {
    flat.line(plotted([end, 0.0]), plotted([end, 1.0]), 1.0, RANGE_EDGE);
  }
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

  fn settled(&self) -> bool {
    self.renderer.settled()
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
    let flat = spectrum(&feed, Detected::Mid.hz());
    let edge = SCREEN_AT.x + SCREEN_SIZE.x / 2.0;
    let mut pitch_lines = 0;
    for shape in flat.shapes() {
      if let Shape::Line {
        from, to, color, ..
      } = shape
      {
        for [x, _] in [from, to] {
          assert!((PLOT[0].x..=edge).contains(x), "a line at {x}");
        }
        let up = from[1] == PLOT[1].y && to[1] == PLOT[0].y;
        pitch_lines += usize::from(up && [PITCH, MADE_PITCH].contains(color));
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
    let groups: Vec<&Group> = (ROWS.iter().flat_map(|row| row.iter()))
      .flat_map(|column| column.iter())
      .collect();
    let grouped: usize = groups.iter().map(|group| group.controls.len()).sum();
    // and the mix on the strip
    assert_eq!(grouped + 1, Control::ALL.len());

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
    // lines beside the name, or the envelope's screen, and the rim at the foot, and its name
    // no wider than its slot
    for group in groups {
      let field = (fields.iter())
        .find(|field| field.name == group.name)
        .expect("a field for each group");
      let foot = field.down[1] - FIELD_BORDER;
      let over = match field.display {
        Some(display) => {
          // the screen lies in the field, under the drops of the lines
          let (at, half) = (display.at, display.size / 2.0 + DISPLAY_BORDER);
          assert!(at.x - half.x >= field.across[0] + FIELD_BORDER);
          assert!(at.x + half.x <= field.across[1] - FIELD_BORDER);
          assert!(at.y - half.y > field.y + RULE_DROP);
          at.y + half.y
        }
        None => field.y + RULE_DROP,
      };
      assert_eq!(field.display.is_some(), group.envelope, "{}", group.name);
      for &control in group.controls {
        let (kind, spot) = (control.kind(), spots[control as usize]);
        let [up, down] = kind.reach();
        let half = kind.slot() / 2.0;
        assert!(
          spot.x - half >= field.across[0] && spot.x + half <= field.across[1],
          "{control:?}"
        );
        assert!(spot.row.middle - up > over, "{control:?}");
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
    let screen_field = (fields.iter())
      .find(|field| field.name == SCREEN_NAME)
      .expect("a field for the screen");
    let half = SCREEN_SIZE / 2.0 + screen.border;
    let [left, right] = screen_field.across;
    assert!(SCREEN_AT.x - half.x >= left + FIELD_BORDER);
    assert!(SCREEN_AT.x + half.x <= right - FIELD_BORDER);
    assert!(SCREEN_AT.y - half.y > screen_field.y + RULE_DROP);
    assert!(SCREEN_AT.y + half.y <= screen_field.down[1] - FIELD_BORDER);
  }
}
