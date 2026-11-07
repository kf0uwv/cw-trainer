// Copyright 2026 Matt Franklin
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Structural guards for copy mode's zero-RF promise.
//!
//! Read the sources at test time, so a file added later is scanned without
//! anyone remembering to add it:
//!
//! 1. Only `src/remote.rs` may name the native protocol, a socket, or any
//!    radio/transport crate. Everything else gets plain data.
//! 2. Nowhere — `remote.rs` included — may name anything that changes the
//!    radio: a protocol command, a command sink, or a CW/PTT write. The
//!    only reads `remote.rs` needs are the handshake's capabilities and a
//!    state read, neither of which takes a `Command`.
//! 3. `Cargo.toml` depends on no radio, transport, framework or tokio crate.
//! 4. The session library, `cw-trainer-core/`, is held to rules 1–3 with
//!    no exception at all (its own `tests/structure.rs` adds stricter ones,
//!    and must keep every word listed here).
//! 5. Every Rust file in both crates, tests included, starts with the
//!    Apache-2.0 header.
//!
//! Comments count: a forbidden word in a comment fails too. Rephrase the
//! comment; do not weaken the list.

use std::fs;
use std::path::{Path, PathBuf};

/// Allowed only in `src/remote.rs`.
const ONLY_IN_REMOTE: &[&str] = &[
    "cat_native",
    "std::net",
    "TcpStream",
    "TcpListener",
    "UdpSocket",
    "ToSocketAddrs",
    "SocketAddr",
];

/// Allowed nowhere under `src/`.
const NOWHERE: &[&str] = &[
    // Radio and transport crates (ts570d's `radio`, the CAT engine, the
    // transports): this program reaches a radio only through its server.
    "radio::",
    "Ts570d",
    "CatSession",
    "cat_transport",
    "cat_framework",
    "cat_client",
    "cat_server",
    "cat_rigctl",
    "tokio",
    // Anything that sends a command to the server. Reads go through the
    // handshake's capabilities and `read_state`, which take no command.
    "Command::",
    "Command {",
    ".command(",
    "CommandSink",
    "Session::connect",
    // Every state-changing cat-native command, by name (including ADR
    // 0023's CW set), in case one is reached some other way.
    "SetFrequency",
    "SetMode",
    "SetSplit",
    "SetMemoryChannel",
    "SetFilterWidth",
    "SetIfShift",
    "Retune",
    "AttachDevice",
    "attach_device",
    "SetKeyerSpeed",
    "ArmCwTransmit",
    "SendCwText",
    "AbortCw",
    "DisarmCw",
    // The system default audio output: on this station it is the radio's
    // ACC2 input, so a sound played there can be transmitted.
    "open_default",
    // ts570d-era writes.
    "send_cw",
    "transmit(",
    "set_keyer_speed",
    "set_cw_pitch",
    "set_ptt",
];

/// Direct dependencies this program must never take.
const FORBIDDEN_DEPS: &[&str] = &[
    "radio",
    "ts570d",
    "ft991a",
    "ic7100",
    "cat-framework",
    "cat-client",
    "cat-server",
    "cat-rigctl",
    "cat-transport-core",
    "cat-transport-serial",
    "cat-transport-tcp",
    "cat-transport-udp",
    "cat-transport-rfc2217",
    "tokio",
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` file under `dir`, recursively, as (path relative to the
/// crate root, contents).
fn sources(dir: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let rel = p
                    .strip_prefix(root())
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, fs::read_to_string(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// `(file, word)` for each forbidden word found.
fn violations<'a>(files: &[(String, String)], words: &[&'a str]) -> Vec<(String, &'a str)> {
    let mut found = Vec::new();
    for (name, src) in files {
        for w in words {
            if src.contains(w) {
                found.push((name.clone(), *w));
            }
        }
    }
    found
}

fn src_files() -> Vec<(String, String)> {
    let files = sources(&root().join("src"));
    // Not vacuous: lib, main, and the trainer modules are all here.
    assert!(
        files.len() >= 10,
        "expected the trainer sources under src/, found {}: {:?}",
        files.len(),
        files.iter().map(|f| &f.0).collect::<Vec<_>>()
    );
    for must in [
        "src/lib.rs",
        "src/main.rs",
        "src/remote.rs",
        "src/session.rs",
    ] {
        assert!(files.iter().any(|f| f.0 == must), "{must} not scanned");
    }
    files
}

#[test]
fn only_remote_names_the_protocol_or_a_socket() {
    let outside: Vec<_> = src_files()
        .into_iter()
        .filter(|f| f.0 != "src/remote.rs")
        .collect();
    let v = violations(&outside, ONLY_IN_REMOTE);
    assert!(
        v.is_empty(),
        "only src/remote.rs may reach the radio's server: {v:?}"
    );
}

#[test]
fn nothing_names_a_radio_write_or_a_transport() {
    let v = violations(&src_files(), NOWHERE);
    assert!(
        v.is_empty(),
        "copy mode must have no path that changes the radio: {v:?}"
    );
}

#[test]
fn no_radio_transport_or_tokio_dependency() {
    for manifest in [root().join("Cargo.toml"), core_root().join("Cargo.toml")] {
        let text = fs::read_to_string(&manifest).unwrap();
        let mut found = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            let Some((key, _)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim().trim_matches('"');
            if FORBIDDEN_DEPS.contains(&key) {
                found.push(key.to_string());
            }
        }
        assert!(
            found.is_empty(),
            "forbidden dependencies in {}: {found:?}",
            manifest.display()
        );
    }
}

fn core_root() -> PathBuf {
    root().join("cw-trainer-core")
}

fn core_src_files() -> Vec<(String, String)> {
    let files = sources(&core_root().join("src"));
    assert!(
        files.iter().any(|f| f.0 == "cw-trainer-core/src/lib.rs"),
        "cw-trainer-core/src/lib.rs not scanned: {:?}",
        files.iter().map(|f| &f.0).collect::<Vec<_>>()
    );
    files
}

#[test]
fn the_core_reaches_no_server_and_names_no_radio_write() {
    // No exception for the library: it never reaches a server at all.
    let files = core_src_files();
    let mut v = violations(&files, ONLY_IN_REMOTE);
    v.extend(violations(&files, NOWHERE));
    assert!(v.is_empty(), "the core must name none of these: {v:?}");
}

#[test]
fn the_cores_own_denylist_keeps_every_word_listed_here() {
    // The core's structure test is stricter, never looser: each word
    // above appears there too, as a string literal.
    let core_test = fs::read_to_string(core_root().join("tests/structure.rs")).unwrap();
    let missing: Vec<_> = ONLY_IN_REMOTE
        .iter()
        .chain(NOWHERE)
        .filter(|w| !core_test.contains(&format!("{w:?}")))
        .collect();
    assert!(
        missing.is_empty(),
        "cw-trainer-core/tests/structure.rs lacks: {missing:?}"
    );
}

/// The first lines of every Rust file in this repository.
const HEADER: &str = "\
// Copyright 2026 Matt Franklin
//
// Licensed under the Apache License, Version 2.0 (the \"License\");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an \"AS IS\" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
";

#[test]
fn every_rust_file_starts_with_the_apache_header() {
    let mut files = Vec::new();
    for dir in [
        "src",
        "tests",
        "cw-trainer-core/src",
        "cw-trainer-core/tests",
    ] {
        files.extend(sources(&root().join(dir)));
    }
    for extra in ["build.rs", "cw-trainer-core/build.rs"] {
        if let Ok(text) = fs::read_to_string(root().join(extra)) {
            files.push((extra.to_string(), text));
        }
    }
    for must in [
        "src/lib.rs",
        "tests/structure.rs",
        "cw-trainer-core/src/lib.rs",
        "cw-trainer-core/tests/structure.rs",
    ] {
        assert!(files.iter().any(|f| f.0 == must), "{must} not scanned");
    }
    let bad: Vec<_> = files
        .iter()
        .filter(|(_, text)| !text.replace("\r\n", "\n").starts_with(HEADER))
        .map(|(name, _)| name)
        .collect();
    assert!(bad.is_empty(), "missing the Apache-2.0 header: {bad:?}");
}

/// The one radio-cat-rs revision every crate here builds against.
const RADIO_CAT_RS: &str = "https://github.com/kf0uwv/radio-cat-rs";

#[test]
fn radio_cat_rs_is_pinned_once_by_rev_with_no_patch() {
    // Pinned by rev until radio-cat-rs tags a release with cat-morse, then
    // by tag: either way in one place, `[workspace.dependencies]`, with no
    // `[patch]` and no sibling checkout, so a git consumer resolves the
    // same sources this repository tests against.
    let manifest = fs::read_to_string(root().join("Cargo.toml")).unwrap();
    let mut section = "";
    let mut pins = Vec::new();
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            section = line;
            assert!(!line.starts_with("[patch"), "no [patch] section: {line}");
            continue;
        }
        assert!(
            !line.contains("../radio-cat-rs"),
            "no path to a sibling checkout: {line}"
        );
        if line.contains(RADIO_CAT_RS) {
            assert_eq!(
                section, "[workspace.dependencies]",
                "radio-cat-rs is named only in [workspace.dependencies]: {line}"
            );
            let rev = line
                .split("rev = \"")
                .nth(1)
                .and_then(|r| r.split('"').next())
                .unwrap_or_else(|| panic!("pinned by rev: {line}"));
            let name = line.split('=').next().unwrap().trim();
            pins.push((name.to_string(), rev.to_string()));
        }
    }
    let mut names: Vec<&str> = pins.iter().map(|p| p.0.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        ["cat-morse", "cat-native", "cat-signal", "cat-signal-audio"]
    );
    assert!(
        pins.windows(2).all(|w| w[0].1 == w[1].1),
        "one revision for every crate: {pins:?}"
    );
    assert_eq!(pins[0].1.len(), 40, "a full commit hash: {pins:?}");
}

#[test]
fn the_scanner_finds_what_it_is_looking_for() {
    // Guards the guards: a scanner that never matches would pass forever.
    let files = vec![
        (
            "src/a.rs".to_string(),
            "use cat_native::client;".to_string(),
        ),
        (
            "src/b.rs".to_string(),
            "conn.command(Command::SetMode { mode })".to_string(),
        ),
        ("src/c.rs".to_string(), "fn ok() {}".to_string()),
    ];
    assert_eq!(
        violations(&files, ONLY_IN_REMOTE),
        vec![("src/a.rs".to_string(), "cat_native")]
    );
    assert_eq!(
        violations(&files, NOWHERE),
        vec![
            ("src/b.rs".to_string(), "Command::"),
            ("src/b.rs".to_string(), ".command("),
            ("src/b.rs".to_string(), "SetMode"),
        ]
    );
}
