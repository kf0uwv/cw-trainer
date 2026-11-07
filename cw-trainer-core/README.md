# cw-trainer-core

Morse code copy practice as a library with no I/O: synthesized on-air
contacts (rag-chews, contest exchanges, POTA and SOTA activators, DX
pileups) through band noise, fading, static and QRM, grouped into overs,
scored per character, with adaptive speed and a serializable session
record. It is the session logic of the
[`cw-trainer`](https://github.com/kf0uwv/cw-trainer) command-line trainer,
split out so an app consumer can run the very same sessions.

Everything about Morse comes from
[cat-morse](https://github.com/kf0uwv/radio-cat-rs) (re-exported as
`cw_trainer_core::cat_morse`). The library has no terminal, sound card,
network, file, thread or clock code, and its only dependencies are
cat-morse, serde and thiserror, so it builds for iOS, Android, macOS,
Windows and Linux with no SDK.

## Depending on it

The library is not on crates.io; depend on it from git, pinned to a
commit of this repository:

```toml
[dependencies]
cw-trainer-core = { git = "https://github.com/kf0uwv/cw-trainer", rev = "<commit>" }
```

Use cat-morse **through the re-export**, `cw_trainer_core::cat_morse`, so
that its types (`CharStats`, `Score`, `Callsign`, `ChannelRenderer`, ...)
are the very ones the library uses. If you must also depend on cat-morse
directly, use exactly the spec this repository pins:

```toml
cat-morse = { git = "https://github.com/kf0uwv/radio-cat-rs", rev = "b821bf47ceb57b8f23e437ca3136cd745235e180", features = ["serde"] }
```

Cargo treats a `rev`, a `tag` and a `branch` of the same repository as
different sources: any other spelling links a second cat-morse whose types
do not match the library's. radio-cat-rs has not yet tagged a release
with cat-morse, so it is pinned by rev for now; once it is tagged, this
repository moves to the tag, and a consumer's direct spec must move with
it.

## Driving a session

A `CopySession` decides; the caller does everything that touches the
world.

```rust,ignore
use cw_trainer_core::{CopySession, OverOutcome, Step};

let mut session = CopySession::new(options, output_sample_rate)?;
while let Some(step) = session.next_step()? {
    match step {
        Step::QsoStart { qso, of, label } => { /* show a heading */ }
        Step::YouLine { text, hold } => { /* show the operator's line for `hold` */ }
        Step::ThemOver(over) => {
            // Play over.audio.renderer()? to the output the operator chose,
            // collect the copy, then:
            let report = session.submit(OverOutcome::Copied {
                text: copy, agn, audio_gaps, clipped,
            })?;
            // report.result is the recorded result, report.shown the alignment.
        }
        Step::QsoEnd { speed_change } => { /* save the stats */ }
    }
}
let finished = session.finish(None); // record + this session's stats
```

- `quit()` is valid at any step: the next step ends the open QSO, then
  `None`.
- AGN is `over.audio.renderer()` again: the PCM is identical.
- An output whose rate changes: `set_sample_rate`, from the next over.
- On the caller's own error (the output failed, say), call
  `finish(Some(error_text))`: what was scored is kept, marked
  `ended_by_error`. After a library error, only `finish` is left.
- `CopySession` and an over's audio are `Send`.

## Session rules

`CopyOptions::adapt` is the adaptive-speed rule (`None` keeps the speed
fixed); `AdaptRule::default()` is the CLI's rule. `CopyOptions::band`
replaces the level's band conditions (cat-morse's `BandConditions`).
`CopySession::new` refuses an unusable rule or band. A record notes a
non-default rule or band in trailing fields, and only then.

## Persistence is the caller's

The library returns plain serde types and stores nothing. `SessionRecord`
is one line of the CLI's `history.jsonl` (version 1); `CharStats` (from
cat-morse) merges into all-time stats with `CharStats::merge`. The record's
serialized form is frozen: fields are only ever appended, with defaults, so
older lines always read.

## Determinism

The library reads no clock and no entropy: the caller passes the seed and
the start time (`time::iso8601_utc` formats one) in `CopyOptions`. The same
options and the same outcomes give the same steps and the same record, bit
for bit, on every platform. Rendered PCM is identical per platform and
equal within float tolerance across platforms.

## Audio safety: every consumer's duty

The library produces audio to render; it never chooses or opens an output
device. **Never play it to a device the operator did not explicitly
choose.** On a radio station the system's default output may be wired to a
transmitter's audio input, so a sound played there can go out over the
air. The `cw-trainer` CLI plays only to an output the operator named (or
remembered), checks it against a deny list before and after opening it,
and refuses otherwise; enforce an equivalent rule on your platform.

## License

Apache-2.0. See [LICENSE.txt](../LICENSE.txt).
