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

//! Structural guards for the session library.
//!
//! The library is pure: it is handed a seed, a start time, a sample rate
//! and the operator's copy, and it hands back steps, audio to render and
//! records. It builds for desktop and mobile targets alike, so it must not
//! reach a terminal, a file, the network, a clock, a thread, entropy or a
//! sound card, and the numbers it records must come out the same on every
//! platform. These tests read the manifest and sources at test time:
//!
//! 1. `[dependencies]` is exactly cat-morse, serde and thiserror; there are
//!    no build, target-specific or optional dependencies and no build
//!    script.
//! 2. No source names anything on [`FORBIDDEN`].
//! 3. `src/lib.rs` forbids `unsafe` code.
//!
//! Comments count: a forbidden word in a comment fails too. Rephrase the
//! comment; never weaken a list (the repository's root structure test
//! checks this list still holds every word of its own).

use std::fs;
use std::path::{Path, PathBuf};

/// Exactly the library's direct dependencies.
const ALLOWED_DEPS: &[&str] = &["cat-morse", "serde", "thiserror"];

/// Allowed nowhere under `src/`.
const FORBIDDEN: &[&str] = &[
    // Terminal and process I/O: what to show is the caller's business.
    "println!",
    "eprintln!",
    "print!",
    "eprint!",
    "dbg!",
    "stdin",
    "stdout",
    "stderr",
    "std::io",
    "std::env",
    "std::process",
    // Files, network, threads.
    "std::fs",
    "std::net",
    "std::thread",
    "thread::spawn",
    // Clocks and entropy: the caller supplies the seed and the start time.
    "Instant",
    "SystemTime",
    "UNIX_EPOCH",
    "rand::",
    "getrandom",
    // Front-end, device, logging and async crates.
    "crossterm",
    "ctrlc",
    "cat_signal",
    "cat_signal_audio",
    "cpal",
    "tracing",
    "serde_json",
    // Floating point beyond IEEE + - * / and rounding: these may round
    // differently from one platform's maths library to another's, and the
    // records must be identical everywhere.
    ".ln(",
    ".log(",
    ".log2(",
    ".log10(",
    ".ln_1p(",
    ".exp(",
    ".exp2(",
    ".exp_m1(",
    ".powf(",
    ".powi(",
    ".sqrt(",
    ".cbrt(",
    ".hypot(",
    ".sin(",
    ".cos(",
    ".tan(",
    ".sin_cos(",
    ".asin(",
    ".acos(",
    ".atan(",
    ".atan2(",
    ".sinh(",
    ".cosh(",
    ".tanh(",
    ".mul_add(",
    // The CLI's lists, kept whole here: only its remote module may reach
    // a server, and nothing anywhere may change a radio.
    "cat_native",
    "TcpStream",
    "TcpListener",
    "UdpSocket",
    "ToSocketAddrs",
    "SocketAddr",
    "radio::",
    "Ts570d",
    "CatSession",
    "cat_transport",
    "cat_framework",
    "cat_client",
    "cat_server",
    "cat_rigctl",
    "tokio",
    "Command::",
    "Command {",
    ".command(",
    "CommandSink",
    "Session::connect",
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
    "open_default",
    "send_cw",
    "transmit(",
    "set_keyer_speed",
    "set_cw_pitch",
    "set_ptt",
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

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

/// The manifest as `(section, key)` pairs, comments dropped.
fn manifest_keys() -> Vec<(String, String)> {
    let text = fs::read_to_string(root().join("Cargo.toml")).unwrap();
    let mut section = String::new();
    let mut keys = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            section = line.trim_matches(|c| c == '[' || c == ']').to_string();
            keys.push((section.clone(), String::new()));
            continue;
        }
        if let Some((key, _)) = line.split_once('=') {
            keys.push((section.clone(), key.trim().trim_matches('"').to_string()));
        }
    }
    keys
}

#[test]
fn depends_on_exactly_cat_morse_serde_and_thiserror() {
    let keys = manifest_keys();
    let mut deps: Vec<&str> = keys
        .iter()
        .filter(|(s, k)| s == "dependencies" && !k.is_empty())
        .map(|(_, k)| k.as_str())
        .chain(
            keys.iter()
                .filter_map(|(s, _)| s.strip_prefix("dependencies.")),
        )
        .collect();
    deps.sort();
    deps.dedup();
    assert_eq!(deps, ALLOWED_DEPS);
    for (section, key) in &keys {
        assert!(
            !section.contains("build-dependencies"),
            "no build dependencies: [{section}]"
        );
        assert!(
            !section.starts_with("target."),
            "no target-specific dependencies: [{section}]"
        );
        assert!(
            !(section == "package" && key == "build"),
            "no build script setting"
        );
        assert!(
            key != "optional",
            "no optional dependencies: [{section}] {key}"
        );
    }
    let text = fs::read_to_string(root().join("Cargo.toml")).unwrap();
    assert!(
        !text.contains("optional = true"),
        "no optional dependencies"
    );
}

#[test]
fn has_no_build_script() {
    assert!(!root().join("build.rs").exists(), "no build.rs");
}

#[test]
fn names_nothing_forbidden() {
    let files = sources(&root().join("src"));
    assert!(
        files.iter().any(|f| f.0 == "src/lib.rs"),
        "src/lib.rs not scanned: {:?}",
        files.iter().map(|f| &f.0).collect::<Vec<_>>()
    );
    let v = violations(&files, FORBIDDEN);
    assert!(v.is_empty(), "the library must stay pure: {v:?}");
}

#[test]
fn forbids_unsafe_code() {
    let lib = fs::read_to_string(root().join("src/lib.rs")).unwrap();
    assert!(lib.contains("#![forbid(unsafe_code)]"));
}

#[test]
fn the_scanner_finds_what_it_is_looking_for() {
    let files = vec![
        (
            "src/a.rs".to_string(),
            "let t = std::time::Instant::now(); println!(\"{t:?}\");".to_string(),
        ),
        ("src/b.rs".to_string(), "let y = x.powf(2.0);".to_string()),
        (
            "src/c.rs".to_string(),
            "fn ok() -> f64 { 1.0 / 3.0 }".to_string(),
        ),
    ];
    assert_eq!(
        violations(&files, FORBIDDEN),
        vec![
            ("src/a.rs".to_string(), "println!"),
            ("src/a.rs".to_string(), "Instant"),
            ("src/b.rs".to_string(), ".powf("),
        ]
    );
}
