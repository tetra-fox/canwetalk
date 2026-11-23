//! the effects around the voices: smear on their input, detune and stereo on their sum

use std::f64::consts::PI;

use crate::ring::{Ring, cubic};

/// adds `step` to a phase kept in -1..1 and reports whether it wrapped
fn advance(phase: &mut f64, step: f64) -> bool {
  *phase += step;
  let wrapped = *phase >= 1.0;
  if wrapped {
    *phase -= 2.0;
  }
  wrapped
}

/// `phase` moved on by `offset`, still in -1..1
fn shifted(phase: f64, offset: f64) -> f64 {
  (phase + offset + 1.0).rem_euclid(2.0) - 1.0
}

/// how much of each effect there is, 0 to 1
#[derive(Clone, Copy, Debug)]
pub(crate) struct Amounts {
  pub(crate) detune: f64,
  pub(crate) smear: f64,
  pub(crate) stereo: f64,
}

const SMEAR_SECONDS: f32 = 11.0;
/// input periods in half a slow cycle with the control at zero
const SMEAR_SHORTEST: f64 = 6.0;
/// what the cube of the control is scaled by, for both the mix and the added length
const SMEAR_SCALE: f64 = 200.0;

/// freezes the input: two periods of it are looped from an anchor that moves on every few
/// periods, the higher the control the fewer. two such loops half a cycle apart cross over
/// so neither is heard while its anchor moves
pub(crate) struct Smear {
  lines: [Ring; 2],
  /// one cycle per two input periods, a loop restarts each half
  fast: f64,
  /// one cycle per anchor of each loop
  slow: f64,
  /// samples since each loop's anchor
  since_anchor: [usize; 2],
  /// per loop, the delay of the grain that restarts when `fast` wraps and of the one that
  /// restarts when it rises through zero
  delays: [[usize; 2]; 2],
}

impl Smear {
  pub(crate) fn new(sample_rate: f32) -> Self {
    let line = || Ring::new((sample_rate * SMEAR_SECONDS) as usize);
    Self {
      lines: [line(), line()],
      fast: 0.0,
      slow: 0.0,
      since_anchor: [0; 2],
      delays: [[0; 2]; 2],
    }
  }

  pub(crate) fn reset(&mut self) {
    for line in &mut self.lines {
      line.clear();
    }
    self.rest();
  }

  /// the most samples ago that what comes out can have gone in
  pub(crate) fn longest_delay(&self) -> usize {
    self.lines[0].len()
  }

  /// lets go of the stretches held, for while there is no smear and nothing goes into the
  /// lines. the loops then start from the input as it next comes, not from what the lines
  /// were left with
  pub(crate) fn rest(&mut self) {
    self.fast = 0.0;
    self.slow = 0.0;
    self.since_anchor = [0; 2];
    self.delays = [[0; 2]; 2];
  }

  /// `periods_per_sample` is the reciprocal of the input period
  pub(crate) fn process(
    &mut self,
    frame: [f32; 2],
    periods_per_sample: f64,
    amount: f64,
  ) -> [f32; 2] {
    let scaled = SMEAR_SCALE * amount.powi(3);
    let mix = scaled.min(1.0) as f32;
    // the grains of a loop cross over with a raised cosine, the loops with equal power
    let wrap_grain = (0.5 + 0.5 * (PI * self.fast).cos()) as f32;
    let grain_gains = [wrap_grain, 1.0 - wrap_grain];
    let loop_gains = [
      (0.5 * PI * self.slow).cos() as f32,
      (0.5 * PI * self.slow).sin().abs() as f32,
    ];

    let mut out = [0.0; 2];
    for ((out, line), dry) in out.iter_mut().zip(&mut self.lines).zip(frame) {
      line.push(dry);
      let mut wet = 0.0;
      for (delays, loop_gain) in self.delays.iter().zip(loop_gains) {
        for (&delay, grain_gain) in delays.iter().zip(grain_gains) {
          wet += loop_gain * grain_gain * line.ago(delay);
        }
      }
      *out = dry + mix * (wet - dry);
    }

    let fast_before = self.fast;
    let fast_wrapped = advance(&mut self.fast, periods_per_sample);
    let fast_rose = fast_before <= 0.0 && self.fast > 0.0;
    let slow_before = self.slow;
    // a loop reads back to its anchor, at most a slow cycle of two times this many periods
    // ago. on a low pitch the longest holds are cut to what the line holds
    let held = (self.lines[0].len() - 1) as f64;
    let periods = (SMEAR_SHORTEST + scaled.floor()).min(0.5 * held * periods_per_sample);
    let slow_wrapped = advance(&mut self.slow, periods_per_sample / periods);
    let slow_rose = slow_before <= 0.0 && self.slow > 0.0;
    // each loop's anchor moves while its gain is zero, one at the wrap and one at the rise
    for (since, moved) in self.since_anchor.iter_mut().zip([slow_wrapped, slow_rose]) {
      // the pitch can fall during a hold, which lengthens it past what was allowed for
      *since = if moved {
        0
      } else {
        (*since + 1).min(held as usize)
      };
    }
    for (delays, &since) in self.delays.iter_mut().zip(&self.since_anchor) {
      if fast_wrapped {
        delays[0] = since;
      }
      if fast_rose {
        delays[1] = since;
      }
    }
    out
  }
}

const DETUNE_CYCLE_SECONDS: f64 = 5.0;
/// the sweep of each of a channel's two delays at full detune, in seconds. the first grows
/// and lowers the pitch, the second shrinks and raises it
const DETUNE_SWEEPS: [[f64; 2]; 2] = [[0.15, 0.10], [0.075, 0.20]];
/// how far through the cycle each channel starts, in half cycles
const DETUNE_OFFSETS: [f64; 2] = [0.0, 0.5];

/// four slow pitch shifters, a lowered and a raised copy for each side. a shifter is a pair of
/// delays swept by a ramp half a cycle apart, each faded out while it jumps back
pub(crate) struct Detune {
  line: Ring,
  phase: f64,
}

impl Detune {
  pub(crate) fn new(sample_rate: f32) -> Self {
    let longest = DETUNE_SWEEPS
      .as_flattened()
      .iter()
      .copied()
      .fold(0.0, f64::max);
    let line = Ring::new((f64::from(sample_rate) * longest) as usize + 4);
    Self { line, phase: 0.0 }
  }

  pub(crate) fn reset(&mut self) {
    self.line.clear();
    self.phase = 0.0;
  }

  /// the most samples ago that what comes out can have gone in
  pub(crate) fn longest_delay(&self) -> usize {
    self.line.len()
  }

  /// `amount` is the control squared. at zero the frame comes back as it is, and still goes
  /// into the line for the copies to start from
  pub(crate) fn process(&mut self, frame: [f32; 2], amount: f64, sample_rate: f64) -> [f32; 2] {
    self.line.push(0.5 * (frame[0] + frame[1]));
    if amount == 0.0 {
      return frame;
    }
    advance(&mut self.phase, 2.0 / (DETUNE_CYCLE_SECONDS * sample_rate));
    // both copies at full level from a hundredth of the way up, half each at zero where
    // they coincide
    let gain = (0.5 + (50.0 * amount).min(0.5)) as f32;
    let mut out = [0.0; 2];
    for ((out, sweeps), offset) in out.iter_mut().zip(DETUNE_SWEEPS).zip(DETUNE_OFFSETS) {
      for half in [0.0, 1.0] {
        let phase = shifted(self.phase, offset + half);
        let ramp = 0.5 * phase + 0.5;
        // zero where the ramp jumps
        let fade = (1.0 - phase.abs()) as f32;
        let [lowering, raising] = sweeps.map(|sweep| sweep * sample_rate * amount);
        *out += fade * (self.line.read(ramp * lowering) + self.line.read((1.0 - ramp) * raising));
      }
      *out *= gain;
    }
    out
  }
}

/// the all-pass delay of each side at full stereo. they differ so the sides' phases differ
const STEREO_DELAY_SECONDS: [f64; 2] = [0.006, 0.006 * 1.37];
/// the all-pass gain at half stereo and above
const STEREO_FEEDBACK: f32 = 0.6;
/// the most samples a delay moves by in a sample, which is how far the pitch bends meanwhile
const STEREO_SLEW: f64 = 0.05;

/// widens by putting each side through an all-pass delay of its own length. each side keeps
/// the level of everything in it, only its phase is turned
pub(crate) struct Stereo {
  /// what goes round each side's loop, the input plus what comes back
  lines: [Ring; 2],
  /// each side's delay in samples. it moves to the whole number nearest what the amount
  /// asks for and rests there, since a read between samples loses highs each time round
  delays: [f64; 2],
}

impl Stereo {
  pub(crate) fn new(sample_rate: f32) -> Self {
    Self {
      lines: STEREO_DELAY_SECONDS
        .map(|seconds| Ring::new((f64::from(sample_rate) * seconds) as usize + 4)),
      delays: [0.0; 2],
    }
  }

  pub(crate) fn reset(&mut self) {
    for line in &mut self.lines {
      line.clear();
    }
    self.delays = [0.0; 2];
  }

  /// the most samples ago that what comes out can have gone in, to within 60 dB
  pub(crate) fn longest_delay(&self) -> usize {
    // each time round the loop takes what is in it down by the feedback
    let rounds = (-3.0 / STEREO_FEEDBACK.log10()).ceil() as usize;
    self.lines.iter().map(Ring::len).fold(0, usize::max) * rounds
  }

  /// at an `amount` of zero the frame comes back as it is once the delays have come down
  /// to nothing, and still goes into the lines for the loops to start from
  pub(crate) fn process(&mut self, frame: [f32; 2], amount: f64, sample_rate: f64) -> [f32; 2] {
    let feedback = STEREO_FEEDBACK * (2.0 * amount).min(1.0) as f32;
    let mut out = frame;
    for (((out, line), delay), seconds) in out
      .iter_mut()
      .zip(&mut self.lines)
      .zip(&mut self.delays)
      .zip(STEREO_DELAY_SECONDS)
    {
      let asked = (seconds * sample_rate * amount).round();
      let gap = asked - *delay;
      *delay = if gap.abs() <= STEREO_SLEW {
        asked
      } else {
        *delay + STEREO_SLEW.copysign(gap)
      };
      let input = *out;
      // `back` is what went round the loop `delay` samples ago
      let (round, back) = if *delay >= 1.0 {
        let back = line.read(*delay - 1.0);
        (input + feedback * back, back)
      } else {
        // under a sample, what comes back is partly what goes round now. the cubic of
        // `Ring::read` is linear in its taps, so the loop is solved for that part
        let part = *delay as f32;
        let [newest, next] = [line.ago(0), line.ago(1)];
        let now = cubic([2.0, 1.0, 0.0, 0.0], part);
        let before = cubic([-newest, 0.0, newest, next], part);
        let round = (input + feedback * before) / (1.0 - feedback * now);
        (round, now * round + before)
      };
      line.push(round);
      *out = back - feedback * round;
    }
    out
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const RATE: f32 = 48_000.0;

  fn tone(hz: f32) -> impl Iterator<Item = f32> {
    (0..RATE as usize).map(move |i| (std::f32::consts::TAU * (i as f32 * hz / RATE).fract()).sin())
  }

  fn power(samples: &[f32]) -> f32 {
    samples.iter().map(|x| x * x).sum::<f32>() / samples.len() as f32
  }

  /// each side is an all-pass, so a tone comes out of it as loud as it went in, whatever
  /// its pitch and however much stereo there is. the highest tones are a few of the dips
  /// apart that delays resting between samples would give
  #[test]
  fn stereo_keeps_the_level_of_a_tone_on_each_side() {
    let high = (0..5).map(|step| 15_000.0 + 50.0 * step as f32);
    let tones: Vec<f32> = [110.0, 3100.0].into_iter().chain(high).collect();
    for amount in [0.0, 0.001, 0.3, 0.7, 1.0] {
      for &hz in &tones {
        let mut stereo = Stereo::new(RATE);
        let (dry, wet): (Vec<f32>, Vec<[f32; 2]>) = tone(hz)
          .map(|x| (x, stereo.process([x, x], amount, f64::from(RATE))))
          .skip(RATE as usize / 2)
          .unzip();
        for side in 0..2 {
          let wet: Vec<f32> = wet.iter().map(|frame| frame[side]).collect();
          let ratio = power(&wet) / power(&dry);
          assert!(
            (0.98..1.02).contains(&ratio),
            "side {side} at {hz} Hz and {amount} is {ratio} of the power"
          );
        }
      }
    }
  }

  /// under a sample of delay the loop is solved for what goes round now. what it pushes
  /// and gives has to be what the loop's two sums come to with `Ring::read` at that delay
  #[test]
  fn stereo_under_a_sample_reads_what_it_pushes() {
    for start in [0.0, 0.2, 0.45, 0.7, 0.9] {
      let mut stereo = Stereo::new(RATE);
      for x in tone(3100.0).take(100) {
        stereo.process([x, -x], 0.0, f64::from(RATE));
      }
      stereo.delays = [start; 2];
      let frame = [0.3, -0.8];
      let out = stereo.process(frame, 1.0, f64::from(RATE));
      let delay = start + STEREO_SLEW;
      for ((line, input), out) in stereo.lines.iter().zip(frame).zip(out) {
        let (round, back) = (line.ago(0), line.read(delay));
        assert!((round - (input + STEREO_FEEDBACK * back)).abs() < 1e-6);
        assert!((out - (back - STEREO_FEEDBACK * round)).abs() < 1e-6);
      }
    }
  }

  /// a steady tone frozen and looped by whole periods is still that tone in phase. the two
  /// loops cross over with equal power, so where both sound it is up to sqrt(2) louder
  #[test]
  fn smear_keeps_a_steady_tone() {
    let hz = 200.0;
    let mut smear = Smear::new(RATE);
    let output: Vec<f32> = tone(hz)
      .map(|x| smear.process([x, x], f64::from(hz / RATE), 0.5)[0])
      .collect();
    for (dry, wet) in tone(hz).zip(output).skip(RATE as usize / 2) {
      if dry.abs() > 0.5 {
        let ratio = wet / dry;
        assert!((0.95..1.45).contains(&ratio), "wet is {ratio} of dry");
      }
    }
  }

  /// the lowest input pitch with the control at full holds a stretch for longer than anything
  /// else does, and every delay still has to be inside what the line holds
  #[test]
  fn smear_at_its_longest_stays_inside_its_line() {
    let rate = 44_100.0;
    let mut smear = Smear::new(rate);
    for index in 0..20 * rate as usize {
      smear.process([index as f32; 2], 25.0 / f64::from(rate), 1.0);
    }
  }

  /// each side is the mean of a lowered and a raised copy, by the ratios the sweeps give
  #[test]
  fn detune_shifts_by_the_sweep_rates() {
    let (hz, amount) = (1000.0, 1.0);
    let mut detune = Detune::new(RATE);
    let left: Vec<f32> = tone(hz)
      .map(|x| detune.process([x, x], amount, f64::from(RATE))[0])
      .collect();
    let power_at = |hz: f32| {
      let (re, im) = left
        .iter()
        .enumerate()
        .fold((0.0, 0.0), |(re, im), (i, &x)| {
          let angle = std::f32::consts::TAU * (hz * i as f32 / RATE).fract();
          (re + x * angle.cos(), im + x * angle.sin())
        });
      re * re + im * im
    };
    let per_second = |sweep: f64| (sweep / DETUNE_CYCLE_SECONDS) as f32;
    let [lowering, raising] = DETUNE_SWEEPS[0].map(per_second);
    let original = power_at(hz);
    assert!(
      power_at(hz * (1.0 - lowering)) > 20.0 * original,
      "no lowered copy"
    );
    assert!(
      power_at(hz * (1.0 + raising)) > 20.0 * original,
      "no raised copy"
    );
  }
}
