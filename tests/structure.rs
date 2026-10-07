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
    let manifest = fs::read_to_string(root().join("Cargo.toml")).unwrap();
    let mut found = Vec::new();
    for line in manifest.lines() {
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
    assert!(found.is_empty(), "forbidden dependencies: {found:?}");
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
