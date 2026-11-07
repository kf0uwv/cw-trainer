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
//! 3. `src/lib.rs` forbids `unsafe` code, and the manifest has no
//!    `[features]`.
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
    // The same, called as associated functions.
    "f64::sqrt",
    "f64::powf",
    "f64::powi",
    "f64::exp",
    "f64::ln",
    "f64::log",
    "f64::sin",
    "f64::cos",
    "f64::tan",
    "f32::sqrt",
    "f32::powf",
    "f32::powi",
    "f32::exp",
    "f32::ln",
    "f32::log",
    "f32::sin",
    "f32::cos",
    "f32::tan",
    // A grouped import hides which module it reaches into
    // (`std::{fs, net}`): name each module on its own line instead.
    "std::{",
    // Hash maps iterate in a per-process random order, so anything
    // derived from iterating one differs from run to run.
    "HashMap",
    "HashSet",
    "RandomState",
    // The build environment: the library's output must not depend on
    // where or by whom it was compiled.
    "env!(",
    "option_env!(",
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
    manifest_keys_of(&fs::read_to_string(root().join("Cargo.toml")).unwrap())
}

fn manifest_keys_of(text: &str) -> Vec<(String, String)> {
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

/// The manifest's problems beyond its dependency list: features, which
/// could switch on code no test builds.
fn manifest_problems(keys: &[(String, String)]) -> Vec<String> {
    keys.iter()
        .filter(|(s, _)| s == "features" || s.starts_with("features."))
        .map(|(s, k)| format!("[{s}] {k}"))
        .collect()
}

#[test]
fn has_no_features() {
    assert_eq!(manifest_problems(&manifest_keys()), Vec::<String>::new());
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

#[test]
fn the_scanner_catches_every_way_round_the_list() {
    // Negative controls: each of these must be found.
    let cases: &[(&str, &str)] = &[
        ("use std::{fs, io};", "std::{"),
        ("let r = f64::sqrt(x);", "f64::sqrt"),
        ("let r = f64::powf(x, 2.0);", "f64::powf"),
        ("let r = f64::powi(x, 2);", "f64::powi"),
        ("let r = f64::exp(x);", "f64::exp"),
        ("let r = f64::ln(x);", "f64::ln"),
        ("let r = f64::log10(x);", "f64::log"),
        ("let r = f64::sin(x);", "f64::sin"),
        ("let r = f64::cos(x);", "f64::cos"),
        ("let r = f64::tan(x);", "f64::tan"),
        ("let r = f32::sqrt(x);", "f32::sqrt"),
        ("let r = f32::powf(x, 2.0);", "f32::powf"),
        ("let r = f32::powi(x, 2);", "f32::powi"),
        ("let r = f32::exp(x);", "f32::exp"),
        ("let r = f32::ln(x);", "f32::ln"),
        ("let r = f32::log2(x);", "f32::log"),
        ("let r = f32::sin(x);", "f32::sin"),
        ("let r = f32::cos(x);", "f32::cos"),
        ("let r = f32::tan(x);", "f32::tan"),
        ("let m: HashMap<u8, u8> = Default::default();", "HashMap"),
        ("let m: HashSet<u8> = Default::default();", "HashSet"),
        ("let h = RandomState::new();", "RandomState"),
        ("const V: &str = env!(\"CARGO_PKG_VERSION\");", "env!("),
        (
            "const V: Option<&str> = option_env!(\"HOME\");",
            "option_env!(",
        ),
    ];
    for (src, word) in cases {
        let files = vec![("src/x.rs".to_string(), src.to_string())];
        let found = violations(&files, FORBIDDEN);
        assert!(
            found.iter().any(|(_, w)| w == word),
            "{src:?} should be caught as {word:?}, found {found:?}"
        );
    }
    let keys = manifest_keys_of("[package]\nname = \"x\"\n\n[features]\nextra = []\n");
    assert_eq!(
        manifest_problems(&keys),
        vec!["[features] ".to_string(), "[features] extra".to_string()]
    );
}
