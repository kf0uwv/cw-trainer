# cw-trainer

A radio-neutral Morse code (CW) trainer for any radio supported by
[radio-cat-rs](https://github.com/kf0uwv/radio-cat-rs) (TS-570D, FT-991A,
IC-7100, ...). It reads the radio's CW pitch and keyer speed once from the
radio's server, then runs copy practice through a sound card you name.

**Status:** bootstrap. The trainer is moving here from `ts570d cw`.

Audio is only ever played to an explicitly chosen device, never the system
default (which, on many stations, feeds the radio's audio input).

## Layout

- `cw-trainer` (this directory): the command-line trainer — terminal,
  speaker, files, and the one read of the radio's defaults.
- [`cw-trainer-core`](cw-trainer-core/): the copy session as a library
  with no I/O, which builds for desktop and mobile targets alike.

## Building

radio-cat-rs is fetched by Cargo, pinned to one revision in `Cargo.toml`.

```
cargo build                          # no sound card support
cargo build --features audio-device  # needs ALSA headers on Linux
cargo test
```

## License

Apache-2.0. See [LICENSE.txt](LICENSE.txt).
