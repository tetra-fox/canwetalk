//! where the cycles of the input start. a mark is stepped on a period at a time to where the
//! waveform around it best matches the waveform around the mark before, so the marks stay at
//! one point of the cycle however the pitch moves, which a running estimate of the period
//! does not

use crate::ring::Ring;

/// how far from one period after the last mark the next is looked for, in periods. far
/// enough for the fastest slide of a voice from one cycle to the next and no further, since a
/// match further off is a wrong one
const REACH: f64 = 0.06;
/// how far the marks' period may be from the detected one, as a part of it, before the marks
/// are taken to be following a pitch that is gone
const AGREE: f64 = 0.15;
/// how much of a period either side of a mark is matched. a whole period between the two
/// sides, so that the loud part of the cycle is always among what is matched
const HALF_WINDOW: f64 = 0.5;
/// correlation under which the waveform is taken not to repeat, and the mark runs on by the
/// detected period
const MATCH: f32 = 0.5;
/// how much of the way a mark moves each period toward the middle of the period's energy, so
/// that a grain's window has the bulk of a cycle under its own middle. the first mark of a
/// run goes all the way, and the rest little enough that the pitch does not move with them
const PULL: f64 = 0.02;

pub(crate) struct Marks {
  /// samples since the last mark
  age: f64,
  /// the distance between the last two marks while the waveform repeats
  period: Option<f64>,
  /// scratch, a score for each lag looked at
  scores: Vec<f32>,
}

/// the input's cycle as the grains go by it
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cycle {
  /// in cycles per two samples
  pub(crate) pitch: f64,
  /// how far through its cycle the input is, 0 to 1, where the cycles are marked. otherwise
  /// the grains count it off by the pitch themselves
  pub(crate) phase: Option<f64>,
}

impl Marks {
  /// `longest_period` is in samples
  pub(crate) fn new(longest_period: f64) -> Self {
    Self {
      age: 0.0,
      period: None,
      scores: vec![0.0; 2 * (REACH * longest_period).ceil() as usize + 3],
    }
  }

  pub(crate) fn reset(&mut self) {
    self.age = 0.0;
    self.period = None;
  }

  /// takes in the sample just pushed to `input` and gives the period in samples and how far
  /// through its cycle the input is. `detected` is the period the detector has, which says
  /// where to look
  pub(crate) fn tick(&mut self, input: &[Ring; 2], detected: f64) -> (f64, f64) {
    self.age += 1.0;
    // marks that have come away from the detected period follow a pitch that is gone
    let followed = self
      .period
      .filter(|period| (period / detected - 1.0).abs() <= AGREE);
    let expected = followed.unwrap_or(detected);
    let reach = (REACH * expected)
      .ceil()
      .min(((self.scores.len() - 1) / 2) as f64);
    let half = (HALF_WINDOW * expected).ceil();
    // the samples either side of the next mark have to be there to be matched
    if self.age >= expected + reach + half {
      let step = self.step(input, expected, reach as usize, half as usize);
      let pull = if self.period.is_some() { PULL } else { 1.0 };
      self.period = step;
      self.age -= step.unwrap_or(expected);
      if step.is_some() {
        self.age -= pull * energy_offset(input, self.age, half as usize);
      }
    }
    let period = self.period.unwrap_or(detected);
    (period, (self.age / period).rem_euclid(1.0))
  }

  /// the distance from the last mark to where the waveform best matches the waveform around
  /// it, if it matches anywhere near `expected`
  fn step(&mut self, input: &[Ring; 2], expected: f64, reach: usize, half: usize) -> Option<f64> {
    let mono = |ago: usize| input[0].ago(ago) + input[1].ago(ago);
    let last = self.age.round() as usize;
    let nearest = expected.round() as usize;
    let around = |centre: usize| (centre - half..=centre + half).map(mono);
    let energy = |centre: usize| around(centre).map(|x| x * x).sum::<f32>();
    let last_energy = energy(last);

    let scores = &mut self.scores[..2 * reach + 1];
    for (score, lag) in scores.iter_mut().zip(nearest - reach..) {
      let product: f32 = around(last)
        .zip(around(last - lag))
        .map(|(a, b)| a * b)
        .sum();
      let norm = (last_energy * energy(last - lag)).sqrt();
      *score = if norm > 0.0 { product / norm } else { 0.0 };
    }
    let best = (0..scores.len())
      .max_by(|&a, &b| scores[a].total_cmp(&scores[b]))
      .expect("there are 2 * reach + 1 lags");
    if scores[best] < MATCH {
      return None;
    }
    let mut step = (nearest - reach + best) as f64;
    if best > 0 && best + 1 < scores.len() {
      // vertex of the parabola through the best score and its neighbours
      let (a, b, c) = (scores[best - 1], scores[best], scores[best + 1]);
      let curvature = a - 2.0 * b + c;
      if curvature < 0.0 {
        step += f64::from(0.5 * (a - c) / curvature);
      }
    }
    Some(step)
  }
}

/// how many samples before the mark of `age` the middle of the energy around it lies
fn energy_offset(input: &[Ring; 2], age: f64, half: usize) -> f64 {
  let centre = age.round() as usize;
  let (mut moment, mut energy) = (0.0, 0.0);
  for ago in centre.saturating_sub(half)..=centre + half {
    let power = (input[0].ago(ago) + input[1].ago(ago)).powi(2);
    moment += (ago as f32 - centre as f32) * power;
    energy += power;
  }
  if energy > 0.0 {
    f64::from(moment / energy)
  } else {
    0.0
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// a pulse train whose period glides, with a resonance on it. the marks are told a period
  /// a twenty-fifth off, inside what they reach, and still step by the true one
  #[test]
  fn marks_step_by_the_true_period() {
    let mut input = [Ring::new(8192), Ring::new(8192)];
    let mut marks = Marks::new(2000.0);
    let (mut phase, mut y1, mut y2) = (0.0, 0.0, 0.0);
    let mut worst = 0f64;
    for index in 0..48_000 {
      let true_period = 200.0 + 40.0 * (index as f64 / 48_000.0);
      phase += 1.0 / true_period;
      let pulse = if phase >= 1.0 {
        phase -= 1.0;
        1.0
      } else {
        0.0
      };
      let y: f64 = pulse + 1.8 * y1 - 0.9 * y2;
      (y2, y1) = (y1, y);
      for line in &mut input {
        line.push(y as f32);
      }
      let (period, _) = marks.tick(&input, 1.04 * true_period);
      if index > 4800 {
        worst = worst.max((period / true_period - 1.0).abs());
      }
    }
    assert!(worst < 0.01, "the marked period was {worst} off at worst");
  }
}
