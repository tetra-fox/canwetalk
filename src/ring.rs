/// a delay line read by how long ago a sample was pushed
pub(crate) struct Ring {
  samples: Vec<f32>,
  mask: usize,
  next: usize,
}

impl Ring {
  /// holds at least `len` samples
  pub(crate) fn new(len: usize) -> Self {
    let len = len.next_power_of_two();
    Self {
      samples: vec![0.0; len],
      mask: len - 1,
      next: 0,
    }
  }

  pub(crate) fn clear(&mut self) {
    self.samples.fill(0.0);
  }

  pub(crate) fn push(&mut self, sample: f32) {
    self.samples[self.next] = sample;
    self.next = (self.next + 1) & self.mask;
  }

  /// how many samples are held
  pub(crate) fn len(&self) -> usize {
    self.samples.len()
  }

  /// the sample pushed `delay` pushes before the newest, which has to be one still held
  pub(crate) fn ago(&self, delay: usize) -> f32 {
    debug_assert!(
      delay < self.samples.len(),
      "a delay of {delay} is past what the ring holds"
    );
    self.samples[self.next.wrapping_sub(1 + delay) & self.mask]
  }

  /// `ago` at a fractional delay, by cubic interpolation
  pub(crate) fn read(&self, delay: f64) -> f32 {
    let (whole, part) = split(delay);
    cubic(self.taps(0, whole), part)
  }

  /// the four samples a read at a delay of `whole` samples and a part of one goes by, the
  /// newest first, as they were before the newest `newer` samples were pushed
  pub(crate) fn taps(&self, newer: usize, whole: usize) -> [f32; 4] {
    let ago = |delay: usize| self.ago(newer + delay);
    let (b, c, d) = (ago(whole), ago(whole + 1), ago(whole + 2));
    // nothing is newer than the newest sample, so its neighbour is taken from the line
    // through it and the one before
    let a = if whole == 0 {
      2.0 * b - c
    } else {
      ago(whole - 1)
    };
    [a, b, c, d]
  }
}

/// the whole samples in a delay and the part of one that is left
pub(crate) fn split(delay: f64) -> (usize, f32) {
  // the floor, taken by hand since `floor` is a call into the math library
  let toward_zero = delay as i64;
  let whole = toward_zero - i64::from(toward_zero as f64 > delay);
  let part = (delay - whole as f64) as f32;
  // under zero the whole part counts as zero, as it does cast to an index
  (whole.max(0) as usize, part)
}

/// the curve through four samples a sample apart, at `part` of the way from the second to
/// the third
pub(crate) fn cubic([a, b, c, d]: [f32; 4], part: f32) -> f32 {
  let t = part;
  b + 0.5 * t * (c - a + t * (2.0 * a - 5.0 * b + 4.0 * c - d + t * (3.0 * (b - c) + d - a)))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn ring_reads_between_samples() {
    let mut ring = Ring::new(64);
    for i in 0..40 {
      ring.push(i as f32);
    }
    assert_eq!(ring.ago(0), 39.0);
    assert_eq!(ring.ago(7), 32.0);
    assert!((ring.read(7.25) - 31.75).abs() < 1e-4);
    assert!((ring.read(0.5) - 38.5).abs() < 1e-4);
    assert!((cubic(ring.taps(3, 0), 0.5) - 35.5).abs() < 1e-4);
  }
}
