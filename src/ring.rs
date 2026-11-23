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
    debug_assert!(
      newer + whole + 2 < self.samples.len(),
      "a delay of {whole} is past what the ring held {newer} samples ago"
    );
    // they lie one after another, the oldest first, unless the end of the ring comes between
    let oldest = self.next.wrapping_sub(newer + whole + 3) & self.mask;
    let [d, c, b, after] = match self.samples.get(oldest..oldest + 4) {
      Some(&[d, c, b, after]) => [d, c, b, after],
      _ => std::array::from_fn(|tap| self.samples[(oldest + tap) & self.mask]),
    };
    // nothing is newer than the newest sample, so its neighbour is taken from the line
    // through it and the one before
    let a = if whole == 0 { 2.0 * b - c } else { after };
    [a, b, c, d]
  }

  /// `taps` for reads at one delay a sample after another, each of the four a run of its
  /// own. the first read is as it was before the newest `newer` samples were pushed. a run
  /// is of the ring itself where it lies in one piece there, and else is put in `room`
  pub(crate) fn taps_along<'a>(
    &'a self,
    newer: usize,
    whole: usize,
    [a, b, c, d]: [&'a mut [f32]; 4],
  ) -> [&'a [f32]; 4] {
    let (b, c, d) = (
      self.run(newer + whole, b),
      self.run(newer + whole + 1, c),
      self.run(newer + whole + 2, d),
    );
    if whole > 0 {
      return [self.run(newer + whole - 1, a), b, c, d];
    }
    for ((a, b), c) in a.iter_mut().zip(b).zip(c) {
      *a = 2.0 * *b - *c;
    }
    [a, b, c, d]
  }

  /// as many samples as `room` has in the order they were pushed, from the one pushed
  /// `ago` pushes before the newest. `room` is taken up only if the end of the ring comes
  /// among them
  fn run<'a>(&'a self, ago: usize, room: &'a mut [f32]) -> &'a [f32] {
    debug_assert!(
      room.len() <= ago + 1 && ago < self.samples.len(),
      "{} samples from {ago} ago are not all held",
      room.len()
    );
    let first = self.next.wrapping_sub(1 + ago) & self.mask;
    if let Some(run) = self.samples.get(first..first + room.len()) {
      return run;
    }
    let (straight, around) = room.split_at_mut(self.samples.len() - first);
    straight.copy_from_slice(&self.samples[first..]);
    around.copy_from_slice(&self.samples[..around.len()]);
    room
  }
}

/// the whole samples in a delay, which no ring holds 2^31 of, and the part of one that is
/// left
pub(crate) fn split(delay: f64) -> (usize, f32) {
  // the floor, taken by hand since `floor` is a call into the math library
  let toward_zero = delay as i32;
  let whole = toward_zero - i32::from(f64::from(toward_zero) > delay);
  let part = (delay - f64::from(whole)) as f32;
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

  /// reads a sample after another at one delay go by the same samples whether they are
  /// taken together or one by one, across the end of the ring too
  #[test]
  fn taps_along_are_the_taps_of_each_read() {
    let mut ring = Ring::new(32);
    for i in 0..70 {
      ring.push((i * i) as f32);
    }
    for (newer, whole) in [(4, 0), (4, 1), (9, 3), (7, 20)] {
      let mut room = [[0.0; 5]; 4];
      let along = ring.taps_along(newer, whole, room.each_mut().map(|tap| &mut tap[..]));
      for read in 0..5 {
        let each = ring.taps(newer - read, whole);
        assert_eq!(
          along.map(|tap| tap[read]),
          each,
          "read {read} of {newer}, {whole}"
        );
      }
    }
  }
}
