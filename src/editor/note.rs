//! the note nearest a pitch, as the screen writes it

const NAMES: [&str; 12] = [
  "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// the name of the note nearest `hz` with its octave, "A4" at 440 hertz, and the cents from
/// that note to `hz`, from -50 to 50
pub(super) fn nearest(hz: f32) -> (String, i32) {
  let key = 69.0 + 12.0 * (hz / 440.0).log2();
  let nearest = key.round();
  let cents = ((key - nearest) * 100.0).round() as i32;
  let nearest = nearest as i32;
  let name = NAMES[nearest.rem_euclid(12) as usize];
  // MIDI's middle C, key 60, is C4
  (format!("{name}{}", nearest.div_euclid(12) - 1), cents)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_pitch_is_named_by_its_nearest_note_and_the_cents_off_it() {
    for (hz, named) in [
      (440.0, "A4"),
      (261.626, "C4"),
      (27.5, "A0"),
      (466.164, "A#4"),
      (246.942, "B3"),
      (16.352, "C0"),
      (8.176, "C-1"),
    ] {
      assert_eq!(nearest(hz), (named.to_owned(), 0), "{hz} hertz");
    }
    // a fifth of a semitone either way of A4, and just short of halfway to the next one
    assert_eq!(nearest(445.0), ("A4".to_owned(), 20));
    assert_eq!(nearest(435.0), ("A4".to_owned(), -20));
    assert_eq!(
      nearest(440.0 * 2f32.powf(0.49 / 12.0)),
      ("A4".to_owned(), 49)
    );
    // past halfway it is the next note's, flat
    assert_eq!(
      nearest(440.0 * 2f32.powf(0.51 / 12.0)),
      ("A#4".to_owned(), -49)
    );
  }
}
