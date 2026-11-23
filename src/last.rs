//! the last answer of something costly to work out, for where the same is asked sample
//! after sample

/// what was asked last and the answer to it
#[derive(Clone, Copy, Default)]
pub(crate) struct Last {
  /// none before anything is asked
  asked: Option<f64>,
  answer: f64,
}

impl Last {
  /// `work(asked)`, worked out only if `asked` is not what was asked last. one `Last` is
  /// for one `work`
  pub(crate) fn answer(&mut self, asked: f64, work: impl FnOnce(f64) -> f64) -> f64 {
    // by the bits, so that the two zeros are not taken for each other
    if self.asked.map(f64::to_bits) != Some(asked.to_bits()) {
      *self = Self {
        asked: Some(asked),
        answer: work(asked),
      };
    }
    self.answer
  }
}
