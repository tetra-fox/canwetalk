//! the input's spectrum, its detected pitch and what formant follow does, taken on the audio
//! thread and written to the feed the editor draws them from

use std::f32::consts::TAU;
use std::sync::Arc;

use potpot_controls::axis::Axis;
use potpot_controls::spectrum;
use potpot_window::Feed;
use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

/// how many bands the spectrum is drawn in
pub(crate) const BANDS: usize = 120;
/// the feed's slots: each band's level in decibels, then the detected pitch in hertz, which is
/// zero while there is none, then the semitones formant follow moves the formants of the voice
/// that started last by, which is zero while none sounds
pub(crate) const PITCH_SLOT: usize = BANDS;
pub(crate) const FOLLOW_SLOT: usize = BANDS + 1;
pub(crate) const SLOTS: usize = BANDS + 2;
/// the frequencies the bands are laid along
pub(crate) const HERTZ: Axis = Axis::log([20.0, 20_000.0]);
/// the quietest level a band shows, in decibels against a full scale sine
pub(crate) const FLOOR_DB: f32 = -90.0;
/// how fast a band falls back once its level has passed, in decibels a second
const FALL_DB: f32 = 40.0;

/// takes the spectrum of the input, which the pitch is detected in, so that the detected pitch
/// is drawn over one of its peaks
pub(crate) struct Analyzer {
  fft: Arc<dyn RealToComplex<f32>>,
  /// a hann window as long as the transform, scaled so that a full scale sine comes out at
  /// one in its bin
  window: Vec<f32>,
  /// the latest input, as long as the transform. `next` is the oldest
  heard: Vec<f32>,
  next: usize,
  /// samples between transforms, and how many are left before the next
  hop: usize,
  until: usize,
  /// what each transform is worked in, made once with the plan
  input: Vec<f32>,
  output: Vec<Complex<f32>>,
  scratch: Vec<Complex<f32>>,
  amplitudes: Vec<f32>,
  bands: [f32; BANDS],
  /// what each band shows, in decibels
  shown: [f32; BANDS],
  /// decibels a band falls by from one transform to the next
  fall: f32,
  sample_rate: f32,
}

impl Analyzer {
  /// writes what it shows before it hears anything to `feed`, the spectrum at its floor and no
  /// pitch, so that the editor never draws the zeros a new feed holds
  pub(crate) fn new(sample_rate: f32, feed: &Feed) -> Self {
    // about a twelfth of a second at every rate, which keeps bins 12 hertz apart or closer
    let length = ((sample_rate / 12.0) as usize).next_power_of_two();
    let fft = RealFftPlanner::<f32>::new().plan_fft_forward(length);
    let hann = |index: usize| 0.5 - 0.5 * (TAU * index as f32 / length as f32).cos();
    // the window sums to half its length, and a sine's energy is split between its bin and
    // the one mirrored past half the rate
    let window = (0..length)
      .map(|index| hann(index) * 4.0 / length as f32)
      .collect();
    let hop = length / 4;
    let analyzer = Self {
      window,
      heard: vec![0.0; length],
      next: 0,
      hop,
      until: hop,
      input: fft.make_input_vec(),
      output: fft.make_output_vec(),
      scratch: fft.make_scratch_vec(),
      amplitudes: vec![0.0; fft.complex_len()],
      bands: [0.0; BANDS],
      shown: [FLOOR_DB; BANDS],
      fall: FALL_DB * hop as f32 / sample_rate,
      sample_rate,
      fft,
    };
    analyzer.write(feed, None, None);
    analyzer
  }

  /// back to how `new` makes it, with nothing heard and nothing shown. does not allocate
  pub(crate) fn reset(&mut self, feed: &Feed) {
    self.heard.fill(0.0);
    self.next = 0;
    self.until = self.hop;
    self.shown.fill(FLOOR_DB);
    self.write(feed, None, None);
  }

  /// takes in frames of input and writes the spectrum to `feed` each time a transform is due,
  /// with `pitch` in hertz if one is detected and the semitones formant follow moves the
  /// formants by, if a voice sounds. does not allocate
  pub(crate) fn take(
    &mut self,
    frames: &[[f32; 2]],
    pitch: Option<f32>,
    followed: Option<f32>,
    feed: &Feed,
  ) {
    for [left, right] in frames {
      self.heard[self.next] = 0.5 * (left + right);
      self.next = (self.next + 1) % self.heard.len();
      self.until -= 1;
      if self.until == 0 {
        self.until = self.hop;
        self.transform();
        self.write(feed, pitch, followed);
      }
    }
  }

  fn transform(&mut self) {
    let (newer, older) = self.heard.split_at(self.next);
    let heard = older.iter().chain(newer);
    for ((input, sample), window) in self.input.iter_mut().zip(heard).zip(&self.window) {
      *input = sample * window;
    }
    self
      .fft
      .process_with_scratch(&mut self.input, &mut self.output, &mut self.scratch)
      .expect("the buffers are the ones the plan made");
    for (amplitude, bin) in self.amplitudes.iter_mut().zip(&self.output) {
      *amplitude = bin.norm();
    }
    spectrum::bands(&self.amplitudes, self.sample_rate, HERTZ, &mut self.bands);
    for (shown, band) in self.shown.iter_mut().zip(&self.bands) {
      // a band at no level at all is at the floor, and a level that is no number goes there too
      let level = (20.0 * band.log10()).max(FLOOR_DB);
      *shown = level.max(*shown - self.fall).max(FLOOR_DB);
    }
  }

  fn write(&self, feed: &Feed, pitch: Option<f32>, followed: Option<f32>) {
    for (slot, shown) in self.shown.iter().enumerate() {
      feed.set(slot, *shown);
    }
    feed.set(PITCH_SLOT, pitch.unwrap_or(0.0));
    feed.set(FOLLOW_SLOT, followed.unwrap_or(0.0));
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const RATE: f32 = 48_000.0;

  /// `samples` of a full scale sine at `hz`
  fn sine(hz: f32, samples: usize) -> Vec<[f32; 2]> {
    (0..samples)
      .map(|index| [(TAU * hz * index as f32 / RATE).sin(); 2])
      .collect()
  }

  /// the band a frequency in hertz is drawn in
  fn band(hz: f32) -> usize {
    (HERTZ.turn(hz) * BANDS as f32) as usize
  }

  #[test]
  fn a_full_scale_sine_reads_full_scale_in_its_band_and_little_far_from_it() {
    let feed = Feed::new(SLOTS);
    let mut analyzer = Analyzer::new(RATE, &feed);
    let length = analyzer.heard.len();
    // between two bins, where the window gives up some of a sine's level, and for a run that
    // ends with the oldest sample partway along the ring
    let hz = 102.3 * RATE / length as f32;
    analyzer.take(&sine(hz, 2 * length + 3 * length / 4), None, None, &feed);
    let level = analyzer.shown[band(hz)];
    assert!((-1.5..0.1).contains(&level), "{level} dB");
    // far from it the window lets almost nothing through, which a ring read out of order
    // would, at the step it puts in the sine
    let far = analyzer.shown[band(hz * 8.0)];
    assert!(far < -75.0, "{far} dB");
  }

  #[test]
  fn a_band_falls_back_at_its_rate_once_its_sound_stops() {
    let feed = Feed::new(SLOTS);
    let mut analyzer = Analyzer::new(RATE, &feed);
    let (length, hop) = (analyzer.heard.len(), analyzer.hop);
    let hz = 1000.0;
    analyzer.take(&sine(hz, 2 * length), None, None, &feed);
    // once the ring holds nothing of the sine, every transform finds the band at the floor
    analyzer.take(&vec![[0.0; 2]; length], None, None, &feed);
    let from = analyzer.shown[band(hz)];
    let hops = 10;
    analyzer.take(&vec![[0.0; 2]; hops * hop], None, None, &feed);
    let fallen = from - analyzer.shown[band(hz)];
    let seconds = (hops * hop) as f32 / RATE;
    assert!(
      (fallen / seconds - FALL_DB).abs() < 0.01,
      "{fallen} dB in {seconds} s"
    );
  }

  #[test]
  fn a_reset_forgets_what_was_heard() {
    let feed = Feed::new(SLOTS);
    let mut analyzer = Analyzer::new(RATE, &feed);
    let hop = analyzer.hop;
    analyzer.take(&sine(1000.0, 3 * hop), None, None, &feed);
    analyzer.reset(&feed);
    assert!(analyzer.shown.iter().all(|&level| level == FLOOR_DB));
    // the next transform finds nothing of the sine in the ring
    analyzer.take(&vec![[0.0; 2]; hop], None, None, &feed);
    assert!(analyzer.shown.iter().all(|&level| level == FLOOR_DB));
  }
}
