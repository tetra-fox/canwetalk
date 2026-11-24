//! the values the editor has set that have not been seen in the parameters yet.
//!
//! a value set from the editor reaches its parameter later: the wrappers take it at the end of
//! an audio block, and the vst3 one waits for the host to hand it back first. until then the
//! panel shows what was set and not what the parameter still holds. whether it has got there
//! is told by looking at the parameter, each time the panel is about to be drawn or checked,
//! and not by anything the wrapper reports, which may come late or not at all.
//!
//! what is kept for a control is empty whenever its parameter is at the value set last, and
//! is emptied by the parameter going anywhere the editor did not send it. so the panel shows
//! a value of the editor's own only while the parameter is still on its way to it

use std::collections::VecDeque;

use super::Control;

/// how many values of one control are remembered. a host that takes none of them leaves them
/// here, and past this many the oldest are forgotten
const ROOM: usize = 256;
/// a parameter this close to a value that was set has that value
const SAME: f32 = 1e-6;

#[derive(Debug)]
pub(super) struct Sent {
  /// for each control what was set and has not been seen in its parameter, oldest first
  waiting: [VecDeque<f32>; 3],
  /// where each parameter was when it was last looked at
  seen: [f32; 3],
}

impl Sent {
  /// `values` is where the parameters are, in the order of [`Control::ALL`]
  pub(super) fn new(values: [f32; 3]) -> Self {
    Self {
      waiting: Default::default(),
      seen: values,
    }
  }

  /// the editor has set a control. its parameter has to have been looked at just before
  pub(super) fn set(&mut self, control: Control, value: f32) {
    let waiting = &mut self.waiting[control as usize];
    if waiting.len() == ROOM {
      waiting.pop_front();
    }
    waiting.push_back(value);
  }

  /// looks at where a control's parameter is now. values get there in the order they were
  /// set, so one of them in the parameter means every one before it has been and gone
  pub(super) fn look(&mut self, control: Control, value: f32) {
    let (waiting, seen) = (
      &mut self.waiting[control as usize],
      &mut self.seen[control as usize],
    );
    let same = |sent: &f32| (sent - value).abs() <= SAME;
    // at the value set last, whether it has just got there or was there all along
    if waiting.back().is_some_and(same) {
      waiting.clear();
    } else if value != *seen {
      // it has moved: to one of the values still on their way, or to one of the host's own
      match waiting.iter().position(same) {
        Some(place) => drop(waiting.drain(..=place)),
        None => waiting.clear(),
      }
    }
    *seen = value;
  }

  /// the value to show for a control whose parameter is at `value`
  pub(super) fn shown(&self, control: Control, value: f32) -> f32 {
    (self.waiting[control as usize].back().copied()).unwrap_or(value)
  }
}

#[cfg(test)]
mod tests {
  use super::Control::{Mix, Pitch};
  use super::*;

  /// the pitch at a half and nothing set
  fn sent() -> Sent {
    Sent::new([0.5, 0.5, 1.0])
  }

  /// what the panel shows of the pitch once its parameter has been looked at
  fn shown(sent: &mut Sent, parameter: f32) -> f32 {
    sent.look(Pitch, parameter);
    sent.shown(Pitch, parameter)
  }

  #[test]
  fn with_nothing_set_the_parameter_shows_wherever_it_goes() {
    let mut sent = sent();
    for parameter in [0.5, 0.7, 0.7, 0.2] {
      assert_eq!(shown(&mut sent, parameter), parameter);
    }
  }

  #[test]
  fn what_was_set_shows_until_the_parameter_gets_there() {
    let mut sent = sent();
    for value in [0.1, 0.2, 0.3] {
      sent.set(Pitch, value);
    }
    // not yet moved, and then through the values in the order they were set
    for parameter in [0.5, 0.5, 0.1, 0.2] {
      assert_eq!(shown(&mut sent, parameter), 0.3, "at {parameter}");
    }
    assert_eq!(shown(&mut sent, 0.3), 0.3);
    // and from there on it is the parameter that shows
    assert_eq!(shown(&mut sent, 0.9), 0.9);
    // none of which touches another control
    assert_eq!(sent.shown(Mix, 1.0), 1.0);
  }

  #[test]
  fn values_that_were_skipped_on_the_way_are_settled_with_the_one_that_came() {
    // the parameter is looked at now and then, and may have taken several values between
    let mut sent = sent();
    for value in [0.1, 0.2, 0.3, 0.4] {
      sent.set(Pitch, value);
    }
    assert_eq!(shown(&mut sent, 0.3), 0.4);
    assert_eq!(sent.waiting[Pitch as usize], [0.4]);
    assert_eq!(shown(&mut sent, 0.4), 0.4);
    assert!(sent.waiting[Pitch as usize].is_empty());
  }

  #[test]
  fn the_hosts_own_value_takes_over_at_once() {
    let mut sent = sent();
    sent.set(Pitch, 0.4);
    assert_eq!(shown(&mut sent, 0.5), 0.4);
    assert_eq!(shown(&mut sent, 0.7), 0.7);
    assert!(sent.waiting[Pitch as usize].is_empty());
    assert_eq!(shown(&mut sent, 0.4), 0.4);
    assert_eq!(shown(&mut sent, 0.45), 0.45);
  }

  #[test]
  fn a_value_set_where_the_parameter_already_is_leaves_nothing_waiting() {
    // the wrapper reports no change for it, and the parameter never moves to show it came
    let mut sent = sent();
    sent.set(Pitch, 0.5);
    assert_eq!(shown(&mut sent, 0.5), 0.5);
    assert!(sent.waiting[Pitch as usize].is_empty());
    // dragged away and back before the parameter was looked at again
    sent.set(Pitch, 0.6);
    sent.set(Pitch, 0.5);
    assert_eq!(shown(&mut sent, 0.5), 0.5);
    assert!(sent.waiting[Pitch as usize].is_empty());
    // so the host putting it where the drag had passed is the host's and shows
    assert_eq!(shown(&mut sent, 0.6), 0.6);
  }

  #[test]
  fn the_host_moving_through_a_value_still_waiting_does_not_hold_the_panel_there() {
    // set 0.2 then 0.3, with only 0.2 arrived: the host now takes the parameter through 0.3
    // and on. the 0.3 counts as the editor's own arriving, and what follows is the host's
    let mut sent = sent();
    sent.set(Pitch, 0.2);
    sent.set(Pitch, 0.3);
    assert_eq!(shown(&mut sent, 0.2), 0.3);
    assert_eq!(shown(&mut sent, 0.3), 0.3);
    for parameter in [0.35, 0.4, 0.2, 0.3] {
      assert_eq!(shown(&mut sent, parameter), parameter);
    }
  }

  #[test]
  fn a_host_that_takes_nothing_does_not_fill_the_room() {
    let mut sent = sent();
    for step in 0..ROOM * 3 {
      sent.set(Mix, step as f32);
    }
    assert_eq!(sent.waiting[Mix as usize].len(), ROOM);
    assert_eq!(sent.shown(Mix, 1.0), (ROOM * 3 - 1) as f32);
  }
}
