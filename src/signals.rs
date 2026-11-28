//! inputs for the tests

pub(crate) const VOWEL_HZ: f32 = 110.0;
pub(crate) const FORMANT_HZ: f32 = 800.0;

/// a pulse train through a low-pass, for the tilt of a voice's spectrum, and a resonance.
/// `hz` gives the pitch for each sample
pub(crate) fn vowel_sampled_at(rate: f32, samples: usize, hz: impl Fn(usize) -> f32) -> Vec<f32> {
  let radius = (-std::f32::consts::PI * 80.0 / rate).exp();
  let feedback = 2.0 * radius * (std::f32::consts::TAU * FORMANT_HZ / rate).cos();
  let tilt = (-std::f32::consts::TAU * VOWEL_HZ / rate).exp();
  let (mut y1, mut y2, mut phase, mut source) = (0.0, 0.0, 0.0, 0.0);
  (0..samples)
    .map(|index| {
      phase += hz(index) / rate;
      let pulse = if phase >= 1.0 {
        phase -= 1.0;
        3.0
      } else {
        0.0
      };
      source = pulse + tilt * (source - pulse);
      let y = source + feedback * y1 - radius * radius * y2;
      (y2, y1) = (y1, y);
      y
    })
    .collect()
}

/// a vowel at `hz`, wandering a tenth of an octave either way, with a pause from 40 to 46
/// percent of the way through and noise like a consonant from there to 52
pub(crate) fn spoken(rate: f32, samples: usize, hz: f32) -> Vec<f32> {
  let wander = |index: usize| {
    let turn = (1.7 * index as f32 / rate).fract();
    hz * 2f32.powf(0.1 * (std::f32::consts::TAU * turn).sin())
  };
  let mut voice = vowel_sampled_at(rate, samples, wander);
  let mut state = 0x2545_f491_u32;
  for (index, sample) in voice.iter_mut().enumerate() {
    state ^= state << 13;
    state ^= state >> 17;
    state ^= state << 5;
    *sample = match 100 * index / samples {
      40..46 => 0.0,
      46..52 => 0.2 * (state as f32 / u32::MAX as f32 - 0.5),
      _ => 0.1 * *sample,
    };
  }
  voice
}
