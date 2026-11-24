//! the values the editor has set that the wrapper has not reported back.
//!
//! a value set from the editor reaches the parameter later: the wrappers take it at the end of
//! an audio block, and the vst3 one waits for the host to hand it back first. until then the
//! panel shows what was set and not what the parameter still holds

use std::collections::VecDeque;

use super::Control;

/// how many values of one control are remembered. a host that never reports any back leaves
/// them here, and past this many the oldest are forgotten
const ROOM: usize = 256;
/// a report this close to a value that was set is that value come back
const SAME: f32 = 1e-6;

#[derive(Debug, Default)]
pub(super) struct Sent {
  /// for each control what was set and has not come back, oldest first
  waiting: [VecDeque<f32>; 3],
}

impl Sent {
  pub(super) fn set(&mut self, control: Control, value: f32) {
    let waiting = &mut self.waiting[control as usize];
    if waiting.len() == ROOM {
      waiting.pop_front();
    }
    waiting.push_back(value);
  }

  /// takes in that the parameter of a control now has a value, and tells whether that changes
  /// what the panel shows. values come back in the order they were set, so one coming back
  /// settles every one before it too. a value that was never set here is the host's own, and
  /// the parameter is shown again
  pub(super) fn reported(&mut self, control: Control, value: f32) -> bool {
    let waiting = &mut self.waiting[control as usize];
    if waiting.is_empty() {
      return true;
    }
    match (waiting.iter()).position(|sent| (sent - value).abs() <= SAME) {
      Some(place) => {
        waiting.drain(..=place);
        false
      }
      None => {
        waiting.clear();
        true
      }
    }
  }

  /// every parameter may have changed, so all of them are shown as they are
  pub(super) fn forget(&mut self) {
    for waiting in &mut self.waiting {
      waiting.clear();
    }
  }

  /// the value to show for a control in place of its parameter's
  pub(super) fn shown(&self, control: Control) -> Option<f32> {
    self.waiting[control as usize].back().copied()
  }
}

#[cfg(test)]
mod tests {
  use super::Control::{Mix, Pitch};
  use super::*;

  #[test]
  fn what_was_set_shows_until_all_of_it_has_come_back() {
    let mut sent = Sent::default();
    assert_eq!(sent.shown(Pitch), None);
    for value in [0.1, 0.2, 0.3] {
      sent.set(Pitch, value);
    }
    assert_eq!(sent.shown(Pitch), Some(0.3));
    assert_eq!(sent.shown(Mix), None);
    // the older ones coming back change nothing that shows
    assert!(!sent.reported(Pitch, 0.1));
    assert_eq!(sent.shown(Pitch), Some(0.3));
    assert!(!sent.reported(Pitch, 0.2));
    assert_eq!(sent.shown(Pitch), Some(0.3));
    assert!(!sent.reported(Pitch, 0.3));
    assert_eq!(sent.shown(Pitch), None);
  }

  #[test]
  fn a_value_coming_back_settles_the_ones_before_it() {
    // a wrapper reports only a value that differs from the one before, so some never come
    let mut sent = Sent::default();
    for value in [0.1, 0.1, 0.2] {
      sent.set(Pitch, value);
    }
    assert!(!sent.reported(Pitch, 0.2));
    assert_eq!(sent.shown(Pitch), None);
  }

  #[test]
  fn the_hosts_own_value_takes_over() {
    let mut sent = Sent::default();
    sent.set(Pitch, 0.4);
    sent.set(Mix, 0.9);
    assert!(sent.reported(Pitch, 0.7));
    assert_eq!(sent.shown(Pitch), None);
    assert_eq!(sent.shown(Mix), Some(0.9));
    // with nothing waiting every report is news
    assert!(sent.reported(Pitch, 0.7));
    sent.forget();
    assert_eq!(sent.shown(Mix), None);
  }

  #[test]
  fn a_host_that_reports_nothing_back_does_not_fill_the_room() {
    let mut sent = Sent::default();
    for step in 0..ROOM * 3 {
      sent.set(Mix, step as f32);
    }
    assert_eq!(sent.waiting[Mix as usize].len(), ROOM);
    assert_eq!(sent.shown(Mix), Some((ROOM * 3 - 1) as f32));
  }
}
