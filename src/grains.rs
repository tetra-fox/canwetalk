//! pitch-synchronous granular resynthesis. grains two input periods long are overlap-added
//! at the output pitch, so the grain spacing sets the pitch and the speed each grain is read
//! at sets the formants
//!
//! frequencies are in cycles per two samples, as in `shifter`

use crate::last::Last;
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
/// samples a grain has to sound in a block for what it reads to be taken as runs of the
/// input. fewer are sooner read one by one
const SHORTEST_RUN: usize = 8;

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
  /// the ratio the pitch was last shifted by, for the octaves that was
  shift: Last,
  grains: Vec<Grain>,
}

/// what the grains of a voice work a block out in. nothing in it is kept for the next
pub(crate) struct Scratch {
  /// what the grains come to at each sample, for each channel
  sums: [[f32; BLOCK]; 2],
  /// for one grain at each sample it sounds: how far through its window it is and how
  /// loud that makes it, how far behind it reads, as whole samples and the part of one
  /// past them, and each of the four samples of each channel that it reads between
  phases: [f64; BLOCK],
  levels: [f32; BLOCK],
  delays: [f64; BLOCK],
  wholes: [usize; BLOCK],
  parts: [f32; BLOCK],
  taps: [[[f32; BLOCK]; 4]; 2],
}

impl Scratch {
  pub(crate) fn new() -> Self {
    Self {
      sums: [[0.0; BLOCK]; 2],
      phases: [0.0; BLOCK],
      levels: [0.0; BLOCK],
      delays: [0.0; BLOCK],
      wholes: [0; BLOCK],
      parts: [0.0; BLOCK],
      taps: [[[0.0; BLOCK]; 4]; 2],
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
      shift: Last::default(),
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
    let mut gone = [0u16; BLOCK + 1];
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
      sounding -= usize::from(gone[index]);
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
    let shift = (self.shift).answer(shape.pitch + drift, |octaves| 2f64.powf(octaves));
    let pitch = (shift * target).clamp(*OUTPUT_PITCH.start(), *OUTPUT_PITCH.end());

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

/// the terms of the sine of x quarter turns, in odd powers of x from the first up
const QUARTER_TURN_SINE: [f64; 11] = [
  std::f64::consts::FRAC_PI_2,
  -0.6459640975062463,
  0.07969262624616705,
  -0.004681754135318688,
  0.00016044118478735983,
  -3.598843235212085e-6,
  5.692172921967927e-8,
  -6.688035109811468e-10,
  6.0669357311061955e-12,
  -4.377065467313742e-14,
  2.571422892860474e-16,
];

/// a raised cosine `quarters` quarter turns from its foot, `0.5 - 0.5 * cos(PI * quarters)`.
/// taken as the square of the sine of half the angle, with the sine summed from its series,
/// at a fraction of what the math library's cosine costs
fn raised_cosine(quarters: f64) -> f64 {
  let square = quarters * quarters;
  // in pairs, and pairs of pairs, which are all worked out at once where a sum term by
  // term has to wait for each
  let [a, b, c, d, e, f, g, h, i, j, k] = QUARTER_TURN_SINE;
  let (fourth, eighth) = (square * square, (square * square) * (square * square));
  let low = (a + b * square) + fourth * (c + d * square);
  let middle = (e + f * square) + fourth * (g + h * square);
  let high = (i + j * square) + fourth * k;
  let sine = quarters * (low + eighth * (middle + eighth * high));
  sine * sine
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
    // what a phase is multiplied by for how far it is through each half of the window, in
    // place of a division for every sample
    let (up, down) = (1.0 / self.peak, 1.0 / (1.0 - self.peak));
    for (level, &phase) in scratch.levels[..sounded].iter_mut().zip(&scratch.phases) {
      // half a raised cosine up to the peak and half one down from it
      let quarters = if phase < self.peak {
        phase * up
      } else {
        1.0 - (phase - self.peak) * down
      };
      *level = raised_cosine(quarters) as f32;
    }
    // at the input's own speed every read is as far behind as the first, and each of the
    // samples they go by is a run of the input
    let along = self.slope == 0.0 && sounded >= SHORTEST_RUN;
    let (wholes, parts) = (
      &mut scratch.wholes[..sounded],
      &mut scratch.parts[..sounded],
    );
    if along {
      (wholes[0], parts[0]) = split(scratch.delays[0]);
      parts.fill(parts[0]);
    } else {
      for ((whole, part), &delay) in wholes.iter_mut().zip(parts.iter_mut()).zip(&scratch.delays) {
        (*whole, *part) = split(delay);
      }
      for (index, &whole) in wholes.iter().enumerate() {
        for (line, taps) in input.iter().zip(&mut scratch.taps) {
          for (tap, sample) in taps.iter_mut().zip(line.taps(newer - index, whole)) {
            tap[index] = sample;
          }
        }
      }
    }
    for ((sums, line), taps) in (scratch.sums.iter_mut().zip(input)).zip(&mut scratch.taps) {
      let taps = taps.each_mut().map(|tap| &mut tap[..sounded]);
      let [a, b, c, d] = if along {
        line.taps_along(newer, wholes[0], taps)
      } else {
        taps.map(|tap| &*tap)
      };
      let sums = &mut sums[samples.start..][..sounded];
      let taps = a.iter().zip(b).zip(c).zip(d);
      for ((sum, (((&a, &b), &c), &d)), (part, level)) in
        (sums.iter_mut().zip(taps)).zip(parts.iter().zip(&scratch.levels))
      {
        *sum += level * cubic([a, b, c, d], *part);
      }
    }
    (!going).then(|| sounded - 1)
  }
}

#[cfg(test)]
mod tests {
  use std::f64::consts::PI;

  use super::*;

  /// the series is the raised cosine to a few parts in 1e16 over a window, a quarter turn
  /// either way. the first sample of a grain shorter than a sample can lie a little over
  /// two quarter turns before that, and is still within 1e-9 there
  #[test]
  fn the_series_is_a_raised_cosine() {
    let furthest = |quarters: std::ops::RangeInclusive<i32>| {
      let errors = quarters.map(|thousandths| {
        let quarters = f64::from(thousandths) / 1000.0;
        (raised_cosine(quarters) - (0.5 - 0.5 * (PI * quarters).cos())).abs()
      });
      errors.fold(0.0, f64::max)
    };
    assert!(furthest(0..=1000) < 1e-15, "{} off", furthest(0..=1000));
    assert!(furthest(-2300..=0) < 1e-9, "{} off", furthest(-2300..=0));
  }
}
