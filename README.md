# CanWeTalk

_baby, can we taaaaalk?_

[![ci](https://github.com/tetra-fox/canwetalk/actions/workflows/ci.yml/badge.svg)](https://github.com/tetra-fox/canwetalk/actions/workflows/ci.yml)
[![license](https://img.shields.io/github/license/tetra-fox/canwetalk)](LICENSE)

A pitch and formant shifter plugin for a monophonic input, played by MIDI notes. VST3 and CLAP.

CanWeTalk detects the pitch of its input and resynthesizes it from grains two input periods
long. Each held note gets a voice at that note's pitch, up to 16 at once. While no note is held,
one voice follows the input's own pitch. The spacing of the grains sets the pitch and the speed
they are read at sets the formants, so the two move separately.

The plugin adds no latency and runs on mono and stereo tracks. Its editor has a control for
each parameter and shows the input's spectrum and the note nearest its detected pitch.

https://github.com/user-attachments/assets/6167cec1-cfab-46a3-8f7f-073d8812b617

## Playing it

- In Off, Mono and Poly the input sounds through while no note is held, at its own pitch. Mono
  Gate and Poly Gate sound only while a note plays, shaped by the envelope.
- The Poly modes play up to 16 notes. A note that finds no free voice takes the one that has had
  its note longest, and the note it replaces comes back when a voice is free.
- The detection ranges look for 25 to 700 Hz (Bass), 40 to 1000 Hz (Mid) and 40 to 2000 Hz
  (High).
- Pitch bend on channel 1 bends every note by up to 2 semitones. On channels 2 to 16 it bends
  that channel's notes by up to the Bend setting.
- Channel pressure and CC 74 set the pressure and slide of a channel's notes. Per-note pressure,
  brightness and tuning from the host set those of one note.

## Building

Rust 1.100, pinned in `rust-toolchain.toml`, and [just](https://github.com/casey/just).
`nix develop` provides both, along with the MinGW linker for the Windows build.

```shell
just bundle          # target/bundled/canwetalk.vst3 and canwetalk.clap
just bundle-windows  # the same for x86_64-pc-windows-gnu
```
