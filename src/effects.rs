//! the effects around the voices: smear on their input, detune and stereo on their sum

use std::f64::consts::PI;

use crate::ring::Ring;

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
    let slow_wrapped = advance(
      &mut self.slow,
      periods_per_sample / (SMEAR_SHORTEST + scaled.floor()),
    );
    let slow_rose = slow_before <= 0.0 && self.slow > 0.0;
    // each loop's anchor moves while its gain is zero, one at the wrap and one at the rise
    for (since, moved) in self.since_anchor.iter_mut().zip([slow_wrapped, slow_rose]) {
      *since = if moved { 0 } else { *since + 1 };
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

  /// `amount` is the control squared
  pub(crate) fn process(&mut self, frame: [f32; 2], amount: f64, sample_rate: f64) -> [f32; 2] {
    self.line.push(0.5 * (frame[0] + frame[1]));
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

/// gains of the four all-pass sections each channel goes through
const ALLPASS_GAINS: [f32; 4] = [0.692_387_8, 0.936_065_43, 0.988_229_5, 0.998_748_84];
const STEREO_DELAY_SECONDS: f64 = 0.006;

/// widens by adding a phase-shifted, delayed copy of each side to itself, with opposite signs
pub(crate) struct Stereo {
  channels: [StereoChannel; 2],
}

struct StereoChannel {
  /// the last two inputs and outputs of each all-pass section
  sections: [[f32; 4]; 4],
  line: Ring,
}

impl Stereo {
  pub(crate) fn new(sample_rate: f32) -> Self {
    let channel = || StereoChannel {
      sections: [[0.0; 4]; 4],
      line: Ring::new((f64::from(sample_rate) * STEREO_DELAY_SECONDS) as usize + 4),
    };
    Self {
      channels: [channel(), channel()],
    }
  }

  pub(crate) fn reset(&mut self) {
    for channel in &mut self.channels {
      channel.sections = [[0.0; 4]; 4];
      channel.line.clear();
    }
  }

  pub(crate) fn process(&mut self, frame: [f32; 2], amount: f64, sample_rate: f64) -> [f32; 2] {
    let gain = (2.0 * amount).min(1.0) as f32;
    let delay = STEREO_DELAY_SECONDS * sample_rate * amount;
    let mut out = frame;
    for ((out, channel), sign) in out.iter_mut().zip(&mut self.channels).zip([1.0, -1.0]) {
      let mut sample = *out;
      for (state, gain) in channel.sections.iter_mut().zip(ALLPASS_GAINS) {
        let [x1, x2, y1, y2] = *state;
        let y = gain * (sample + y2) - x2;
        *state = [sample, x1, y, y1];
        sample = y;
      }
      channel.line.push(sample);
      *out += sign * gain * channel.line.read(delay);
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

  #[test]
  fn stereo_leaves_the_sum_of_a_centred_signal_alone() {
    let mut stereo = Stereo::new(RATE);
    for x in tone(220.0) {
      let [left, right] = stereo.process([x, x], 0.8, f64::from(RATE));
      assert!((left + right - 2.0 * x).abs() < 1e-5);
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
