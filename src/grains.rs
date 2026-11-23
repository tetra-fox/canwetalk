//! pitch-synchronous granular resynthesis. grains two input periods long are overlap-added
//! at the output pitch, so the grain spacing sets the pitch and the speed each grain is read
//! at sets the formants
//!
//! frequencies are in cycles per two samples, as in `shifter`

use std::f64::consts::TAU;

use crate::marks::Cycle;
use crate::ring::{Ring, cubic, split};
use crate::shifter::BLOCK;

const MAX_GRAINS: usize = 128;
const OUTPUT_PITCH: std::ops::RangeInclusive<f64> = 1e-11..=0.25;
/// octaves the formants move at most, either way
const MAX_FORMANT: f64 = 4.0;
/// how much of the way a centred grain's rise moves to the input period with each grain
const RISE_FOLLOW: f64 = 0.01;
/// how far from the input period the rise may be, as a ratio, before it starts over from it
const RISE_RANGE: std::ops::RangeInclusive<f64> = 0.77..=1.3;
/// samples past a delay that reading between samples takes in
const READ_AHEAD: f64 = 4.0;
/// octaves a voice at the input pitch is raised by until its grains line up with the input
pub(crate) const UNITY_DRIFT: f64 = 0.001;
/// a grain starting within this many samples of an input period counts as lined up
const UNITY_REACH: f64 = 4.0;
/// how much of the way the grains fade in or out against the plain input each sample
const WET_STEP: f64 = 0.0125;

/// how the grains differ from the input
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Shape {
  /// octaves above the target pitch
  pub(crate) pitch: f64,
  /// octaves the formants are moved by
  pub(crate) formant: f64,
  /// octaves the formants move further for each octave the output pitch is from the input's
  pub(crate) formant_follow: f64,
  /// 0 to 1, how far grains stretch to keep overlapping when the pitch is lowered
  pub(crate) smooth: f64,
}

/// what a voice wants of its grains at one sample
#[derive(Clone, Copy)]
pub(crate) struct Wanted {
  /// the cycle the input is in
  pub(crate) cycle: Cycle,
  /// the pitch to sound at
  pub(crate) target: f64,
  pub(crate) shape: Shape,
}

/// the grains of one voice
pub(crate) struct Engine {
  /// position in the input period, 0 to 1
  input_phase: f64,
  /// position in the output period, -1 to 1. a grain starts each time it wraps
  output_phase: f64,
  /// the output pitch as of the last grain
  output_step: f64,
  /// whether the grains line up with the input periods and read it undelayed. a new engine
  /// counts both periods off from the same sample, so it starts out lined up
  locked: bool,
  /// how much of the output is grains, the rest is the plain input. none before the first
  /// sample, which has nothing to fade from
  wet: Option<f64>,
  /// input samples a grain centred on a cycle start takes to rise, which follows the input
  /// period slowly
  rise: Option<f64>,
  grains: Vec<Grain>,
}

/// what the grains of a voice work a block out in. nothing in it is kept for the next
pub(crate) struct Scratch {
  /// what the grains come to at each sample, for each channel
  sums: [[f32; BLOCK]; 2],
  /// for one grain at each sample it sounds: how far through its window it is and how
  /// loud that makes it, how far behind it reads, as a part of a sample past the whole
  /// ones, and the samples of each channel that it reads between
  phases: [f64; BLOCK],
  levels: [f32; BLOCK],
  delays: [f64; BLOCK],
  parts: [f32; BLOCK],
  taps: [[[f32; 4]; BLOCK]; 2],
}

impl Scratch {
  pub(crate) fn new() -> Self {
    Self {
      sums: [[0.0; BLOCK]; 2],
      phases: [0.0; BLOCK],
      levels: [0.0; BLOCK],
      delays: [0.0; BLOCK],
      parts: [0.0; BLOCK],
      taps: [[[0.0; 4]; BLOCK]; 2],
    }
  }
}

struct Grain {
  /// how far behind the newest input the grain reads, less `extra`
  delay: f64,
  extra: f64,
  /// change of `extra` per sample, which makes the read speed `1 - slope`
  slope: f64,
  window_phase: f64,
  window_step: f64,
  /// the window phase at which the grain is loudest, a half unless it is centred on a cycle
  /// start
  peak: f64,
}

impl Engine {
  pub(crate) fn new() -> Self {
    Self {
      input_phase: 0.0,
      output_phase: -1.0,
      output_step: 0.0,
      locked: true,
      wet: None,
      rise: None,
      grains: Vec::with_capacity(MAX_GRAINS),
    }
  }

  pub(crate) fn reset(&mut self) {
    self.grains.clear();
    *self = Self {
      grains: std::mem::take(&mut self.grains),
      ..Self::new()
    };
  }

  /// the voice over the samples of `out`, of each of which `wanted` says how it is to
  /// sound. `input` holds `after` samples that came after the last of them
  pub(crate) fn tick(
    &mut self,
    wanted: &[Wanted],
    input: &[Ring; 2],
    after: usize,
    scratch: &mut Scratch,
    out: &mut [[f32; 2]],
  ) {
    let (held, count) = (after + out.len(), out.len());
    for sums in &mut scratch.sums {
      sums[..count].fill(0.0);
    }
    // how many grains are gone by each sample, having ended at the one before. the grains
    // there are at the start sound on to their ends whatever comes, so each is taken to its
    // end or the block's in turn, and so is each that starts from where it does
    let mut gone = [0; BLOCK + 1];
    let mut sounding = self.grains.len();
    self.grains.retain_mut(|grain| {
      let last = grain.sound(input, held - 1, 0..count, scratch);
      if let Some(last) = last {
        gone[last + 1] += 1;
      }
      last.is_none()
    });
    for (index, (out, wanted)) in out.iter_mut().zip(wanted).enumerate() {
      let newer = held - 1 - index;
      sounding -= gone[index];
      if let Some(mut grain) = self.advance(wanted, input[0].len(), sounding) {
        sounding += 1;
        match grain.sound(input, newer, index..count, scratch) {
          Some(last) => gone[index + last + 1] += 1,
          None => self.grains.push(grain),
        }
      }

      // locked grains are at the input pitch, where the formants have nothing to follow
      let active = !(self.locked && wanted.shape.formant == 0.0);
      let goal = f64::from(u8::from(active));
      let wet = self.wet.map_or(goal, |wet| wet + WET_STEP * (goal - wet));
      self.wet = Some(wet);
      let wet = wet as f32;
      for ((out, sums), line) in out.iter_mut().zip(&scratch.sums).zip(input) {
        *out = wet * sums[index] + (1.0 - wet) * line.ago(newer);
      }
    }
  }

  /// moves the voice on by one sample, to sound as `wanted`, and gives the grain that
  /// starts there if one does. `sounding` is how many grains there are and `held` how many
  /// samples the input lines hold
  fn advance(&mut self, wanted: &Wanted, held: usize, sounding: usize) -> Option<Grain> {
    let Wanted {
      cycle,
      target,
      ref shape,
    } = *wanted;
    // at the input pitch the grains only rebuild the input, so they are moved onto the input
    // periods and then faded out for the input itself
    let input_pitch = cycle.pitch;
    let unity = target == input_pitch && shape.pitch == 0.0;
    if !unity && self.locked {
      self.locked = false;
      self.output_phase = 2.0 * self.input_phase - 1.0;
    }
    let drift = if unity && !self.locked {
      UNITY_DRIFT
    } else {
      0.0
    };
    let pitch =
      (2f64.powf(shape.pitch + drift) * target).clamp(*OUTPUT_PITCH.start(), *OUTPUT_PITCH.end());

    match cycle.phase {
      Some(phase) => self.input_phase = phase,
      None => {
        self.input_phase += 0.5 * input_pitch;
        if self.input_phase >= 1.0 {
          self.input_phase -= 1.0;
        }
      }
    }
    self.output_phase += self.output_step;
    let wrapped = self.output_phase >= 1.0;
    if wrapped {
      self.output_phase -= 2.0;
    }
    if wrapped || sounding == 0 {
      self.output_step = pitch;
    }
    if !wrapped || sounding >= MAX_GRAINS {
      return None;
    }

    let followed = shape.formant_follow * (pitch / input_pitch).log2();
    let rate = 2f64.powf((shape.formant + followed).clamp(-MAX_FORMANT, MAX_FORMANT));
    // back to where the input period began, so grains start whole periods apart
    let mut delay = self.input_phase / (0.5 * input_pitch);
    if unity && delay < UNITY_REACH {
      self.locked = true;
    }
    if self.locked {
      delay = 0.0;
    }
    // two input periods at the read speed. below that pitch grains would leave gaps, which
    // smooth closes by stretching them toward two output periods
    let natural = rate * input_pitch;
    let stretch = if pitch <= natural {
      natural / (shape.smooth * pitch + (1.0 - shape.smooth) * natural)
    } else {
      1.0
    };
    // a grain reads from as far back as it starts, and at a read speed off one that
    // distance grows or shrinks by the input it covers. all of it has to be input the lines
    // still hold, which a grain stretched over a very low output pitch would run past
    let period = 2.0 / input_pitch;
    let held = held as f64 - READ_AHEAD - period;
    let covered = held / (1.0 + (rate - 1.0).abs() / rate);
    let stretch = stretch.min(covered / ((1.0 + RISE_RANGE.end()) * period));
    let mut window_step = 0.25 * natural / stretch;
    let mut peak = 0.5;
    if cycle.phase.is_some() && !self.locked {
      // with the cycles marked the grain is centred on the start of one. it is loudest a
      // set time after it begins, so as the input period changes the loud parts of the
      // grains stay evenly spaced. a grain that began at a cycle start would be loudest one
      // input period later, and its pitch would give with the input's
      let rise = self
        .rise
        .filter(|rise| RISE_RANGE.contains(&(rise / period)))
        .unwrap_or(period);
      let rise = rise + RISE_FOLLOW * (period - rise);
      self.rise = Some(rise);
      let (before, after) = (rise * stretch, period * stretch);
      // the next cycle start if the rise reaches back from it to what is already here,
      // else the last
      let to_next = period - delay;
      delay = if to_next <= before {
        before - to_next
      } else {
        before + delay
      };
      window_step = rate / (before + after);
      peak = before / (before + after);
    }
    Some(Grain {
      delay,
      // reading faster than the input arrives needs a head start of what it will gain
      extra: if rate > 1.0 {
        (rate - 1.0) / window_step
      } else {
        0.0
      },
      slope: rate - 1.0,
      // the wrap fell between samples, the grain is that far in already
      window_phase: (self.output_phase + 1.0) / pitch * window_step,
      window_step,
      peak,
    })
  }
}

impl Grain {
  /// adds the grain to the sums of `scratch` over `samples`, a sample at a time until it
  /// ends, and gives how far into them it ended if it did. `input` holds `newer` samples
  /// that came after the first of them
  fn sound(
    &mut self,
    input: &[Ring; 2],
    newer: usize,
    samples: std::ops::Range<usize>,
    scratch: &mut Scratch,
  ) -> Option<usize> {
    // one thing at a time for all the samples, in loops short enough to run several
    // samples at once
    let (mut sounded, mut going) = (0, true);
    while going && sounded < samples.len() {
      scratch.phases[sounded] = self.window_phase;
      scratch.delays[sounded] = self.delay + self.extra;
      self.extra -= self.slope;
      self.window_phase += self.window_step;
      going = self.window_phase <= 1.0;
      sounded += 1;
    }
    for (level, &phase) in scratch.levels[..sounded].iter_mut().zip(&scratch.phases) {
      // half a raised cosine up to the peak and half one down from it
      let turn = if phase < self.peak {
        0.5 * phase / self.peak
      } else {
        0.5 + 0.5 * (phase - self.peak) / (1.0 - self.peak)
      };
      *level = (0.5 - 0.5 * (TAU * turn).cos()) as f32;
    }
    for (index, &delay) in scratch.delays[..sounded].iter().enumerate() {
      let (whole, part) = split(delay);
      scratch.parts[index] = part;
      for (line, taps) in input.iter().zip(&mut scratch.taps) {
        taps[index] = line.taps(newer - index, whole);
      }
    }
    for (sums, taps) in scratch.sums.iter_mut().zip(&scratch.taps) {
      let sums = &mut sums[samples.start..][..sounded];
      for (((sum, &taps), &part), level) in
        (sums.iter_mut().zip(taps).zip(&scratch.parts)).zip(&scratch.levels)
      {
        *sum += level * cubic(taps, part);
      }
    }
    (!going).then(|| sounded - 1)
  }
}
