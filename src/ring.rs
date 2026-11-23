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
    let whole = delay.floor();
    let t = (delay - whole) as f32;
    let whole = whole as usize;
    let (b, c, d) = (self.ago(whole), self.ago(whole + 1), self.ago(whole + 2));
    // nothing is newer than the newest sample, so its neighbour is taken from the line
    // through it and the one before
    let a = if whole == 0 {
      2.0 * b - c
    } else {
      self.ago(whole - 1)
    };
    b + 0.5 * t * (c - a + t * (2.0 * a - 5.0 * b + 4.0 * c - d + t * (3.0 * (b - c) + d - a)))
  }
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
  }
}
