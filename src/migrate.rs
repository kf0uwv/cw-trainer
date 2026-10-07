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

//! One-time carry-over of `ts570d cw`'s files into cw-trainer's directory.
//!
//! The trainer used to live in ts570d and kept its files in
//! `<data>/ts570d/cw`. On the first run without `--data-dir`, when
//! cw-trainer's own directory does not exist yet and the old one does,
//! the old files are **copied** — never moved; the old directory is not
//! touched, so ts570d keeps working — into the new one:
//!
//! - `stats.json`, `audio.json`: copied only if they read as valid files
//!   of this version. An unreadable one is skipped with a warning, never
//!   copied as if it were good (the new directory then starts fresh for
//!   that file, and the old one is still there for a human).
//! - `history.jsonl`: copied if it can be read as text; a bad line in it
//!   is skipped by the reader as always.
//!
//! `audio.json` matters for safety, not only convenience: it carries the
//! deny list that keeps the radio's ACC2 interface from being chosen.
//!
//! The new directory is assembled under a temporary sibling name and
//! renamed into place whole, so a crash part-way cannot leave a
//! half-filled directory that a later run would take as "already done".

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use crate::store::{data_dir_from, parse_audio, parse_stats, AUDIO_FILE, HISTORY_FILE, STATS_FILE};

/// Where `ts570d cw` kept its files: `<data>/ts570d/cw`, with `<data>`
/// resolved exactly as for [`data_dir_from`].
pub fn legacy_data_dir_from(
    env: impl Fn(&str) -> Option<OsString>,
    windows: bool,
) -> Option<PathBuf> {
    // data_dir_from is `<data>/cw-trainer`; the legacy dir shares `<data>`.
    let new = data_dir_from(env, windows)?;
    Some(new.parent()?.join("ts570d").join("cw"))
}

/// [`legacy_data_dir_from`] for this process and platform.
pub fn default_legacy_data_dir() -> Option<PathBuf> {
    legacy_data_dir_from(|k| std::env::var_os(k), cfg!(windows))
}

/// What a migration did, in lines for the operator.
#[derive(Debug, Default, PartialEq)]
pub struct Migration {
    /// File names copied.
    pub copied: Vec<String>,
    /// Problems: files skipped, or the copy failing as a whole.
    pub warnings: Vec<String>,
}

impl Migration {
    /// Every line worth printing, informational first.
    pub fn lines(&self, old: &Path, new: &Path) -> Vec<String> {
        let mut out = Vec::new();
        if !self.copied.is_empty() {
            out.push(format!(
                "copied {} from {} to {} (the originals are untouched)",
                self.copied.join(", "),
                old.display(),
                new.display()
            ));
        }
        out.extend(self.warnings.iter().map(|w| format!("warning: {w}")));
        out
    }
}

/// Copy `old`'s files into `new`, if and only if `new` does not exist and
/// `old` does. Never writes to `old`.
pub fn migrate(old: &Path, new: &Path) -> Migration {
    let mut m = Migration::default();
    if new.exists() || !old.is_dir() {
        return m;
    }

    // Read and check everything first; nothing is written unless at
    // least one file is worth carrying over.
    let mut keep: Vec<(&str, Vec<u8>)> = Vec::new();
    for name in [STATS_FILE, AUDIO_FILE, HISTORY_FILE] {
        let path = old.join(name);
        if !path.exists() {
            continue;
        }
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                m.warnings.push(format!(
                    "{}: cannot be read ({e}); not copied",
                    path.display()
                ));
                continue;
            }
        };
        let check = match name {
            STATS_FILE => parse_stats(&bytes).map(drop),
            AUDIO_FILE => parse_audio(&bytes).map(drop),
            _ => std::str::from_utf8(&bytes)
                .map(drop)
                .map_err(|e| format!("not UTF-8: {e}")),
        };
        match check {
            Ok(()) => keep.push((name, bytes)),
            Err(e) => m.warnings.push(format!(
                "{}: unreadable ({e}); not copied, left where it is",
                path.display()
            )),
        }
    }
    if keep.is_empty() {
        return m;
    }

    // Assemble under a temporary sibling, then rename into place whole.
    let Some(parent) = new.parent() else {
        m.warnings.push(format!(
            "{} has no parent directory; nothing copied",
            new.display()
        ));
        return m;
    };
    let mut tmp_name = new.file_name().map(OsString::from).unwrap_or_default();
    tmp_name.push(format!(".migrating-{}", std::process::id()));
    let staging = parent.join(tmp_name);
    let result = (|| -> std::io::Result<()> {
        fs::create_dir_all(&staging)?;
        for (name, bytes) in &keep {
            fs::write(staging.join(name), bytes)?;
        }
        fs::rename(&staging, new)
    })();
    match result {
        Ok(()) => m.copied = keep.iter().map(|(n, _)| n.to_string()).collect(),
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            m.warnings.push(format!(
                "could not copy {} into {} ({e}); starting fresh there",
                old.display(),
                new.display()
            ));
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let m: HashMap<String, OsString> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |k| m.get(k).cloned()
    }

    const STATS: &str = "{\"version\":1,\"stats\":{\"units\":[]}}";
    const AUDIO: &str = "{\"version\":1,\"device\":\"audio:Headphones\",\"deny\":[\"USB PnP\"]}";
    const HISTORY: &str = "{\"not\":\"checked line by line\"}\n";

    /// An old dir holding `files`, and a new path that does not exist.
    fn setup(files: &[(&str, &[u8])]) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("ts570d").join("cw");
        fs::create_dir_all(&old).unwrap();
        for (name, body) in files {
            fs::write(old.join(name), body).unwrap();
        }
        let new = tmp.path().join("cw-trainer");
        (tmp, old, new)
    }

    fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut v: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let p = e.unwrap().path();
                (
                    p.file_name().unwrap().to_string_lossy().into_owned(),
                    fs::read(&p).unwrap(),
                )
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn the_legacy_dir_is_ts570d_cw_under_the_same_base() {
        let d = legacy_data_dir_from(env(&[("XDG_DATA_HOME", "/x"), ("HOME", "/h")]), false);
        assert_eq!(d, Some(PathBuf::from("/x/ts570d/cw")));
        let d = legacy_data_dir_from(env(&[("HOME", "/h")]), false);
        assert_eq!(d, Some(PathBuf::from("/h/.local/share/ts570d/cw")));
        let d = legacy_data_dir_from(env(&[("APPDATA", "/r")]), true);
        assert_eq!(d, Some(PathBuf::from("/r/ts570d/cw")));
        assert_eq!(legacy_data_dir_from(env(&[]), false), None);
    }

    #[test]
    fn everything_valid_is_copied_and_the_originals_are_untouched() {
        let (tmp, old, new) = setup(&[
            ("stats.json", STATS.as_bytes()),
            ("audio.json", AUDIO.as_bytes()),
            ("history.jsonl", HISTORY.as_bytes()),
        ]);
        let before = snapshot(&old);
        let m = migrate(&old, &new);
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
        assert_eq!(m.copied, ["stats.json", "audio.json", "history.jsonl"]);
        assert_eq!(snapshot(&new), before, "byte-for-byte copies");
        assert_eq!(snapshot(&old), before, "the old directory is not touched");
        // No temporary directory left beside the new one.
        let names: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
        let lines = m.lines(&old, &new);
        assert!(lines[0].contains("untouched"), "{lines:?}");
    }

    #[test]
    fn an_existing_new_directory_is_never_migrated_into() {
        let (_tmp, old, new) = setup(&[("stats.json", STATS.as_bytes())]);
        fs::create_dir_all(&new).unwrap();
        assert_eq!(migrate(&old, &new), Migration::default());
        assert!(snapshot(&new).is_empty());
    }

    #[test]
    fn no_old_directory_means_nothing_happens() {
        let tmp = tempfile::tempdir().unwrap();
        let new = tmp.path().join("cw-trainer");
        assert_eq!(
            migrate(&tmp.path().join("nope"), &new),
            Migration::default()
        );
        assert!(!new.exists());
    }

    #[test]
    fn a_partial_old_directory_copies_what_is_there() {
        let (_tmp, old, new) = setup(&[("audio.json", AUDIO.as_bytes())]);
        let m = migrate(&old, &new);
        assert_eq!(m.copied, ["audio.json"]);
        assert!(m.warnings.is_empty(), "{:?}", m.warnings);
        assert_eq!(fs::read_to_string(new.join("audio.json")).unwrap(), AUDIO);
    }

    #[test]
    fn unreadable_files_are_skipped_with_a_warning_not_copied() {
        let (_tmp, old, new) = setup(&[
            ("stats.json", b"{ corrupt"),
            ("audio.json", b"{\"version\":2,\"device\":null,\"deny\":[]}"),
            ("history.jsonl", b"\xff\xfe not text"),
        ]);
        let before = snapshot(&old);
        let m = migrate(&old, &new);
        assert!(m.copied.is_empty(), "{:?}", m.copied);
        assert_eq!(m.warnings.len(), 3, "{:?}", m.warnings);
        for (w, name) in m
            .warnings
            .iter()
            .zip(["stats.json", "audio.json", "history.jsonl"])
        {
            assert!(w.contains(name) && w.contains("not copied"), "{w}");
        }
        // Nothing worth copying: no new directory, and the old files stay.
        assert!(!new.exists());
        assert_eq!(snapshot(&old), before);
    }

    #[test]
    fn a_bad_file_does_not_stop_the_good_ones() {
        let (_tmp, old, new) = setup(&[
            ("stats.json", b"{ corrupt"),
            ("audio.json", AUDIO.as_bytes()),
        ]);
        let m = migrate(&old, &new);
        assert_eq!(m.copied, ["audio.json"]);
        assert_eq!(m.warnings.len(), 1, "{:?}", m.warnings);
        assert!(!new.join("stats.json").exists());
        assert!(new.join("audio.json").exists());
    }
}
