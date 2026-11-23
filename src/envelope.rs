//! the level of a voice over the life of its note

use crate::last::Last;

/// level a released voice counts as silent at, 60 dB down
const SILENT: f64 = 0.001;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Adsr {
  /// seconds to rise to full level
  pub(crate) attack: f64,
  /// seconds to fall 60 dB of the way from full level to `sustain`
  pub(crate) decay: f64,
  /// level held while the note is, 0 to 1
  pub(crate) sustain: f64,
  /// seconds to fall 60 dB once the note is let go
  pub(crate) release: f64,
}

#[derive(Default)]
pub(crate) struct Envelope {
  level: f64,
  attacking: bool,
  /// the factor the level last fell by each sample, for the samples that fall took
  fall: Last,
}

impl Envelope {
  /// starts the attack from the level the envelope is at
  pub(crate) fn trigger(&mut self) {
    self.attacking = true;
  }

  pub(crate) fn is_silent(&self) -> bool {
    self.level <= SILENT
  }

  /// the level for the next sample. `held` is whether the note is still down
  pub(crate) fn tick(&mut self, held: bool, adsr: &Adsr, sample_rate: f64) -> f64 {
    // a time of zero makes the factor zero and the attack's step infinite, either of which
    // ends its stage in one sample
    let fall = &mut self.fall;
    let mut falling = |seconds: f64| {
      fall.answer(seconds * sample_rate, |samples| {
        (SILENT.ln() / samples).exp()
      })
    };
    if !held {
      self.attacking = false;
      self.level *= falling(adsr.release);
    } else if self.attacking {
      self.level += 1.0 / (adsr.attack * sample_rate);
      if self.level >= 1.0 {
        self.level = 1.0;
        self.attacking = false;
      }
    } else {
      self.level = adsr.sustain + (self.level - adsr.sustain) * falling(adsr.decay);
    }
    self.level
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const RATE: f64 = 48_000.0;
  const ADSR: Adsr = Adsr {
    attack: 0.01,
    decay: 0.05,
    sustain: 0.4,
    release: 0.1,
  };

  fn after(envelope: &mut Envelope, held: bool, adsr: &Adsr, seconds: f64) -> f64 {
    let samples = (seconds * RATE).round() as usize;
    (0..samples)
      .map(|_| envelope.tick(held, adsr, RATE))
      .last()
      .expect("at least a sample")
  }

  #[test]
  fn each_stage_takes_its_time() {
    let mut envelope = Envelope::default();
    envelope.trigger();
    let rising = after(&mut envelope, true, &ADSR, 0.5 * ADSR.attack);
    assert!(
      (rising - 0.5).abs() < 0.01,
      "half way up the attack is {rising}"
    );
    let top = after(&mut envelope, true, &ADSR, 0.5 * ADSR.attack);
    assert!((top - 1.0).abs() < 0.01, "the attack ends at {top}");
    let sustained = after(&mut envelope, true, &ADSR, ADSR.decay);
    assert!(
      (sustained - ADSR.sustain).abs() < 0.002,
      "the decay ends at {sustained}"
    );
    // 60 dB below the sustain level is under the silent level, so that comes a little early
    after(&mut envelope, false, &ADSR, 0.5 * ADSR.release);
    assert!(!envelope.is_silent(), "silent half way through the release");
    after(&mut envelope, false, &ADSR, 0.5 * ADSR.release);
    assert!(envelope.is_silent(), "sounding after the release");
  }

  #[test]
  fn zero_times_take_one_sample() {
    let instant = Adsr {
      attack: 0.0,
      decay: 0.0,
      sustain: 0.4,
      release: 0.0,
    };
    let mut envelope = Envelope::default();
    envelope.trigger();
    assert_eq!(envelope.tick(true, &instant, RATE), 1.0);
    assert_eq!(envelope.tick(true, &instant, RATE), instant.sustain);
    assert_eq!(envelope.tick(false, &instant, RATE), 0.0);
  }
}
