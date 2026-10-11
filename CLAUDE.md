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
  `cat_native` or a socket, and that no source names a radio write.
- Sending (keying RF) is a later, separate, gated feature: dummy load only,
  behind an explicit acknowledgment. Do not add any transmit path without
  an approved plan.

## Dependency rules (violation is a blocking issue)
- Depend only on radio-cat-rs's `cat-native`, `cat-morse`, `cat-signal`,
  `cat-signal-audio`. Never on a radio crate (ts570d's `radio`, ft991a,
  ic7100) and never directly on `cat-framework` or any `cat-transport-*`.
- Engine/protocol changes belong in radio-cat-rs (its ADR 0001 / rule 7),
  never a local fork or vendored copy.
- The `[patch."https://github.com/kf0uwv/radio-cat-rs"]` section points at
  the sibling checkout `../radio-cat-rs` and is TEMPORARY until cat-morse is
  in a tagged release — same arrangement as kf0uwv/ts570d.

## Runtime
- **Tokio must NEVER be used.** The trainer has no async code: the
  cat-native client is blocking `std::net`. If async is ever needed, follow
  ts570d ADR 0006 (monoio on Linux only, hand-rolled executor on Windows);
  never a general-purpose async-executor crate.

## Platforms
- Linux and Windows. **Windows means `x86_64-pc-windows-msvc`** only
  (radio-cat-rs ADR 0012); CI's `windows-latest` job runs `cargo check`
  **and** `cargo test`. Local signal only:
  `cargo xwin check --target x86_64-pc-windows-msvc --all-targets`
  (one-time: `cargo install cargo-xwin --locked` and
  `rustup target add x86_64-pc-windows-msvc`).

## Essential commands
- Build: `cargo build` (sound card: `--features audio-device`, needs ALSA
  headers on Linux)
- Test: `cargo test`
- Lint: `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo fmt --check`

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
