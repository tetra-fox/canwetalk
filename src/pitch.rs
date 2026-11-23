//! monophonic pitch detection. zero crossings of a heavily low-passed copy of the input say
//! when to look and roughly where, the correlation of the newest period with the one before
//! scores a lag, and once a lag is held only its neighbours are scored

use crate::ring::Ring;

/// the span of pitches looked for
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Range {
  High,
  Mid,
  Bass,
}

struct RangeSpec {
  /// every this many input samples one goes to the analysis
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

  /// period of the lowest pitch of the range, in samples
  pub(crate) fn longest_period(self, sample_rate: f32) -> f32 {
    sample_rate / self.spec().lowest_hz
  }
}

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
/// mean magnitude of an analysis sample below which the input is taken for silence
const SILENCE_LEVEL: f32 = 5e-5;

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

pub(crate) struct Detector {
  sample_rate: f32,
  range: Range,
  decimation: usize,
  shortest_lag: usize,
  longest_lag: usize,
  analysis_filter: Biquad,
  crossing_filters: [Biquad; 2],
  /// input samples until the next analysis sample
  until_tick: usize,
  /// analysis samples
  history: Ring,
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
  /// the lag taken for the period, in analysis samples
  held: Option<usize>,
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
      analysis_filter: Biquad::default(),
      crossing_filters: [Biquad::default(); 2],
      until_tick: 1,
      history: Ring::new(HISTORY),
      now: 0,
      positive: false,
      crossings: [0; MAX_CROSSINGS],
      crossing_count: 0,
      until_look: None,
      patience: 0,
      level: 0.0,
      until_track: TRACK_EVERY,
      held: None,
      needed: LOCK_SCORE,
    };
    detector.set_range(range);
    detector
  }

  pub(crate) fn range(&self) -> Range {
    self.range
  }

  /// also forgets everything heard so far
  pub(crate) fn set_range(&mut self, range: Range) {
    let spec = range.spec();
    let analysis_rate = self.sample_rate / spec.decimation as f32;
    self.range = range;
    self.decimation = spec.decimation;
    self.shortest_lag = (analysis_rate / spec.highest_hz) as usize;
    // two periods of the longest lag and the neighbours tracking scores have to fit
    self.longest_lag =
      ((analysis_rate / spec.lowest_hz) as usize).min(HISTORY / 2 - TRACK_ABOVE - 2);
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
    self.history.clear();
    self.positive = false;
    self.until_look = None;
    self.patience = 0;
    self.until_track = TRACK_EVERY;
    self.held = None;
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
    self.history.push(analysis);
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
          self.held = None;
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
    let mut best = self.track().unwrap_or((0, 0.0));
    if look {
      best = self.look(best);
    }
    let (lag, score) = best;
    let accepted = score >= self.needed;
    self.needed = if score >= LOCK_SCORE || (self.needed == HOLD_SCORE && score > HOLD_SCORE) {
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
    if !accepted {
      self.held = None;
      return None;
    }
    self.held = Some(lag);
    Some(self.refined(lag, score) * self.decimation as f32)
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
      self.held = None;
      return false;
    }
    true
  }

  /// how alike the newest `lag` samples and the `lag` before them are, 1 for identical
  fn score(&self, lag: usize) -> f32 {
    let (mut product, mut energy) = (0.0, 0.0);
    for age in 0..lag {
      let (newer, older) = (self.history.ago(age), self.history.ago(age + lag));
      product += newer * older;
      energy += newer * newer + older * older;
    }
    // silence is alike but is no pitch
    if energy > 0.0 {
      2.0 * product / energy
    } else {
      0.0
    }
  }

  /// the best scoring lag and its score, the shortest of equals
  fn best_of(&self, lags: impl Iterator<Item = usize>) -> Option<(usize, f32)> {
    lags
      .map(|lag| (lag, self.score(lag)))
      .reduce(|best, next| if next.1 > best.1 { next } else { best })
  }

  /// whether every lag of `lags` is one a look may score
  fn in_range(&self, lags: &std::ops::Range<usize>) -> bool {
    self.shortest_lag <= lags.start && lags.end <= self.longest_lag
  }

  /// the best of the lags around the held one
  fn track(&self) -> Option<(usize, f32)> {
    let held = self
      .held
      .filter(|&lag| self.shortest_lag < lag && lag < self.longest_lag)?;
    self.best_of(held.saturating_sub(TRACK_BELOW).max(1)..=held + TRACK_ABOVE)
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
  /// the best of them. crossings come twice a period, so a stretch of them that splits into
  /// equal halves spans whole periods, and the lag is looked for around half its length
  fn suggested(&self) -> Option<(usize, f32)> {
    let times = &self.crossings[..self.crossing_count];
    let [.., third_last, second_last, last] = *times else {
      return None;
    };
    let mut ranges = [None; MAX_CROSSINGS];
    for (range, (index, &start)) in ranges
      .iter_mut()
      .zip(times.iter().enumerate().take(times.len().saturating_sub(5)))
    {
      let middle = index + (times.len() - 2 - index) / 2;
      let splits_evenly = [middle, middle + 1].iter().any(|&at| {
        let (earlier, later) = (times[at] - start, second_last - times[at]);
        earlier.abs_diff(later) * 100 < HALVES_PERCENT * later
      });
      let lags = ((third_last - start) / 2) as usize..((last - start) / 2) as usize;
      if splits_evenly && self.in_range(&lags) {
        *range = self.best_of(lags);
      }
    }
    let found = ranges.iter().flatten();
    let best = found.clone().map(|range| range.1).max_by(f32::total_cmp)?;
    // later crossings start shorter stretches, so the last range is the shortest
    found
      .rev()
      .find(|range| range.1 > SHORTER_WINS_ABOVE * best)
      .copied()
  }

  /// `lag` moved to the vertex of the parabola through its score and its neighbours'
  fn refined(&self, lag: usize, score: f32) -> f32 {
    let (below, above) = (self.score(lag - 1), self.score(lag + 1));
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

  const RATE: f32 = 48_000.0;

  fn estimates(signal: impl Iterator<Item = f32>, range: Range) -> Vec<f32> {
    let mut detector = Detector::new(RATE, range);
    signal.filter_map(|x| detector.push(x)).collect()
  }

  fn assert_detects(hz: f32, range: Range, wave: impl Fn(f32) -> f32) {
    let signal = (0..RATE as usize / 2).map(|i| wave((i as f32 * hz / RATE).fract()));
    let periods = estimates(signal, range);
    let Some(&period) = periods.last() else {
      panic!("{hz} Hz was not found")
    };
    let detected = RATE / period;
    assert!(
      (detected / hz - 1.0).abs() < 0.005,
      "{hz} Hz detected as {detected} Hz"
    );
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

  #[test]
  fn noise_and_silence_have_no_pitch() {
    let mut state = 0x2545_f491_u32;
    let noise = (0..RATE as usize / 2).map(|_| {
      state ^= state << 13;
      state ^= state >> 17;
      state ^= state << 5;
      state as f32 / u32::MAX as f32 - 0.5
    });
    assert!(estimates(noise, Range::Mid).is_empty());
    assert!(estimates(std::iter::repeat_n(0.0, RATE as usize / 2), Range::Mid).is_empty());
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
}
