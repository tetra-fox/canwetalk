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
/// how much of the way a mark moves each period away from the middle of the energy around
/// it. the first mark of a run moves the whole distance. the marks settle on the quiet part
/// of the cycle, half a period from the loud part
const PULL: f64 = 0.02;
/// lags a search scores before it takes to scoring every few
const MAX_LAGS: usize = 256;
/// samples a match compares before it takes to comparing every few
const MAX_COMPARED: usize = 512;
/// lags scored side by side
const LANES: usize = 8;

pub(crate) struct Marks {
  /// samples since the last mark
  age: f64,
  /// the distance between the last two marks while the waveform repeats
  period: Option<f64>,
  /// scratch, a score for each lag looked at
  scores: Vec<f32>,
  /// scratch, the sum of the input's channels over the stretch a search takes in
  window: Vec<f32>,
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
    // the marks' own period can be `AGREE` past the longest detected one. a search takes in
    // a step of that, the reach either way from it and a window, each rounded up
    let longest = (1.0 + AGREE) * longest_period;
    let taken_in = (1.0 + REACH + 2.0 * HALF_WINDOW) * longest;
    Self {
      age: 0.0,
      period: None,
      scores: vec![0.0; 2 * (REACH * longest_period).ceil() as usize + 3],
      window: vec![0.0; taken_in.ceil() as usize + 5],
    }
  }

  pub(crate) fn reset(&mut self) {
    self.age = 0.0;
    self.period = None;
  }

  /// takes in the sample just pushed to `input` and gives the period in samples and how far
  /// through its cycle the input is, while its waveform repeats. `detected` is the period the
  /// detector has, which says where to look
  pub(crate) fn tick(&mut self, input: &[Ring; 2], detected: f64) -> Option<(f64, f64)> {
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
    let period = self.period?;
    Some((period, (self.age / period).rem_euclid(1.0)))
  }

  /// the distance from the last mark to where the waveform best matches the waveform around
  /// it, if it matches anywhere near `expected`
  fn step(&mut self, input: &[Ring; 2], expected: f64, reach: usize, half: usize) -> Option<f64> {
    let last = self.age.round() as usize;
    let nearest = expected.round() as usize;
    // a match is made in the time of one sample, which a long period has too many samples
    // and lags for. past `MAX_COMPARED` every few samples are compared, and past `MAX_LAGS`
    // every few lags are scored and then all of those around the best
    let (mut first, mut end) = (nearest - reach, nearest + reach);
    let stride = (end - first + 1).div_ceil(MAX_LAGS);
    let stretch = Stretch {
      at: end,
      every: (2 * half + 1).div_ceil(MAX_COMPARED),
      count: 2 * half,
    };
    // the input from the newest sample a match takes in, half a window after the longest
    // step, back to the oldest, half a window before the last mark. lanes past the lags
    // looked at read on beyond that, which a short period has not the samples for
    let newest = last - end - half;
    let width = end.max(2 * reach + (LANES - 1) * stride) + 2 * half + 1;
    let window = &mut self.window[..width];
    for (sample, ago) in window.iter_mut().zip(newest..) {
      *sample = input[0].ago(ago) + input[1].ago(ago);
    }

    if stride > 1 {
      let scores = &mut self.scores[..(end - first) / stride + 1];
      stretch.score(window, scores, first, stride);
      let best = (0..scores.len())
        .max_by(|&a, &b| scores[a].total_cmp(&scores[b]))
        .expect("there is a lag in reach");
      let rough = first + best * stride;
      (first, end) = (
        rough.saturating_sub(stride).max(first),
        (rough + stride).min(end),
      );
    }
    let scores = &mut self.scores[..end - first + 1];
    stretch.score(window, scores, first, 1);
    let best = (0..scores.len())
      .max_by(|&a, &b| scores[a].total_cmp(&scores[b]))
      .expect("there is a lag to score");
    if scores[best] < MATCH {
      return None;
    }
    let mut step = (first + best) as f64;
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

/// the samples around the last mark that a search matches, in a window of the input that
/// starts with its newest sample
struct Stretch {
  /// where in the window the stretch starts. the stretch a lag before the mark starts that
  /// many samples sooner
  at: usize,
  /// every this many samples one is compared
  every: usize,
  /// samples from the first compared to the last that could be
  count: usize,
}

impl Stretch {
  /// how alike the stretch around the mark is to the one each lag before it, 1 for
  /// identical. the lags are `apart` apart from `first` up, one for each of `scores`. they
  /// are summed `LANES` side by side, each in the order it would be alone, since one sum has
  /// to wait for each of its additions and many can wait together
  fn score(&self, window: &[f32], scores: &mut [f32], first: usize, apart: usize) {
    let compared = || (0..=self.count).step_by(self.every);
    let energy: f32 = compared()
      .map(|index| window[self.at + index].powi(2))
      .sum();
    for (chunk, scores) in scores.chunks_mut(LANES).enumerate() {
      // a lane for each lag, the longest first, which puts their stretches in order
      let longest = first + (chunk * LANES + scores.len() - 1) * apart;
      let (mut products, mut energies) = ([0f32; LANES], [0f32; LANES]);
      for index in compared() {
        let sample = window[self.at + index];
        let from = self.at - longest + index;
        let mut others = [0.0; LANES];
        if apart == 1 {
          others.copy_from_slice(&window[from..][..LANES]);
        } else {
          for (lane, other) in others.iter_mut().enumerate() {
            *other = window[from + lane * apart];
          }
        }
        for ((product, energy), other) in products.iter_mut().zip(&mut energies).zip(others) {
          *product += sample * other;
          *energy += other * other;
        }
      }
      for ((score, product), other_energy) in scores.iter_mut().rev().zip(products).zip(energies) {
        let norm = (energy * other_energy).sqrt();
        *score = if norm > 0.0 { product / norm } else { 0.0 };
      }
    }
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

  /// a pulse train whose period glides by a fifth, with a resonance on it. the marks are told
  /// a period a twenty-fifth off, inside what they reach, and still step by the true one.
  /// the second period is long enough that every few samples are compared, and the third
  /// that every few lags are scored first, as a low voice at a high sample rate is
  #[test]
  fn marks_step_by_the_true_period() {
    for first_period in [200.0_f64, 600.0, 4267.0] {
      let mut input = [Ring::new(65_536), Ring::new(65_536)];
      let mut marks = Marks::new(6400.0);
      // ringing forty samples to the cycle, short against the long periods as a voice's is
      // at a high sample rate. that is what a search too coarse would step over
      let radius = (-1.0_f64 / 40.0).exp();
      let feedback = 2.0 * radius * (std::f64::consts::TAU / 40.0).cos();
      let (mut phase, mut y1, mut y2) = (0.0, 0.0, 0.0);
      let mut worst = 0f64;
      let samples = 240 * first_period as usize;
      for index in 0..samples {
        let true_period = first_period * (1.0 + 0.2 * index as f64 / samples as f64);
        phase += 1.0 / true_period;
        let pulse = if phase >= 1.0 {
          phase -= 1.0;
          1.0
        } else {
          0.0
        };
        let y: f64 = pulse + feedback * y1 - radius * radius * y2;
        (y2, y1) = (y1, y);
        for line in &mut input {
          line.push(y as f32);
        }
        let marked = marks.tick(&input, 1.04 * true_period);
        if index > samples / 10 {
          let (period, _) = marked.expect("the waveform repeats");
          worst = worst.max((period / true_period - 1.0).abs());
        }
      }
      assert!(
        worst < 0.01,
        "a period of {first_period} was marked {worst} off at worst"
      );
    }
  }

  /// a search for the longest step there can be takes in the most input, and one for the
  /// shortest has lanes that read past the little it takes in. both stay inside the scratch
  #[test]
  fn searches_at_both_ends_fit_the_scratch() {
    let longest = 6400.0;
    let mut input = [Ring::new(65_536), Ring::new(65_536)];
    let mut state = 0x2545_f491_u32;
    for _ in 0..65_536 {
      state ^= state << 13;
      state ^= state >> 17;
      state ^= state << 5;
      for line in &mut input {
        line.push(state as f32 / u32::MAX as f32 - 0.5);
      }
    }
    // just inside how far the marks' period may be from the detected one
    let furthest = (1.0 + AGREE) * (1.0 - 1e-12);
    for (period, detected) in [(furthest * longest, longest), (4.0, 4.0)] {
      let mut marks = Marks::new(longest);
      marks.period = Some(period);
      marks.age = 3.0 * period;
      marks.tick(&input, detected);
      assert!(marks.age < 3.0 * period, "no search was made");
    }
  }
}
