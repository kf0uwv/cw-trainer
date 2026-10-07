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

//! The trainer's memory: all-time per-character stats and a session log.
//!
//! | File | What | Written |
//! |------|------|---------|
//! | `stats.json` | `{ "version": 1, "stats": CharStats }`, all time | atomically, after every QSO |
//! | `history.jsonl` | one [`SessionRecord`] per line | appended at session end |
//! | `audio.json` | `{ "version": 1, "device": .., "deny": [..] }` — see [`super::output`] | when the choice or deny list changes |
//!
//! **Nothing here can stop a session.** A missing directory is created on
//! the first save. A `stats.json` that cannot be read — corrupt, or from a
//! newer version of this program — is renamed aside (never overwritten,
//! never deleted) and the session starts from fresh stats. A history line
//! that cannot be read is skipped. A directory that cannot be written is
//! a warning.

use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use cat_morse::CharStats;
use serde::{Deserialize, Serialize};

use super::output::AudioPrefs;
use super::session::SessionRecord;

/// The version this program writes and reads.
pub const STATS_VERSION: u32 = 1;

const STATS_FILE: &str = "stats.json";
const HISTORY_FILE: &str = "history.jsonl";
const AUDIO_FILE: &str = "audio.json";

#[derive(Serialize, Deserialize)]
struct AudioFile {
    version: u32,
    #[serde(flatten)]
    prefs: AudioPrefs,
}

#[derive(Serialize, Deserialize)]
struct StatsFile {
    version: u32,
    stats: CharStats,
}

/// Where the trainer keeps its files.
///
/// Linux and other Unix: `$XDG_DATA_HOME/cw-trainer`, else
/// `$HOME/.local/share/cw-trainer`. Windows: `%APPDATA%\cw-trainer`, else
/// `%USERPROFILE%\AppData\Roaming\cw-trainer`. `None` when the environment
/// names no home at all; the session then runs without saving.
pub fn data_dir_from(env: impl Fn(&str) -> Option<OsString>, windows: bool) -> Option<PathBuf> {
    let nonempty = |k: &str| env(k).filter(|v| !v.is_empty());
    let base = if windows {
        nonempty("APPDATA").map(PathBuf::from).or_else(|| {
            nonempty("USERPROFILE").map(|h| PathBuf::from(h).join("AppData").join("Roaming"))
        })?
    } else {
        nonempty("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| nonempty("HOME").map(|h| PathBuf::from(h).join(".local").join("share")))?
    };
    Some(base.join(crate::PROGRAM))
}

/// [`data_dir_from`] for this process and platform.
pub fn default_data_dir() -> Option<PathBuf> {
    data_dir_from(|k| std::env::var_os(k), cfg!(windows))
}

/// The loaded state of the data directory.
pub struct Store {
    dir: Option<PathBuf>,
    baseline: CharStats,
    /// `false` when an unreadable `stats.json` could not be moved aside:
    /// then it must not be overwritten either, so stats are not saved.
    stats_writable: bool,
    audio: AudioPrefs,
    audio_writable: bool,
    warnings: Vec<String>,
}

/// How a file on disk turned out.
enum Loaded<T> {
    Missing,
    Read(T),
    /// Unreadable and moved aside; start fresh and write normally.
    SetAside(String),
    /// Unreadable and could not be moved; start fresh, never write it.
    Stuck(String),
}

type Rename = fn(&Path, &Path) -> std::io::Result<()>;

/// Read and parse `path`. Any failure on a file that exists — unreadable
/// bytes, invalid UTF-8, bad JSON, another version — moves it aside so it
/// is kept for a human and never overwritten.
fn load<T>(path: &Path, parse: impl Fn(&[u8]) -> Result<T, String>, rename: Rename) -> Loaded<T> {
    let why = match fs::read(path) {
        // Nothing there (or nothing reachable to set aside): fresh.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound || !path.exists() => {
            return Loaded::Missing
        }
        Err(e) => e.to_string(),
        Ok(bytes) => match parse(&bytes) {
            Ok(v) => return Loaded::Read(v),
            Err(why) => why,
        },
    };
    match set_aside(path, rename) {
        Ok(aside) => Loaded::SetAside(format!(
            "{} could not be read ({why}); moved to {} and starting fresh",
            path.display(),
            aside.display()
        )),
        Err(e) => Loaded::Stuck(format!(
            "{} could not be read ({why}) and could not be moved aside ({e}); starting fresh \
             and NOT saving over it this session",
            path.display()
        )),
    }
}

impl Store {
    /// Load from `dir` (`None`: run without saving). Never fails.
    pub fn open(dir: Option<PathBuf>) -> Store {
        Store::open_with(dir, |a, b| fs::rename(a, b))
    }

    fn open_with(dir: Option<PathBuf>, rename: Rename) -> Store {
        let mut store = Store {
            dir,
            baseline: CharStats::new(),
            stats_writable: true,
            audio: AudioPrefs::default(),
            audio_writable: true,
            warnings: Vec::new(),
        };
        let Some(dir) = store.dir.clone() else {
            store
                .warnings
                .push("no home directory found; this session will not be saved".to_string());
            return store;
        };
        match load(&dir.join(STATS_FILE), parse_stats, rename) {
            Loaded::Missing => {}
            Loaded::Read(stats) => store.baseline = stats,
            Loaded::SetAside(w) => store.warnings.push(w),
            Loaded::Stuck(w) => {
                store.stats_writable = false;
                store.warnings.push(w);
            }
        }
        match load(&dir.join(AUDIO_FILE), parse_audio, rename) {
            Loaded::Missing => {}
            Loaded::Read(prefs) => store.audio = prefs,
            Loaded::SetAside(w) => store.warnings.push(w),
            Loaded::Stuck(w) => {
                store.audio_writable = false;
                store.warnings.push(w);
            }
        }
        store
    }

    /// The remembered audio output and deny list.
    pub fn audio_prefs(&self) -> &AudioPrefs {
        &self.audio
    }

    /// Remember `prefs` (written only if they changed).
    pub fn save_audio_prefs(&mut self, prefs: AudioPrefs) {
        if prefs == self.audio {
            return;
        }
        self.audio = prefs;
        let Some(dir) = self.dir.clone() else { return };
        if !self.audio_writable {
            return;
        }
        let file = AudioFile {
            version: STATS_VERSION,
            prefs: self.audio.clone(),
        };
        if let Err(e) = write_atomic(&dir, AUDIO_FILE, &file) {
            self.warnings.push(format!(
                "could not remember the audio choice in {}: {e}",
                dir.display()
            ));
        }
    }

    /// Warnings collected since the last call, for the operator.
    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    /// All-time stats: what was on disk plus `session`.
    pub fn all_time(&self, session: &CharStats) -> CharStats {
        let mut all = self.baseline.clone();
        all.merge(session);
        all
    }

    /// Write the all-time stats (baseline + `session`) atomically.
    pub fn save_stats(&mut self, session: &CharStats) {
        let Some(dir) = self.dir.clone() else { return };
        if !self.stats_writable {
            return;
        }
        let file = StatsFile {
            version: STATS_VERSION,
            stats: self.all_time(session),
        };
        if let Err(e) = write_atomic(&dir, STATS_FILE, &file) {
            self.warnings
                .push(format!("could not save stats in {}: {e}", dir.display()));
        }
    }

    /// Append one session to `history.jsonl`.
    pub fn append_history(&mut self, record: &SessionRecord) {
        let Some(dir) = self.dir.clone() else { return };
        let result = (|| -> std::io::Result<()> {
            fs::create_dir_all(&dir)?;
            let line = serde_json::to_string(record).map_err(std::io::Error::other)?;
            let mut f = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join(HISTORY_FILE))?;
            writeln!(f, "{line}")?;
            f.sync_all()
        })();
        if let Err(e) = result {
            self.warnings
                .push(format!("could not save history in {}: {e}", dir.display()));
        }
    }

    /// Every readable session in `history.jsonl`, oldest first.
    pub fn read_history(&self) -> Vec<SessionRecord> {
        let Some(dir) = &self.dir else {
            return Vec::new();
        };
        let Ok(bytes) = fs::read(dir.join(HISTORY_FILE)) else {
            return Vec::new();
        };
        String::from_utf8_lossy(&bytes)
            .lines()
            .filter_map(|l| serde_json::from_str::<SessionRecord>(l).ok())
            .collect()
    }
}

/// Write `value` as `dir/name` via a temporary file unique to this
/// process, synced to disk before it is renamed into place.
fn write_atomic<T: Serialize>(dir: &Path, name: &str, value: &T) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let tmp = dir.join(format!("{name}.tmp-{}-{}", std::process::id(), nanos()));
    let result = (|| {
        let json = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, dir.join(name))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn parse_stats(bytes: &[u8]) -> Result<CharStats, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("not UTF-8: {e}"))?;
    let file: StatsFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if file.version != STATS_VERSION {
        return Err(format!(
            "version {} is not {STATS_VERSION}; written by a different cw-trainer",
            file.version
        ));
    }
    Ok(file.stats)
}

fn parse_audio(bytes: &[u8]) -> Result<AudioPrefs, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("not UTF-8: {e}"))?;
    let file: AudioFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if file.version != STATS_VERSION {
        return Err(format!(
            "version {} is not {STATS_VERSION}; written by a different cw-trainer",
            file.version
        ));
    }
    Ok(file.prefs)
}

/// Rename an unreadable file out of the way, keeping it for a human. The
/// new name is unique even for two set-asides in the same second.
fn set_aside(path: &Path, rename: Rename) -> std::io::Result<PathBuf> {
    let base = path.file_name().map(OsString::from).unwrap_or_default();
    let stamp = nanos();
    let mut n = 0u32;
    let aside = loop {
        let mut name = base.clone();
        name.push(format!(".unreadable-{stamp}"));
        if n > 0 {
            name.push(format!("-{n}"));
        }
        let candidate = path.with_file_name(name);
        if !candidate.exists() {
            break candidate;
        }
        n += 1;
    };
    rename(path, &aside)?;
    Ok(aside)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use cat_morse::{align, ScoreOptions};

    use super::*;
    use crate::session::tests::sample_record;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let m: HashMap<String, OsString> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        move |k| m.get(k).cloned()
    }

    fn some_stats() -> CharStats {
        let mut s = CharStats::new();
        s.record(&align("CQ DE W1AW", "CQ DE W1AQ", &ScoreOptions::default()).unwrap());
        s
    }

    #[test]
    fn linux_prefers_xdg_data_home_then_home() {
        let d = data_dir_from(env(&[("XDG_DATA_HOME", "/x"), ("HOME", "/h")]), false);
        assert_eq!(d, Some(PathBuf::from("/x/cw-trainer")));
        let d = data_dir_from(env(&[("HOME", "/h")]), false);
        assert_eq!(d, Some(PathBuf::from("/h/.local/share/cw-trainer")));
        let d = data_dir_from(env(&[("XDG_DATA_HOME", ""), ("HOME", "/h")]), false);
        assert_eq!(d, Some(PathBuf::from("/h/.local/share/cw-trainer")));
        assert_eq!(data_dir_from(env(&[]), false), None);
    }

    #[test]
    fn windows_uses_appdata_then_the_profile() {
        let d = data_dir_from(env(&[("APPDATA", "/r"), ("HOME", "/h")]), true);
        assert_eq!(d, Some(PathBuf::from("/r/cw-trainer")));
        let d = data_dir_from(env(&[("USERPROFILE", "/u")]), true);
        assert_eq!(d, Some(PathBuf::from("/u/AppData/Roaming/cw-trainer")));
        assert_eq!(data_dir_from(env(&[("HOME", "/h")]), true), None);
    }

    #[test]
    fn a_missing_directory_is_fresh_and_is_created_on_save() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("nested").join("cw");
        let mut store = Store::open(Some(dir.clone()));
        assert!(store.take_warnings().is_empty());
        assert_eq!(store.all_time(&CharStats::new()), CharStats::new());
        store.save_stats(&some_stats());
        assert!(dir.join(STATS_FILE).exists());
        assert!(store.take_warnings().is_empty());
    }

    #[test]
    fn stats_round_trip_and_accumulate_across_sessions() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let mut first = Store::open(Some(dir.clone()));
        first.save_stats(&some_stats());
        // Saving twice in one session must not double-count.
        first.save_stats(&some_stats());

        let second = Store::open(Some(dir));
        assert_eq!(second.all_time(&CharStats::new()), some_stats());
        let mut twice = some_stats();
        twice.merge(&some_stats());
        assert_eq!(second.all_time(&some_stats()), twice);
    }

    #[test]
    fn history_appends_and_reads_back() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = Store::open(Some(tmp.path().to_path_buf()));
        let r = sample_record();
        store.append_history(&r);
        store.append_history(&r);
        assert_eq!(store.read_history(), vec![r.clone(), r]);
    }

    #[test]
    fn a_bad_history_line_is_skipped_not_fatal() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = Store::open(Some(tmp.path().to_path_buf()));
        let r = sample_record();
        store.append_history(&r);
        fs::OpenOptions::new()
            .append(true)
            .open(tmp.path().join(HISTORY_FILE))
            .unwrap()
            .write_all(b"{not json\n")
            .unwrap();
        store.append_history(&r);
        assert_eq!(store.read_history().len(), 2);
    }

    #[test]
    fn corrupt_stats_are_moved_aside_and_the_session_starts_fresh() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(STATS_FILE), "{\"version\":1,\"stats\":").unwrap();
        let mut store = Store::open(Some(tmp.path().to_path_buf()));
        let w = store.take_warnings();
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(store.all_time(&CharStats::new()), CharStats::new());
        let kept: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(
            kept.iter().any(|n| n.starts_with("stats.json.unreadable-")),
            "{kept:?}"
        );
        // And the next save works.
        store.save_stats(&some_stats());
        assert_eq!(
            Store::open(Some(tmp.path().to_path_buf())).all_time(&CharStats::new()),
            some_stats()
        );
    }

    #[test]
    fn the_audio_choice_and_deny_list_are_remembered() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = Store::open(Some(tmp.path().to_path_buf()));
        assert_eq!(store.audio_prefs(), &AudioPrefs::default());
        store.save_audio_prefs(AudioPrefs {
            device: Some("audio:Headphones".into()),
            deny: vec!["USB PnP".into()],
        });
        let again = Store::open(Some(tmp.path().to_path_buf()));
        assert_eq!(
            again.audio_prefs().device.as_deref(),
            Some("audio:Headphones")
        );
        assert_eq!(again.audio_prefs().deny, vec!["USB PnP".to_string()]);
    }

    #[test]
    fn an_unreadable_audio_file_forgets_the_choice_rather_than_guessing() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(AUDIO_FILE), "{\"device\":").unwrap();
        let mut store = Store::open(Some(tmp.path().to_path_buf()));
        assert_eq!(store.take_warnings().len(), 1);
        assert_eq!(store.audio_prefs(), &AudioPrefs::default());
    }

    #[test]
    fn non_utf8_stats_are_set_aside_not_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let junk = [0xffu8, 0xfe, b'{', 0x80];
        fs::write(tmp.path().join(STATS_FILE), junk).unwrap();
        let mut store = Store::open(Some(tmp.path().to_path_buf()));
        assert_eq!(store.take_warnings().len(), 1);
        store.save_stats(&some_stats());
        let aside = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.to_string_lossy().contains("unreadable"))
            .expect("set aside");
        assert_eq!(fs::read(aside).unwrap(), junk);
    }

    #[test]
    fn a_file_that_cannot_be_moved_aside_is_never_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(STATS_FILE), "garbage").unwrap();
        let mut store = Store::open_with(Some(tmp.path().to_path_buf()), |_, _| {
            Err(std::io::Error::other("read-only medium"))
        });
        let w = store.take_warnings();
        assert!(w[0].contains("NOT saving"), "{w:?}");
        store.save_stats(&some_stats());
        assert_eq!(
            fs::read_to_string(tmp.path().join(STATS_FILE)).unwrap(),
            "garbage"
        );
    }

    #[test]
    fn two_set_asides_never_collide() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(STATS_FILE);
        fs::write(&path, "one").unwrap();
        let a = set_aside(&path, |a, b| fs::rename(a, b)).unwrap();
        fs::write(&path, "two").unwrap();
        let b = set_aside(&path, |a, b| fs::rename(a, b)).unwrap();
        assert_ne!(a, b);
        assert_eq!(fs::read_to_string(a).unwrap(), "one");
        assert_eq!(fs::read_to_string(b).unwrap(), "two");
    }

    #[test]
    fn saving_leaves_no_temporary_files_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = Store::open(Some(tmp.path().to_path_buf()));
        store.save_stats(&some_stats());
        store.save_stats(&some_stats());
        let names: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, vec![STATS_FILE.to_string()]);
    }

    #[test]
    fn stats_from_a_newer_version_are_kept_not_clobbered() {
        let tmp = tempfile::tempdir().unwrap();
        let newer = "{\"version\":2,\"stats\":{\"units\":[]}}";
        fs::write(tmp.path().join(STATS_FILE), newer).unwrap();
        let mut store = Store::open(Some(tmp.path().to_path_buf()));
        let warnings = store.take_warnings();
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("different cw-trainer"), "{warnings:?}");
        let aside = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.to_string_lossy().contains("unreadable"))
            .expect("set aside");
        assert_eq!(fs::read_to_string(aside).unwrap(), newer);
    }

    #[test]
    fn an_unwritable_directory_is_a_warning() {
        let tmp = tempfile::tempdir().unwrap();
        // A file where the directory should be: create_dir_all fails on
        // every platform, without relying on permissions.
        let blocker = tmp.path().join("cw");
        fs::write(&blocker, "").unwrap();
        let mut store = Store::open(Some(blocker.join("inner")));
        store.take_warnings();
        store.save_stats(&some_stats());
        store.append_history(&sample_record());
        assert_eq!(store.take_warnings().len(), 2);
    }

    #[test]
    fn no_directory_runs_without_saving() {
        let mut store = Store::open(None);
        assert_eq!(store.take_warnings().len(), 1);
        store.save_stats(&some_stats());
        store.append_history(&sample_record());
        assert!(store.take_warnings().is_empty());
        assert!(store.read_history().is_empty());
    }
}
