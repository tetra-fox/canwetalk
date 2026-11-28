//! monophonic pitch detection. zero crossings of a heavily low-passed copy of the input say
//! when to look and roughly where, the correlation of the newest period with the one before
//! scores a lag, and once a lag is held only its neighbours are scored

use nice_plug::prelude::Enum;

/// the span of pitches looked for, which is also a parameter the host sets. a session keeps it
/// by its id, which leaves the order free to change
#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Range {
  #[id = "bass"]
  Bass,
  #[id = "mid"]
  Mid,
  #[id = "high"]
  High,
}

struct RangeSpec {
  /// every this many input samples one goes to the analysis, at `BASE_RATE`
  decimation: usize,
  lowest_hz: f32,
  highest_hz: f32,
  /// low-pass before the analysis
  analysis_hz: f32,
  /// low-pass before the zero crossings, which leaves little but the fundamental
  crossing_hz: f32,
}

impl Range {
  fn spec(self) -> RangeSpec {
    let spec = |decimation, lowest_hz, highest_hz, analysis_hz, crossing_hz| RangeSpec {
      decimation,
      lowest_hz,
      highest_hz,
      analysis_hz,
      crossing_hz,
    };
    match self {
      Self::Bass => spec(4, 25.0, 700.0, 8000.0, 200.0),
      Self::Mid => spec(2, 40.0, 1000.0, 8000.0, 400.0),
      Self::High => spec(2, 40.0, 2000.0, 16000.0, 800.0),
    }
  }

  /// the lowest and highest pitches the range looks for, in hertz
  pub(crate) fn hz(self) -> [f32; 2] {
    let spec = self.spec();
    [spec.lowest_hz, spec.highest_hz]
  }

  /// period of the lowest pitch of the range, in samples
  pub(crate) fn longest_period(self, sample_rate: f32) -> f32 {
    sample_rate / self.spec().lowest_hz
  }
}

/// the sample rate the decimations of the ranges are for, which 44.1 kHz counts as too
const BASE_RATE: f32 = 48_000.0;
/// high-pass before the zero crossings, so an offset does not hide them
const CROSSING_FLOOR_HZ: f32 = 20.0;
const FILTER_Q: f32 = 0.733;
const HISTORY: usize = 8192;
const MAX_CROSSINGS: usize = 32;
/// crossings from a fresh start to the first look, about two and a half periods
const CROSSINGS_TO_LOOK: usize = 6;
/// crossings between looks while none has found a pitch
const CROSSINGS_TO_RETRY: usize = 2;
/// the held lag is scored again every this many analysis samples
const TRACK_EVERY: usize = 5;
/// lags below and above the held one that tracking scores
const TRACK_BELOW: usize = 5;
const TRACK_ABOVE: usize = 4;
/// score a lag needs to be taken up
const LOCK_SCORE: f32 = 0.92;
/// score below which a held lag is dropped
const HOLD_SCORE: f32 = 0.7;
/// of the lag ranges the crossings suggest, the shortest scoring this much of the best wins
const SHORTER_WINS_ABOVE: f32 = 0.95;
/// percent of the later half that the two halves of a stretch of crossings have to differ by
/// less than, for the stretch to count as whole periods
const HALVES_PERCENT: u64 = 2;
/// the least a score compares, so that a short lag is compared over several of its periods.
/// noise, or a formant ringing after one pulse, can repeat for a period or two by chance
const SHORTEST_COMPARED_SECONDS: f32 = 0.005;
/// seconds a one-pole filter follows the held pitch by. an estimate is compared with that,
/// where the pitch has been, and not only with the last estimate
const STEADY_SECONDS: f32 = 0.01;
/// semitones an estimate may be from the followed pitch and still be the held pitch. a voice
/// moves further only from one sound to the next, and the best of the lags near the held one
/// can run off that fast where the input is barely periodic
const REACH_SEMITONES: f32 = 2.0;
/// seconds a lost pitch still bounds the next one taken up. a harmonic or a resonance can
/// repeat well enough to be taken the moment the fundamental under it fades, where a voice
/// moving to another note leaves a gap or slides
const REMEMBERED_SECONDS: f32 = 0.05;
/// mean magnitude of an analysis sample below which the input is taken for silence
const SILENCE_LEVEL: f32 = 5e-5;
/// lags scored side by side: the ones tracking scores and one either side of them
const LANES: usize = TRACK_BELOW + TRACK_ABOVE + 3;
/// analysis samples kept. two periods of the longest lag there is, and of a set of lanes
/// started on one of the lags tracking scores above it
const KEPT: usize = HISTORY + 2 * LANES;

/// second-order section, transposed direct form II
#[derive(Clone, Copy, Default)]
struct Biquad {
  b: [f32; 3],
  a: [f32; 2],
  state: [f32; 2],
}

impl Biquad {
  fn low_pass(hz: f32, sample_rate: f32) -> Self {
    Self::design(hz, sample_rate, |k, norm| {
      [k * k * norm, 2.0 * k * k * norm, k * k * norm]
    })
  }

  fn high_pass(hz: f32, sample_rate: f32) -> Self {
    Self::design(hz, sample_rate, |_, norm| [norm, -2.0 * norm, norm])
  }

  fn design(hz: f32, sample_rate: f32, numerator: impl Fn(f32, f32) -> [f32; 3]) -> Self {
    // the bilinear transform has no corner at or above half the sample rate
    let k = (std::f32::consts::PI * hz.min(0.45 * sample_rate) / sample_rate).tan();
    let norm = 1.0 / (1.0 + k / FILTER_Q + k * k);
    Self {
      b: numerator(k, norm),
      a: [
        2.0 * (k * k - 1.0) * norm,
        (1.0 - k / FILTER_Q + k * k) * norm,
      ],
      state: [0.0; 2],
    }
  }

  fn process(&mut self, x: f32) -> f32 {
    let y = self.b[0] * x + self.state[0];
    self.state[0] = self.b[1] * x - self.a[0] * y + self.state[1];
    self.state[1] = self.b[2] * x - self.a[1] * y;
    y
  }
}

/// a lag and the scores of the `LANES` lags from it up
type Scored = (usize, [f32; LANES]);

/// what is kept of a pitch taken up
#[derive(Clone, Copy)]
struct Held {
  /// the lag of the last estimate, in analysis samples
  lag: usize,
  /// octaves of the period the estimates are followed to by `STEADY_SECONDS`
  steady: f32,
  /// when `steady` was last moved
  at: u64,
}

pub(crate) struct Detector {
  sample_rate: f32,
  range: Range,
  decimation: usize,
  shortest_lag: usize,
  longest_lag: usize,
  /// `SHORTEST_COMPARED_SECONDS` in analysis samples
  shortest_compared: usize,
  /// `STEADY_SECONDS` and `REMEMBERED_SECONDS` in analysis samples
  steady_samples: f32,
  remembered_samples: u64,
  analysis_filter: Biquad,
  crossing_filters: [Biquad; 2],
  /// input samples until the next analysis sample
  until_tick: usize,
  /// analysis samples, each of them twice and `KEPT` apart, so that the newest `KEPT` are
  /// always in one piece
  history: Vec<f32>,
  /// where in the first `KEPT` of `history` the next sample goes
  next: usize,
  /// analysis samples so far
  now: u64,
  /// whether the crossing path was above zero at the last analysis sample
  positive: bool,
  /// times of the latest crossings, the oldest first
  crossings: [u64; MAX_CROSSINGS],
  crossing_count: usize,
  /// crossings until the next look, none while waiting for the first crossing of a fresh start
  until_look: Option<usize>,
  /// analysis samples until the crossings are given up on for lack of a new one
  patience: usize,
  /// sum of analysis sample magnitudes since the fresh start
  level: f32,
  until_track: usize,
  /// the pitch taken up, while there is one
  held: Option<Held>,
  /// the pitch last held, from when it was lost
  lost: Option<Held>,
  /// score the next estimate has to reach, LOCK_SCORE until one reaches it and HOLD_SCORE
  /// from then until one falls to HOLD_SCORE or under
  needed: f32,
}

impl Detector {
  pub(crate) fn new(sample_rate: f32, range: Range) -> Self {
    let mut detector = Self {
      sample_rate,
      range,
      decimation: 1,
      shortest_lag: 0,
      longest_lag: 0,
      shortest_compared: 0,
      steady_samples: 1.0,
      remembered_samples: 0,
      analysis_filter: Biquad::default(),
      crossing_filters: [Biquad::default(); 2],
      until_tick: 1,
      history: vec![0.0; 2 * KEPT],
      next: 0,
      now: 0,
      positive: false,
      crossings: [0; MAX_CROSSINGS],
      crossing_count: 0,
      until_look: None,
      patience: 0,
      level: 0.0,
      until_track: TRACK_EVERY,
      held: None,
      lost: None,
      needed: LOCK_SCORE,
    };
    detector.set_range(range);
    detector
  }

  pub(crate) fn range(&self) -> Range {
    self.range
  }

  /// whether the last estimate stands, which it does until a later one scores too low, the
  /// crossings stop or the input falls silent
  pub(crate) fn voiced(&self) -> bool {
    self.held.is_some()
  }

  /// also forgets everything heard so far
  pub(crate) fn set_range(&mut self, range: Range) {
    let spec = range.spec();
    // at multiples of the base rate that many times as many samples are passed over, so the
    // analysis sees the same rate and costs the same
    let multiple = (self.sample_rate / BASE_RATE).round().max(1.0) as usize;
    let decimation = spec.decimation * multiple;
    let analysis_rate = self.sample_rate / decimation as f32;
    self.range = range;
    self.decimation = decimation;
    self.shortest_lag = (analysis_rate / spec.highest_hz) as usize;
    // two periods of the longest lag and the neighbours tracking scores have to fit
    self.longest_lag =
      ((analysis_rate / spec.lowest_hz) as usize).min(HISTORY / 2 - TRACK_ABOVE - 2);
    self.shortest_compared = (analysis_rate * SHORTEST_COMPARED_SECONDS).ceil() as usize;
    self.steady_samples = analysis_rate * STEADY_SECONDS;
    self.remembered_samples = (analysis_rate * REMEMBERED_SECONDS) as u64;
    self.analysis_filter = Biquad::low_pass(spec.analysis_hz, self.sample_rate);
    self.crossing_filters = [
      Biquad::low_pass(spec.crossing_hz, self.sample_rate),
      Biquad::high_pass(CROSSING_FLOOR_HZ, self.sample_rate),
    ];
    self.reset();
  }

  pub(crate) fn reset(&mut self) {
    self.analysis_filter.state = [0.0; 2];
    for filter in &mut self.crossing_filters {
      filter.state = [0.0; 2];
    }
    self.until_tick = self.decimation;
    self.history.fill(0.0);
    self.positive = false;
    self.until_look = None;
    self.patience = 0;
    self.until_track = TRACK_EVERY;
    self.held = None;
    self.lost = None;
    self.needed = LOCK_SCORE;
  }

  /// takes one sample, the sum of the input's channels, and returns the period in samples
  /// whenever it has a new estimate. between estimates the last one stands
  pub(crate) fn push(&mut self, sample: f32) -> Option<f32> {
    let analysis = self.analysis_filter.process(sample);
    let [low_pass, high_pass] = &mut self.crossing_filters;
    let crossing = high_pass.process(low_pass.process(sample));
    self.until_tick -= 1;
    if self.until_tick > 0 {
      return None;
    }
    self.until_tick = self.decimation;
    self.history[self.next] = analysis;
    self.history[self.next + KEPT] = analysis;
    self.next = (self.next + 1) % KEPT;
    self.now += 1;
    self.level += analysis.abs();

    let positive = crossing > 0.0;
    let crossed = positive != self.positive;
    self.positive = positive;
    let look = if crossed {
      self.patience = self.longest_lag;
      self.note_crossing()
    } else {
      if self.patience > 0 {
        self.patience -= 1;
        if self.patience == 0 {
          self.until_look = None;
          self.lose();
        }
      }
      false
    };

    self.until_track -= 1;
    if !look && self.until_track > 0 {
      return None;
    }
    self.until_track = TRACK_EVERY;

    // a held lag that has left the range scores nothing, like no lag at all
    let tracked = self.track();
    let mut best = tracked.map_or((0, 0.0), |(best, _)| best);
    if look {
      best = self.look(best);
    }
    let (lag, score) = best;
    let scored = tracked.map(|(_, scored)| scored);
    let period = (score >= self.needed).then(|| self.refined(lag, score, scored));
    let held = period.and_then(|period| self.steadied(lag, period));
    let accepted = held.is_some();
    self.needed =
      if accepted && (score >= LOCK_SCORE || (self.needed == HOLD_SCORE && score > HOLD_SCORE)) {
        HOLD_SCORE
      } else {
        LOCK_SCORE
      };
    if look {
      // found, so the next look starts from fresh crossings. otherwise it comes soon
      self.until_look = if accepted {
        None
      } else {
        Some(CROSSINGS_TO_RETRY)
      };
    }
    match held {
      Some(_) => self.held = held,
      None => self.lose(),
    }
    held
      .and(period)
      .map(|period| period * self.decimation as f32)
  }

  fn lose(&mut self) {
    if let Some(held) = self.held.take() {
      self.lost = Some(held);
    }
  }

  /// what is held once an estimate of `period` analysis samples at `lag` is taken, none if it is
  /// out of reach of the pitch held, or of the one lost if that was lately
  fn steadied(&self, lag: usize, period: f32) -> Option<Held> {
    let octaves = period.log2();
    let out_of_reach = |steady: f32| 12.0 * (octaves - steady).abs() > REACH_SEMITONES;
    let steady = match (self.held, self.lost) {
      (Some(Held { steady, at, .. }), _) => {
        if out_of_reach(steady) {
          return None;
        }
        let follow = 1.0 - (-((self.now - at) as f32) / self.steady_samples).exp();
        steady + follow * (octaves - steady)
      }
      (None, Some(lost)) if self.now - lost.at <= self.remembered_samples => {
        if out_of_reach(lost.steady) {
          return None;
        }
        octaves
      }
      (None, _) => octaves,
    };
    Some(Held {
      lag,
      steady,
      at: self.now,
    })
  }

  /// records a crossing at the current time and says whether it is time to look
  fn note_crossing(&mut self) -> bool {
    let Some(left) = self.until_look else {
      self.crossings[0] = self.now;
      self.crossing_count = 1;
      self.level = 0.0;
      self.until_look = Some(CROSSINGS_TO_LOOK - 1);
      return false;
    };
    if self.crossing_count == MAX_CROSSINGS {
      self.crossings.copy_within(1.., 0);
      self.crossing_count -= 1;
    }
    self.crossings[self.crossing_count] = self.now;
    self.crossing_count += 1;
    if left > 1 {
      self.until_look = Some(left - 1);
      return false;
    }
    let span = self.now - self.crossings[0];
    if self.level <= SILENCE_LEVEL * span as f32 {
      self.until_look = None;
      self.lose();
      return false;
    }
    true
  }

  /// how alike the newest samples and the ones before them are, 1 for identical, over each
  /// of the `LANES` lags from `first` up. a lag takes as many samples of each as it is long,
  /// and `shortest_compared` at least.
  /// the lags are summed side by side, each in the order it would be alone, since one sum
  /// has to wait for each of its additions and many can wait together
  fn scores(&self, first: usize) -> [f32; LANES] {
    let longest = first + LANES - 1;
    let compared = longest.max(self.shortest_compared);
    let recent = &self.history[self.next + KEPT - longest - compared..self.next + KEPT];
    // a lane for each lag, the longest first, which puts the samples a lag behind in order
    let (mut products, mut energies) = ([0f32; LANES], [0f32; LANES]);
    let mut add = |age: usize, lanes: usize| {
      let newer = recent[longest + compared - 1 - age];
      let behind = &recent[compared - 1 - age..][..lanes];
      for ((product, energy), older) in products.iter_mut().zip(&mut energies).zip(behind) {
        *product += newer * older;
        *energy += newer * newer + older * older;
      }
    };
    // every lag goes as far as the shortest or the least compared, then each stops a sample
    // before the next
    let all = first.max(self.shortest_compared).min(compared);
    for age in 0..all {
      add(age, LANES);
    }
    for age in all..compared {
      add(age, longest - age);
    }
    let mut scores = [0.0; LANES];
    for ((score, product), energy) in scores.iter_mut().rev().zip(products).zip(energies) {
      // silence is alike but is no pitch
      if energy > 0.0 {
        *score = 2.0 * product / energy;
      }
    }
    scores
  }

  /// the best scoring lag and its score, the shortest of equals
  fn best_of(&self, lags: std::ops::Range<usize>) -> Option<(usize, f32)> {
    let mut best: Option<(usize, f32)> = None;
    for first in lags.clone().step_by(LANES) {
      for scored in (first..lags.end).zip(self.scores(first)) {
        if best.is_none_or(|best| scored.1 > best.1) {
          best = Some(scored);
        }
      }
    }
    best
  }

  /// whether every lag of `lags` is one a look may score
  fn in_range(&self, lags: &std::ops::Range<usize>) -> bool {
    self.shortest_lag <= lags.start && lags.end <= self.longest_lag
  }

  /// the best of the lags around the held one. with it come the first lag and the scores of
  /// the set of lanes it was scored in, which has those lags and one more either side
  fn track(&self) -> Option<((usize, f32), Scored)> {
    let held = self
      .held
      .map(|held| held.lag)
      .filter(|&lag| self.shortest_lag < lag && lag < self.longest_lag)?;
    let lags = held.saturating_sub(TRACK_BELOW).max(1)..=held + TRACK_ABOVE;
    let first = held.saturating_sub(TRACK_BELOW + 1);
    let scores = self.scores(first);
    let tracked = (first..).zip(scores).filter(|(lag, _)| lags.contains(lag));
    let best = tracked.reduce(|best, next| if next.1 > best.1 { next } else { best })?;
    Some((best, (first, scores)))
  }

  /// a look at the lags the crossings suggest. `tracked` stands if it is good enough, else
  /// a good enough suggestion replaces it, and a lag good enough either way gives way to its
  /// half if that scores no worse
  fn look(&self, tracked: (usize, f32)) -> (usize, f32) {
    let chosen = if tracked.1 >= LOCK_SCORE {
      tracked
    } else {
      match self.suggested() {
        Some(suggested) if suggested.1 >= LOCK_SCORE => suggested,
        _ => return tracked,
      }
    };
    let halves = chosen.0 * 98 / 200..chosen.0 * 102 / 200;
    match self.best_of(halves.clone()) {
      Some(half) if self.in_range(&halves) && half.1 >= chosen.1 => half,
      _ => chosen,
    }
  }

  /// the best lag of the shortest range the crossings suggest that scores nearly as well as
  /// the best of them. a crossing comes at the same point of every cycle, so a stretch that
  /// ends at one of the newest two crossings and has a crossing half way along it spans whole
  /// periods, and the lag is looked for around half its length. the halves are found by time
  /// and not by counting crossings, since where a sound starts and where a harmonic rides
  /// across zero there are more than two a period
  fn suggested(&self) -> Option<(usize, f32)> {
    let times = &self.crossings[..self.crossing_count];
    // each of the two ends has a stretch from every other crossing at most
    let mut ranges = [None; MAX_CROSSINGS];
    let mut ranges_found = ranges.iter_mut();
    for end in (times.len().saturating_sub(2)..times.len()).rev() {
      let end_time = times[end];
      // crossings go up and down by turns, and a period runs from one to the next that goes
      // the same way, which is an even count of crossings on
      for (index, &start) in times[..end].iter().enumerate().rev().skip(1).step_by(2) {
        let halves = times[index + 2..end].iter().step_by(2).find_map(|&at| {
          let (earlier, later) = (at - start, end_time - at);
          let apart = earlier.abs_diff(later);
          // the times are whole analysis samples, so equal halves can come out one apart
          (apart <= 1 || apart * 100 < HALVES_PERCENT * later).then_some((earlier, later))
        });
        let Some((earlier, later)) = halves else {
          continue;
        };
        let (shorter, longer) = (earlier.min(later), earlier.max(later));
        let lags = (shorter * (100 - HALVES_PERCENT) / 100) as usize
          ..(longer * (100 + HALVES_PERCENT) / 100) as usize + 2;
        if self.in_range(&lags) {
          let range = ranges_found.next().expect("a range for every stretch");
          *range = self.best_of(lags);
        }
      }
    }
    let found = ranges.iter().flatten();
    let best = found.clone().map(|range| range.1).max_by(f32::total_cmp)?;
    found
      .filter(|range| range.1 > SHORTER_WINS_ABOVE * best)
      .min_by_key(|range| range.0)
      .copied()
  }

  /// `lag` moved to the vertex of the parabola through its score and its neighbours'.
  /// `scored` is what tracking scored, which has the neighbours unless a look took up
  /// another lag
  fn refined(&self, lag: usize, score: f32, scored: Option<Scored>) -> f32 {
    let [below, above] = match scored {
      Some((first, scores)) if first < lag && lag + 1 < first + LANES => {
        [scores[lag - 1 - first], scores[lag + 1 - first]]
      }
      _ => {
        let [below, _, above, ..] = self.scores(lag - 1);
        [below, above]
      }
    };
    let curvature = below - 2.0 * score + above;
    let offset = if curvature < 0.0 {
      0.5 * (below - above) / curvature
    } else {
      0.0
    };
    lag as f32 + offset
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::signals::{VOWEL_HZ, spoken, vowel_sampled_at};

  const RATE: f32 = 48_000.0;

  fn estimates(signal: impl Iterator<Item = f32>, rate: f32, range: Range) -> Vec<f32> {
    let mut detector = Detector::new(rate, range);
    signal.filter_map(|x| detector.push(x)).collect()
  }

  fn assert_detects(hz: f32, range: Range, wave: impl Fn(f32) -> f32) {
    assert_detects_at(RATE, hz, range, wave);
  }

  fn assert_detects_at(rate: f32, hz: f32, range: Range, wave: impl Fn(f32) -> f32) {
    let signal = (0..rate as usize / 2).map(|i| wave((i as f32 * hz / rate).fract()));
    let periods = estimates(signal, rate, range);
    let Some(&period) = periods.last() else {
      panic!("{hz} Hz was not found at {rate} Hz")
    };
    let detected = rate / period;
    assert!(
      (detected / hz - 1.0).abs() < 0.005,
      "{hz} Hz detected as {detected} Hz at {rate} Hz"
    );
  }

  /// the lags scored side by side each come out as one scored alone does, to the bit
  #[test]
  fn lags_score_side_by_side_as_they_do_alone() {
    let mut detector = Detector::new(RATE, Range::Mid);
    let mut state = 0x2545_f491_u32;
    // more samples than are kept, so the newest are across where the first of them went
    for _ in 0..3 * KEPT {
      state ^= state << 13;
      state ^= state >> 17;
      state ^= state << 5;
      detector.push(state as f32 / u32::MAX as f32 - 0.5);
    }
    let ago = |age: usize| detector.history[(detector.next + KEPT - 1 - age) % KEPT];
    let alone = |lag: usize| {
      let (mut product, mut energy) = (0.0, 0.0);
      for age in 0..lag.max(detector.shortest_compared) {
        let (newer, older) = (ago(age), ago(age + lag));
        product += newer * older;
        energy += newer * newer + older * older;
      }
      2.0 * product / energy
    };
    for first in [1, 2, 17, 300, HISTORY / 2 - 1] {
      for (lag, score) in (first..).zip(detector.scores(first)) {
        assert!(score == alone(lag), "a lag of {lag} scored from {first}");
      }
    }
  }

  #[test]
  fn sines() {
    for hz in [70.0, 110.0, 233.1, 440.0, 880.0] {
      assert_detects(hz, Range::Mid, |phase| {
        0.5 * (phase * std::f32::consts::TAU).sin()
      });
    }
  }

  #[test]
  fn sawtooths() {
    for hz in [82.4, 155.6, 311.1, 622.3] {
      assert_detects(hz, Range::Mid, |phase| phase - 0.5);
    }
  }

  /// a look scores lags from a quarter under to a quarter over the period the crossings
  /// suggest, and all of them have to be in the range. so a pitch is found at its own period
  /// from a quarter above the lowest of the range to a quarter below the highest
  #[test]
  fn each_range_is_found_between_its_quarters() {
    let sine = |phase: f32| 0.5 * (phase * std::f32::consts::TAU).sin();
    for (range, hz) in [
      (Range::Bass, 35.0),
      (Range::Bass, 500.0),
      (Range::High, 1400.0),
    ] {
      assert_detects(hz, range, sine);
    }
  }

  /// near the top of a range a period is few analysis samples, fewer the lower the sample
  /// rate. a stretch of two periods still has to count as whole periods there, or only longer
  /// stretches do and the pitch is found an octave low
  #[test]
  fn the_top_of_each_range_is_found_at_every_sample_rate() {
    let sine = |phase: f32| 0.5 * (phase * std::f32::consts::TAU).sin();
    for rate in [44_100.0, 48_000.0, 88_200.0, 96_000.0, 192_000.0] {
      for (range, hz) in [
        (Range::Bass, 500.0),
        (Range::Mid, 622.3),
        (Range::Mid, 700.0),
        (Range::High, 1400.0),
      ] {
        assert_detects_at(rate, hz, range, sine);
      }
    }
  }

  #[test]
  fn noise_and_silence_have_no_pitch() {
    let mut state = 0x2545_f491_u32;
    let noise = (0..RATE as usize / 2).map(|_| {
      state ^= state << 13;
      state ^= state >> 17;
      state ^= state << 5;
      state as f32 / u32::MAX as f32 - 0.5
    });
    assert!(estimates(noise, RATE, Range::Mid).is_empty());
    let silence = std::iter::repeat_n(0.0, RATE as usize / 2);
    assert!(estimates(silence, RATE, Range::Mid).is_empty());
  }

  /// a voice slides an octave in a tenth of a second at the fastest, which is still one pitch
  /// moving and is followed without losing it
  #[test]
  fn the_fastest_slide_of_a_voice_is_followed() {
    let (before, rise) = (RATE as usize / 2, RATE as usize / 5);
    let mut detector = Detector::new(RATE, Range::Mid);
    let (mut phase, mut lost, mut last) = (0f32, 0, None);
    for i in 0..before + rise + before {
      let risen = (i.saturating_sub(before) as f32 / rise as f32).min(1.0);
      let hz = 110.0 * 4f32.powf(risen);
      phase = (phase + hz / RATE).fract();
      let was_held = detector.voiced();
      if let Some(period) = detector.push(phase - 0.5) {
        last = Some((RATE / period, hz));
      }
      lost += usize::from(was_held && !detector.voiced());
    }
    let (detected, actual) = last.expect("the slide was found");
    assert!(
      (detected / actual - 1.0).abs() < 0.01,
      "{actual} Hz detected as {detected} Hz"
    );
    assert_eq!(lost, 0, "the pitch was dropped on the way");
  }

  /// a held pitch that glides is followed without losing it
  #[test]
  fn follows_a_glide() {
    let mut phase = 0f32;
    let mut detector = Detector::new(RATE, Range::Mid);
    let mut last = None;
    let mut lost = 0;
    let samples = RATE as usize;
    for i in 0..samples {
      // an octave up over a second
      let hz = 110.0 * 2f32.powf(i as f32 / samples as f32);
      phase = (phase + hz / RATE).fract();
      let was_held = detector.held.is_some();
      if let Some(period) = detector.push(phase - 0.5) {
        last = Some((RATE / period, hz));
      }
      lost += usize::from(was_held && detector.held.is_none());
    }
    let (detected, actual) = last.expect("the glide was found");
    assert!(
      (detected / actual - 1.0).abs() < 0.01,
      "{actual} Hz detected as {detected} Hz"
    );
    assert_eq!(lost, 0, "the pitch was dropped on the way");
  }

  /// periods of `hz` from the first sound of a vowel that starts out of silence to the first
  /// estimate within a percent of it, if there is one
  fn periods_to_find(rate: f32, range: Range, hz: f32) -> Option<f32> {
    let lead = rate as usize / 4;
    let mut signal = vec![0.0; lead];
    signal.extend(vowel_sampled_at(rate, rate as usize / 4, |_| hz));
    let sounds = signal.iter().position(|&x| x != 0.0)?;
    let mut detector = Detector::new(rate, range);
    let found = signal.iter().enumerate().position(|(index, &x)| {
      let estimate = detector.push(2.0 * x);
      index >= sounds && estimate.is_some_and(|period| (rate / period / hz - 1.0).abs() < 0.01)
    })?;
    Some((found - sounds) as f32 * hz / rate)
  }

  /// a voice is found within two and a half periods of where it starts, though the crossings
  /// that come first are not yet at one point of its cycle. this vowel's pulses all go one
  /// way, so its mean steps up where it starts and the crossings take some milliseconds more
  /// to settle around it, which only low pitches have the periods to spare for
  #[test]
  fn a_voice_is_found_soon_after_it_starts() {
    for rate in [44_100.0, 48_000.0, 96_000.0] {
      for (range, hz) in [
        (Range::Bass, 45.0),
        (Range::Bass, 70.0),
        (Range::Mid, 70.0),
        (Range::Mid, 100.0),
        (Range::Mid, 125.0),
      ] {
        let periods = periods_to_find(rate, range, hz);
        assert!(
          periods.is_some_and(|periods| periods <= 2.5),
          "{hz} Hz in {range:?} at {rate} Hz is found after {periods:?} periods"
        );
      }
    }
  }

  /// a consonant after a pause is noise, and the vowel after it starts with its formant
  /// ringing alone before the next pulse. neither is taken for a pitch, though each can
  /// repeat over a period or two of a high one
  #[test]
  fn a_consonant_and_the_ringing_after_it_are_no_pitch() {
    let mut detector = Detector::new(RATE, Range::Mid);
    for (index, x) in spoken(RATE, RATE as usize, VOWEL_HZ)
      .into_iter()
      .enumerate()
    {
      if let Some(period) = detector.push(x) {
        // the vowel wanders a little over a semitone either way
        let off = 12.0 * (RATE / period / VOWEL_HZ).log2();
        assert!(off.abs() < 1.5, "{off} semitones off at {index}");
      }
    }
  }
}
