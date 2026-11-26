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
  pub(crate) fn new(sample_rate: f32) -> Self {
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
    Self {
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
    }
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

  #[test]
  fn a_full_scale_sine_reads_nothing_under_full_scale_in_its_band() {
    let rate = 48_000.0;
    let mut analyzer = Analyzer::new(rate);
    let feed = Feed::new(SLOTS);
    // on a bin, a hundred and second times as many hertz as the bins are apart
    let hz = 102.0 * rate / analyzer.heard.len() as f32;
    let frames: Vec<[f32; 2]> = (0..analyzer.heard.len() * 2)
      .map(|index| [(TAU * hz * index as f32 / rate).sin(); 2])
      .collect();
    analyzer.take(&frames, Some(hz), None, &feed);
    let band = (HERTZ.turn(hz) * BANDS as f32) as usize;
    let level = analyzer.shown[band];
    assert!(level.abs() < 0.1, "{level} dB");
    // far from it the window lets almost nothing through
    let far = (HERTZ.turn(hz * 8.0) * BANDS as f32) as usize;
    assert!(analyzer.shown[far] < -60.0, "{} dB", analyzer.shown[far]);
  }
}
