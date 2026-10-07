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

//! Wiring a copy session together, in the one order that keeps it safe.
//!
//! 1. The data directory, with any deny-list edits applied.
//! 2. The speaker: chosen (never the system default), opened, and checked
//!    against the deny list by the name it actually opened as.
//! 3. Only then the radio's server, read once and let go (`remote`).
//! 4. The session, with no radio.
//!
//! A refused speaker therefore stops everything before any network
//! contact. The steps that touch the outside world are passed in, so the
//! order is tested with doubles.

use std::time::Duration;

use crate::audio::AudioSink;
use crate::cli::{AudioArgs, CopyArgs};
use crate::output;
use crate::store::{default_data_dir, Store};
use crate::time::iso8601_utc;
use crate::{resolve_tone_and_speed, CopyOptions, RadioDefaults};

/// Open the data directory and apply `--deny-audio`/`--undeny-audio`.
pub fn open_store(audio: &AudioArgs) -> Store {
    open_store_with(
        audio,
        default_data_dir(),
        crate::migrate::default_legacy_data_dir(),
        &mut |line| eprintln!("{line}"),
    )
}

/// [`open_store`] with the default and legacy directories given, so the
/// one-time carry-over from `ts570d cw` is testable. An explicit
/// `--data-dir` never migrates.
pub fn open_store_with(
    audio: &AudioArgs,
    default_dir: Option<std::path::PathBuf>,
    legacy_dir: Option<std::path::PathBuf>,
    report: &mut dyn FnMut(String),
) -> Store {
    let dir = match &audio.data_dir {
        Some(d) => Some(d.clone()),
        None => {
            if let (Some(new), Some(old)) = (&default_dir, &legacy_dir) {
                for line in crate::migrate::migrate(old, new).lines(old, new) {
                    report(line);
                }
            }
            default_dir
        }
    };
    let mut store = Store::open(dir);
    let mut prefs = store.audio_prefs().clone();
    if prefs.edit_deny(&audio.deny, &audio.undeny) {
        store.save_audio_prefs(prefs);
    }
    store
}

/// Steps 2 and 3: the speaker, then the server. Returns the opened sink
/// and the session's options. `report` gets each warning/note line.
///
/// `now` is time since the Unix epoch: the session's start stamp, and its
/// seed when `--seed` is not given.
pub fn prepare_copy<K, O, R>(
    args: &CopyArgs,
    store: &mut Store,
    open_sink: O,
    read_server: R,
    now: Duration,
    report: &mut dyn FnMut(String),
) -> Result<(K, CopyOptions), String>
where
    K: AudioSink,
    O: FnOnce(&str) -> Result<(K, String), String>,
    R: FnOnce(&str) -> RadioDefaults,
{
    // The speaker first: chosen, never the default, and a refusal comes
    // before anything else happens -- including any contact with the radio.
    let spec = output::choose(args.audio.audio_out.as_deref(), store.audio_prefs())?;
    let (sink, label) = open_sink(&spec)?;
    store.audio_prefs().check_opened(&label)?;
    let mut prefs = store.audio_prefs().clone();
    prefs.device = Some(spec);
    store.save_audio_prefs(prefs);

    // Only now the radio's server, once; it is gone when this returns.
    let radio = match &args.server {
        None => RadioDefaults::default(),
        Some(addr) => read_server(addr),
    };
    for w in &radio.warnings {
        report(format!("warning: {w}"));
    }
    for n in &radio.notes {
        report(n.clone());
    }
    let (wpm, pitch_hz) = resolve_tone_and_speed(args.wpm, args.pitch_hz, &radio);

    let opts = CopyOptions {
        kind: args.kind,
        level: args.level,
        qsos: args.qsos,
        call: args.call.clone(),
        wpm,
        farnsworth: args.farnsworth,
        pitch_hz,
        seed: args.seed.unwrap_or(now.as_nanos() as u64),
        adapt: args.adapt,
        started: iso8601_utc(now.as_secs()),
        audio_out: label,
    };
    Ok((sink, opts))
}

/// `cw-trainer devices`: list outputs, marking the chosen and denied.
pub fn run_devices(audio: &AudioArgs) {
    let mut store = open_store(audio);
    for w in store.take_warnings() {
        eprintln!("warning: {w}");
    }
    print!(
        "{}",
        output::describe_devices(&cat_signal_audio::output_devices(), store.audio_prefs())
    );
}

/// `cw-trainer copy`: the real speaker, server, terminal and session.
pub fn run_copy(args: &CopyArgs) -> Result<(), String> {
    let mut store = open_store(&args.audio);
    for w in store.take_warnings() {
        eprintln!("warning: {w}");
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let (mut sink, opts) = prepare_copy(
        args,
        &mut store,
        crate::sink::open_sink,
        crate::remote::read_server_defaults,
        now,
        &mut |line| eprintln!("{line}"),
    )?;
    eprintln!("\n>>> Playing to: {} <<<\n", opts.audio_out);
    let mut term = crate::term::CrosstermTerminal::new()
        .map_err(|e| format!("cannot put the terminal in raw mode: {e}"))?;
    crate::session::run_copy(&opts, &mut sink, &mut term, &mut store)
        .map(drop)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use cat_morse::Callsign;

    use super::*;
    use crate::audio::tests::FakeSink;
    use crate::{Farnsworth, Kind, Level};

    fn args(server: Option<&str>, audio_out: Option<&str>) -> CopyArgs {
        CopyArgs {
            server: server.map(String::from),
            kind: Kind::RagChew,
            level: Level::Beginner,
            qsos: 2,
            call: Callsign::parse("KF0UWV").unwrap(),
            wpm: None,
            farnsworth: Farnsworth::Off,
            pitch_hz: None,
            seed: Some(9),
            adapt: true,
            audio: AudioArgs {
                audio_out: audio_out.map(String::from),
                ..AudioArgs::default()
            },
        }
    }

    /// Runs prepare_copy with doubles that log what happened, in order.
    fn prepare(
        a: &CopyArgs,
        store: &mut Store,
        opened_as: Result<&str, &str>,
        server: RadioDefaults,
    ) -> (Result<CopyOptions, String>, Vec<String>, Vec<String>) {
        let log = RefCell::new(Vec::new());
        let mut lines = Vec::new();
        let r = prepare_copy(
            a,
            store,
            |spec: &str| {
                log.borrow_mut().push(format!("open {spec}"));
                opened_as
                    .map(|l| (FakeSink::new(8000), l.to_string()))
                    .map_err(String::from)
            },
            |addr: &str| {
                log.borrow_mut().push(format!("server {addr}"));
                server
            },
            Duration::from_secs(1_000_000_000),
            &mut |l| lines.push(l),
        )
        .map(|(_, o)| o);
        (r, log.into_inner(), lines)
    }

    fn tmp_store() -> (tempfile::TempDir, Store) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(Some(tmp.path().to_path_buf()));
        (tmp, store)
    }

    #[test]
    fn the_speaker_is_opened_before_the_server_is_contacted() {
        let (_t, mut store) = tmp_store();
        let a = args(Some("radio:4540"), Some("audio:Headphones"));
        let (r, log, _) = prepare(&a, &mut store, Ok("Headphones"), RadioDefaults::default());
        assert!(r.is_ok(), "{r:?}");
        assert_eq!(log, ["open audio:Headphones", "server radio:4540"]);
    }

    #[test]
    fn no_chosen_speaker_stops_before_opening_anything_or_the_network() {
        let (_t, mut store) = tmp_store();
        let (r, log, _) = prepare(
            &args(Some("radio:4540"), None),
            &mut store,
            Ok("x"),
            RadioDefaults::default(),
        );
        assert!(r.unwrap_err().contains("never used"));
        assert!(log.is_empty(), "{log:?}");
        for spec in ["audio:", "audio: "] {
            let (r, log, _) = prepare(
                &args(Some("radio:4540"), Some(spec)),
                &mut store,
                Ok("x"),
                RadioDefaults::default(),
            );
            assert!(r.is_err());
            assert!(log.is_empty(), "{log:?}");
        }
    }

    #[test]
    fn a_speaker_that_fails_to_open_stops_before_the_network() {
        let (_t, mut store) = tmp_store();
        let (r, log, _) = prepare(
            &args(Some("radio:4540"), Some("audio:Gone")),
            &mut store,
            Err("no such device"),
            RadioDefaults::default(),
        );
        assert!(r.unwrap_err().contains("no such device"));
        assert_eq!(log, ["open audio:Gone"]);
    }

    #[test]
    fn a_speaker_that_opens_as_a_denied_device_stops_before_the_network() {
        let (_t, mut store) = tmp_store();
        let mut prefs = store.audio_prefs().clone();
        prefs.edit_deny(&["USB PnP".into()], &[]);
        store.save_audio_prefs(prefs);
        // Asked for by an innocent-looking name; opened as the radio's.
        let (r, log, _) = prepare(
            &args(Some("radio:4540"), Some("audio:hw:2")),
            &mut store,
            Ok("C-Media USB PnP Sound Device"),
            RadioDefaults::default(),
        );
        assert!(r.unwrap_err().contains("deny list"));
        assert_eq!(log, ["open audio:hw:2"]);
        assert_eq!(
            store.audio_prefs().device,
            None,
            "a refused device is not remembered"
        );
    }

    #[test]
    fn an_opened_speaker_is_remembered() {
        let (tmp, mut store) = tmp_store();
        let a = args(None, Some("audio:Headphones"));
        prepare(&a, &mut store, Ok("Headphones"), RadioDefaults::default())
            .0
            .unwrap();
        let reopened = Store::open(Some(tmp.path().to_path_buf()));
        assert_eq!(
            reopened.audio_prefs().device.as_deref(),
            Some("audio:Headphones")
        );
    }

    #[test]
    fn without_a_server_the_network_is_never_touched() {
        let (_t, mut store) = tmp_store();
        let (r, log, _) = prepare(
            &args(None, Some("audio:Headphones")),
            &mut store,
            Ok("Headphones"),
            RadioDefaults::default(),
        );
        let o = r.unwrap();
        assert_eq!(log, ["open audio:Headphones"]);
        assert_eq!(
            (o.wpm, o.pitch_hz),
            (crate::DEFAULT_WPM, crate::DEFAULT_PITCH_HZ)
        );
    }

    #[test]
    fn the_server_sets_defaults_and_explicit_flags_win() {
        let (_t, mut store) = tmp_store();
        let server = || RadioDefaults {
            pitch_hz: Some(750.0),
            wpm: Some(24),
            warnings: vec!["w1".into()],
            notes: vec!["n1".into()],
        };
        let a = args(Some("radio:4540"), Some("audio:Headphones"));
        let (r, _, lines) = prepare(&a, &mut store, Ok("Headphones"), server());
        let o = r.unwrap();
        assert_eq!((o.wpm, o.pitch_hz), (24, 750.0));
        assert!(lines.iter().any(|l| l == "warning: w1"), "{lines:?}");
        assert!(lines.iter().any(|l| l == "n1"), "{lines:?}");

        let mut a = a;
        a.wpm = Some(18);
        a.pitch_hz = Some(650.0);
        let (r, _, _) = prepare(&a, &mut store, Ok("Headphones"), server());
        let o = r.unwrap();
        assert_eq!((o.wpm, o.pitch_hz), (18, 650.0));
    }

    #[test]
    fn options_carry_the_args_the_opened_label_and_the_time() {
        let (_t, mut store) = tmp_store();
        let a = args(None, Some("audio:hw:1"));
        let (r, _, _) = prepare(&a, &mut store, Ok("Headphones"), RadioDefaults::default());
        let o = r.unwrap();
        assert_eq!(o.audio_out, "Headphones");
        assert_eq!(o.seed, 9);
        assert_eq!(o.started, "2001-09-09T01:46:40Z");
        assert_eq!(
            (o.kind, o.level, o.qsos),
            (Kind::RagChew, Level::Beginner, 2)
        );
        assert_eq!(o.call.as_str(), "KF0UWV");
    }

    #[test]
    fn deny_edits_are_applied_and_saved_on_open() {
        let tmp = tempfile::tempdir().unwrap();
        let audio = AudioArgs {
            deny: vec!["PnP".into()],
            data_dir: Some(tmp.path().to_path_buf()),
            ..AudioArgs::default()
        };
        let store = open_store(&audio);
        assert_eq!(store.audio_prefs().deny, vec!["PnP".to_string()]);
        let again = Store::open(Some(tmp.path().to_path_buf()));
        assert_eq!(again.audio_prefs().deny, vec!["PnP".to_string()]);
    }

    fn legacy_with_deny(root: &std::path::Path) -> std::path::PathBuf {
        let old = root.join("ts570d").join("cw");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(
            old.join("audio.json"),
            "{\"version\":1,\"device\":null,\"deny\":[\"USB PnP\"]}",
        )
        .unwrap();
        old
    }

    #[test]
    fn the_first_run_carries_the_old_deny_list_over() {
        let tmp = tempfile::tempdir().unwrap();
        let old = legacy_with_deny(tmp.path());
        let new = tmp.path().join("cw-trainer");
        let mut lines = Vec::new();
        let store = open_store_with(
            &AudioArgs::default(),
            Some(new.clone()),
            Some(old.clone()),
            &mut |l| lines.push(l),
        );
        assert_eq!(store.audio_prefs().deny, vec!["USB PnP".to_string()]);
        assert!(new.join("audio.json").exists());
        assert!(old.join("audio.json").exists(), "copied, not moved");
        assert!(
            lines.iter().any(|l| l.contains("copied audio.json")),
            "{lines:?}"
        );
    }

    #[test]
    fn an_explicit_data_dir_never_migrates() {
        let tmp = tempfile::tempdir().unwrap();
        let old = legacy_with_deny(tmp.path());
        let new = tmp.path().join("cw-trainer");
        let chosen = tmp.path().join("mine");
        let mut lines = Vec::new();
        let store = open_store_with(
            &AudioArgs {
                data_dir: Some(chosen.clone()),
                ..AudioArgs::default()
            },
            Some(new.clone()),
            Some(old),
            &mut |l| lines.push(l),
        );
        assert!(store.audio_prefs().deny.is_empty());
        assert!(!new.exists() && !chosen.exists());
        assert!(lines.is_empty(), "{lines:?}");
    }
}
