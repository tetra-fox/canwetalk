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

## Parameters

| Group      | Parameter         | Range             | Default | Description                                                       |
| ---------- | ----------------- | ----------------- | ------- | ----------------------------------------------------------------- |
| Shift      | Pitch             | -24 to 24 st      | 0       | Shift of every voice from its note or from the input pitch        |
| Shift      | Formant           | -24 to 24 st      | 0       | Shift of the formants                                             |
| Shift      | Formant Follow    | 0 to 100%         | 0       | Share of the pitch change the formants move along with            |
| Shift      | Smooth            | 0 to 100%         | 0       | Stretch of the grains so they keep overlapping at lowered pitches |
| Effects    | Detune            | 0 to 100%         | 0       | Slowly lowered and raised copies of each side                     |
| Effects    | Smear             | 0 to 100%         | 0       | Freeze of the input into loops two periods long                   |
| Effects    | Stereo            | 0 to 100%         | 0       | Widening of a stereo track                                        |
| Vibrato    | Vibrato           | 0 to 200 ct       | 0       | Depth, either way                                                 |
| Vibrato    | Vibrato Rate      | 2 to 15 Hz        | 5       | Rate without sync                                                 |
| Vibrato    | Vibrato Sync      | on, off           | off     | Rate from the host's tempo                                        |
| Vibrato    | Vibrato Division  | 1/1 to 1/32       | 1/16    | Length of a synced cycle                                          |
| Notes      | MIDI              | see [MIDI](#midi) | Poly    | Mode notes are played in                                          |
| Notes      | Latch             | on, off           | off     | Notes sound on after their keys come up                           |
| Notes      | Glide             | 0 to 100%         | 0       | Time the pitch takes to settle, about a second at 100%            |
| Notes      | Glide Notes Only  | on, off           | off     | Glide between notes, with the input pitch followed as it is       |
| Envelope   | Attack            | 0 to 10000 ms     | 5       | Rise to full level                                                |
| Envelope   | Decay             | 0 to 10000 ms     | 200     | Fall to the sustain level                                         |
| Envelope   | Sustain           | 0 to 100%         | 100     | Level while the note is held                                      |
| Envelope   | Release           | 0 to 10000 ms     | 200     | Fall once the note is let go                                      |
| Expression | Pressure to Level | 0 to 100%         | 0       | Share of a note's level its pressure sets                         |
| Expression | Slide to Formant  | -24 to 24 st      | 0       | Formant shift at either end of a note's slide                     |
| Expression | MPE Bend Range    | 1 to 96 st        | 48      | Reach of pitch bend on channels 2 to 16                           |
|            | Detection Range   | High, Mid, Bass   | Mid     | Span of input pitches looked for                                  |
|            | Anchor Grains     | on, off           | on      | Grains start where the input's cycles do, found from its waveform |
|            | Mix               | 0 to 100%         | 100     | Shifted signal against the input                                  |

- The envelope shapes notes in the two gate modes only.
- Latched notes sound until the next key pressed with none down.
- A synced vibrato starts a cycle on each division of the bar while the host plays. The divisions
  include 1/4T, 1/8T and 1/16T.
- The detection ranges are 25 to 700 Hz for Bass, 40 to 1000 Hz for Mid and 40 to 2000 Hz for
  High.
- Formants move 4 octaves at most, either way.

## MIDI

| Mode      | Description                                                                        |
| --------- | ---------------------------------------------------------------------------------- |
| Off       | Notes are ignored and one voice follows the input pitch                            |
| Mono      | One voice plays the latest note, or follows the input pitch while none is held     |
| Poly      | As Mono, with a voice for each held note, up to 16                                 |
| Mono Gate | As Mono, but nothing sounds without a note and notes rise and fall by the envelope |
| Poly Gate | As Poly, gated the same way                                                        |

- A note that finds no free voice takes the one that has had its note longest. The note it
  replaces comes back when a voice is free.
- A change of mode lets go of every note.
- Pitch bend on channel 1 bends every note by up to 2 semitones. On channels 2 to 16 it bends
  that channel's notes by up to the MPE Bend Range.
- Channel pressure and CC 74 set the pressure and slide of a channel's notes.
- Per-note pressure, brightness and tuning from the host set the pressure, slide and tuning of
  one note.

## Building

Rust 1.100, pinned in `rust-toolchain.toml`, and [just](https://github.com/casey/just).
`nix develop` provides both, along with the MinGW linker for the Windows build.

```shell
just bundle          # target/bundled/canwetalk.vst3 and canwetalk.clap
just bundle-windows  # the same for x86_64-pc-windows-gnu
```
