# cw-trainer - Agent Guidelines

A radio-neutral Morse code (CW) trainer. It works with **any radio-cat-rs
radio** (TS-570D, FT-991A, IC-7100, ...) by reaching that radio's server as
a **cat-native client** (`--server host:port` = the server's console/native
port). It moved here from `ts570d cw`; `ts570d` no longer carries it.

## Safety (non-negotiable — violation is a blocking issue)
- **Never play audio to the system default output device.** On this station
  the default output IS the radio's ACC2 input: anything played there is
  transmitted. Audio goes only to an explicit `--audio-out audio:<name>` or
  the remembered device in `audio.json`, and never to a device on the deny
  list. With neither, refuse — do not fall back to the default.
- Never play test audio to the default device while developing either
  (manual runs, examples, tests). Tests use doubles, not sound cards.
- Copy practice never writes to the radio: it reads defaults once over the
  native protocol, drops the connection, then runs offline. A structural
  test (`tests/structure.rs`) enforces that only `src/remote.rs` names
  `cat_native` or a socket, and that no source (the library's included)
  names a radio write.
- Sending (keying RF) is a later, separate, gated feature: dummy load only,
  behind an explicit acknowledgment. Do not add any transmit path without
  an approved plan.

## Dependency rules (violation is a blocking issue)
- Depend only on radio-cat-rs's `cat-native`, `cat-morse`, `cat-signal`,
  `cat-signal-audio` (the library: `cat-morse` only). Never on a radio crate (ts570d's `radio`, ft991a,
  ic7100) and never directly on `cat-framework` or any `cat-transport-*`.
- Engine/protocol changes belong in radio-cat-rs (its ADR 0001 / rule 7),
  never a local fork or vendored copy.
- All four radio-cat-rs crates are pinned once, in the root
  `[workspace.dependencies]`, to one radio-cat-rs revision: **pinned by rev
  until radio-cat-rs is tagged, then by tag.** No `[patch]`, no path to a
  sibling checkout (`tests/structure.rs` enforces both).

## The session library, `cw-trainer-core/` (violation is a blocking issue)
- The copy session's logic lives here, for the CLI and for an app
  consumer alike: options, QSO planning, over audio, scoring, adaptive
  speed, records, and `CopySession`, which a caller drives step by step.
  The CLI (`src/session.rs`) only shows, plays, collects and stores.
- **Pure.** `[dependencies]` is exactly cat-morse, serde and thiserror; no
  build script; `#![forbid(unsafe_code)]`. No terminal, process, file,
  network, thread, clock, entropy or device code: the caller supplies the
  seed, the start time and the sample rate, and stores what comes back.
  `cw-trainer-core/tests/structure.rs` and CI's dependency-graph step
  enforce it; its word list only ever grows, and keeps every word of the
  CLI's lists.
- **Deterministic.** Code that feeds steps or records uses only IEEE
  `+ - * /` and rounding — no `ln`/`exp`/`powf`/`sqrt`/trig — so records
  and transcripts are identical on every platform (the golden fixtures
  must pass on the Windows MSVC job too).
- **Frozen formats.** `SessionRecord` is history.jsonl v1: fields are only
  appended, `#[serde(default)]`, written only when they differ from the
  default; never `deny_unknown_fields`. `CoreError` Display texts are
  stored in history and never change (each has a test).
- **Portable.** It must check for aarch64-apple-ios, aarch64-apple-ios-sim,
  aarch64-apple-darwin, aarch64-linux-android, x86_64-linux-android and
  armv7-linux-androideabi with no SDK (CI's mobile job).
- **Audio safety is every consumer's duty**: the library never chooses or
  opens an output; its crate docs say so.
- Golden fixtures (`tests/fixtures/golden/`, and the shared records in
  `cw-trainer-core/tests/fixtures/records/`) pin the CLI's behaviour and
  files byte for byte; a refactor never changes them.

## Runtime
- **Tokio must NEVER be used.** The trainer has no async code: the
  cat-native client is blocking `std::net`. If async is ever needed, follow
  ts570d ADR 0006 (monoio on Linux only, hand-rolled executor on Windows);
  never a general-purpose async-executor crate.

## Platforms
- Linux and Windows. **Windows means `x86_64-pc-windows-msvc`** only
  (radio-cat-rs ADR 0012); CI's `windows-latest` job runs `cargo check`
  **and** `cargo test`. Local signal only:
  `cargo xwin check --target x86_64-pc-windows-msvc --workspace --all-targets`
  (one-time: `cargo install cargo-xwin --locked` and
  `rustup target add x86_64-pc-windows-msvc`).

## Essential commands
The repository is a workspace: the CLI at the root and the session
library in `cw-trainer-core/`; plain `cargo build`/`cargo test` cover both
(`default-members`).
- Build: `cargo build` (sound card: `--features audio-device`, needs ALSA
  headers on Linux)
- Test: `cargo test --workspace` (golden fixtures in `tests/fixtures/`;
  re-record only for a deliberate behaviour change, `CW_TRAINER_BLESS=1`)
- Lint: `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  `cargo fmt --all --check`
- Library portability (no SDK needed; `rustup target add` each once):
  `cargo check -p cw-trainer-core --lib --target <t>` for
  aarch64-apple-ios, aarch64-apple-ios-sim, aarch64-apple-darwin,
  aarch64-linux-android, x86_64-linux-android, armv7-linux-androideabi

## Workflow (MANDATORY)
- Planning-with-files: each agent keeps `task_plan.md`, `findings.md`,
  `progress.md` in its own `planning/<agent_name>/` and edits no other
  agent's directory. Plans are written before code and reviewed by the
  architect (and user) before work proceeds; one task at a time, reporting
  after each.
- TDD: failing test first, then the implementation; frequent commits;
  verify (build, test, clippy, fmt, Windows check) before claiming done.
- Unit tests use **doubles** (fakes, scripted sources, `cat_native::testing`
  stubs) — never a real radio, server, serial port or sound card.

## Code style
- Imports: std → external → local.
- Errors: `thiserror` + `Result<T, E>`.
- Every source file carries the Apache-2.0 header (see `src/lib.rs`).
- snake_case / PascalCase conventions; `cargo fmt`.
