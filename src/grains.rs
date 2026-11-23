//! pitch-synchronous granular resynthesis. grains two input periods long are overlap-added
//! at the output pitch, so the grain spacing sets the pitch and the speed each grain is read
//! at sets the formants
//!
//! frequencies are in cycles per two samples, as in `shifter`

use std::f64::consts::TAU;

use crate::ring::Ring;

const MAX_GRAINS: usize = 128;
const OUTPUT_PITCH: std::ops::RangeInclusive<f64> = 1e-11..=0.25;
/// octaves a voice at the input pitch is raised by until its grains line up with the input
pub(crate) const UNITY_DRIFT: f64 = 0.001;
/// a grain starting within this many samples of an input period counts as lined up
const UNITY_REACH: f64 = 4.0;
/// how much of the way the grains fade in or out against the plain input each sample
const WET_STEP: f64 = 0.0125;

/// how the grains differ from the input
pub(crate) struct Shape {
  /// octaves above the target pitch
  pub(crate) pitch: f64,
  /// octaves the formants are moved by
  pub(crate) formant: f64,
  /// 0 to 1, how far grains stretch to keep overlapping when the pitch is lowered
  pub(crate) smooth: f64,
}

/// the grains of one voice
pub(crate) struct Engine {
  /// position in the input period, 0 to 1
  input_phase: f64,
  /// position in the output period, -1 to 1. a grain starts each time it wraps
  output_phase: f64,
  /// the output pitch as of the last grain
  output_step: f64,
  /// whether the grains line up with the input periods and read it undelayed
  locked: bool,
  /// how much of the output is grains, the rest is the plain input
  wet: f64,
  grains: Vec<Grain>,
}

struct Grain {
  /// how far behind the newest input the grain reads, less `extra`
  delay: f64,
  extra: f64,
  /// change of `extra` per sample, which makes the read speed `1 - slope`
  slope: f64,
  window_phase: f64,
  window_step: f64,
}

impl Engine {
  pub(crate) fn new() -> Self {
    Self {
      input_phase: 0.0,
      output_phase: -1.0,
      output_step: 0.0,
      locked: false,
      wet: 0.0,
      grains: Vec::with_capacity(MAX_GRAINS),
    }
  }

  pub(crate) fn reset(&mut self) {
    self.grains.clear();
    *self = Self {
      grains: std::mem::take(&mut self.grains),
      ..Self::new()
    };
  }

  /// one sample of the voice, sounding at `target` an input whose pitch is `input_pitch`
  pub(crate) fn tick(
    &mut self,
    input_pitch: f64,
    target: f64,
    shape: &Shape,
    input: &[Ring; 2],
  ) -> [f32; 2] {
    // at the input pitch the grains only rebuild the input, so they are moved onto the input
    // periods and then faded out for the input itself
    let unity = target == input_pitch && shape.pitch == 0.0;
    if !unity && self.locked {
      self.locked = false;
      self.output_phase = 2.0 * self.input_phase - 1.0;
    }
    let drift = if unity && !self.locked {
      UNITY_DRIFT
    } else {
      0.0
    };
    let pitch =
      (2f64.powf(shape.pitch + drift) * target).clamp(*OUTPUT_PITCH.start(), *OUTPUT_PITCH.end());

    self.input_phase += 0.5 * input_pitch;
    if self.input_phase >= 1.0 {
      self.input_phase -= 1.0;
    }
    self.output_phase += self.output_step;
    let wrapped = self.output_phase >= 1.0;
    if wrapped {
      self.output_phase -= 2.0;
    }
    if wrapped || self.grains.is_empty() {
      self.output_step = pitch;
    }

    if wrapped && self.grains.len() < MAX_GRAINS {
      let rate = 2f64.powf(shape.formant);
      // back to where the input period began, so grains start whole periods apart
      let mut delay = self.input_phase / (0.5 * input_pitch);
      if unity && delay < UNITY_REACH {
        self.locked = true;
      }
      if self.locked {
        delay = 0.0;
      }
      // two input periods at the read speed. below that pitch grains would leave gaps, which
      // smooth closes by stretching them toward two output periods
      let natural = rate * input_pitch;
      let window_step = if pitch <= natural {
        0.25 * (shape.smooth * pitch + (1.0 - shape.smooth) * natural)
      } else {
        0.25 * natural
      };
      self.grains.push(Grain {
        delay,
        // reading faster than the input arrives needs a head start of what it will gain
        extra: if rate > 1.0 {
          (rate - 1.0) / window_step
        } else {
          0.0
        },
        slope: rate - 1.0,
        // the wrap fell between samples, the grain is that far in already
        window_phase: (self.output_phase + 1.0) / pitch * window_step,
        window_step,
      });
    }

    let mut grains = [0.0; 2];
    self.grains.retain_mut(|grain| {
      let hann = (0.5 - 0.5 * (TAU * grain.window_phase).cos()) as f32;
      for (sum, line) in grains.iter_mut().zip(input) {
        *sum += hann * line.read(grain.delay + grain.extra);
      }
      grain.extra -= grain.slope;
      grain.window_phase += grain.window_step;
      grain.window_phase <= 1.0
    });

    let active = !(self.locked && shape.formant == 0.0);
    self.wet += WET_STEP * (f64::from(u8::from(active)) - self.wet);
    let wet = self.wet as f32;
    let mut out = [0.0; 2];
    for ((out, grains), line) in out.iter_mut().zip(grains).zip(input) {
      *out = wet * grains + (1.0 - wet) * line.ago(0);
    }
    out
  }
}
