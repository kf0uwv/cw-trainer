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

use super::session::SessionRecord;

/// The version this program writes and reads.
pub const STATS_VERSION: u32 = 1;

const STATS_FILE: &str = "stats.json";
const HISTORY_FILE: &str = "history.jsonl";

#[derive(Serialize, Deserialize)]
struct StatsFile {
    version: u32,
    stats: CharStats,
}

/// Where the trainer keeps its files.
///
/// Linux and other Unix: `$XDG_DATA_HOME/ts570d/cw`, else
/// `$HOME/.local/share/ts570d/cw`. Windows: `%APPDATA%\ts570d\cw`, else
/// `%USERPROFILE%\AppData\Roaming\ts570d\cw`. `None` when the environment
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
    Some(base.join("ts570d").join("cw"))
}

/// [`data_dir_from`] for this process and platform.
pub fn default_data_dir() -> Option<PathBuf> {
    data_dir_from(|k| std::env::var_os(k), cfg!(windows))
}

/// The loaded state of the data directory.
pub struct Store {
    dir: Option<PathBuf>,
    baseline: CharStats,
    warnings: Vec<String>,
}

impl Store {
    /// Load from `dir` (`None`: run without saving). Never fails.
    pub fn open(dir: Option<PathBuf>) -> Store {
        let mut store = Store {
            dir,
            baseline: CharStats::new(),
            warnings: Vec::new(),
        };
        match &store.dir {
            None => store
                .warnings
                .push("no home directory found; this session will not be saved".to_string()),
            Some(dir) => {
                let path = dir.join(STATS_FILE);
                match fs::read_to_string(&path) {
                    Ok(text) => match parse_stats(&text) {
                        Ok(stats) => store.baseline = stats,
                        Err(why) => {
                            let aside = set_aside(&path);
                            store.warnings.push(format!(
                                "{} could not be read ({why}); moved to {} and starting fresh stats",
                                path.display(),
                                aside.display()
                            ));
                        }
                    },
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => store.warnings.push(format!(
                        "{} could not be read ({e}); starting fresh stats",
                        path.display()
                    )),
                }
            }
        }
        store
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
        let file = StatsFile {
            version: STATS_VERSION,
            stats: self.all_time(session),
        };
        let result = (|| -> std::io::Result<()> {
            fs::create_dir_all(&dir)?;
            let tmp = dir.join(format!("{STATS_FILE}.tmp"));
            let json = serde_json::to_string_pretty(&file).map_err(std::io::Error::other)?;
            fs::write(&tmp, json)?;
            fs::rename(&tmp, dir.join(STATS_FILE))
        })();
        if let Err(e) = result {
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
            writeln!(f, "{line}")
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
        let Ok(text) = fs::read_to_string(dir.join(HISTORY_FILE)) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|l| serde_json::from_str::<SessionRecord>(l).ok())
            .collect()
    }
}

fn parse_stats(text: &str) -> Result<CharStats, String> {
    let file: StatsFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if file.version != STATS_VERSION {
        return Err(format!(
            "version {} is not {STATS_VERSION}; written by a different ts570d",
            file.version
        ));
    }
    Ok(file.stats)
}

/// Rename an unreadable file out of the way, keeping it for a human.
fn set_aside(path: &Path) -> PathBuf {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut name = path.file_name().map(OsString::from).unwrap_or_default();
    name.push(format!(".unreadable-{secs}"));
    let aside = path.with_file_name(name);
    let _ = fs::rename(path, &aside);
    aside
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use cat_morse::{align, ScoreOptions};

    use super::*;
    use crate::cw::session::tests::sample_record;

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
        assert_eq!(d, Some(PathBuf::from("/x/ts570d/cw")));
        let d = data_dir_from(env(&[("HOME", "/h")]), false);
        assert_eq!(d, Some(PathBuf::from("/h/.local/share/ts570d/cw")));
        let d = data_dir_from(env(&[("XDG_DATA_HOME", ""), ("HOME", "/h")]), false);
        assert_eq!(d, Some(PathBuf::from("/h/.local/share/ts570d/cw")));
        assert_eq!(data_dir_from(env(&[]), false), None);
    }

    #[test]
    fn windows_uses_appdata_then_the_profile() {
        let d = data_dir_from(env(&[("APPDATA", "/r"), ("HOME", "/h")]), true);
        assert_eq!(d, Some(PathBuf::from("/r/ts570d/cw")));
        let d = data_dir_from(env(&[("USERPROFILE", "/u")]), true);
        assert_eq!(d, Some(PathBuf::from("/u/AppData/Roaming/ts570d/cw")));
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
    fn stats_from_a_newer_version_are_kept_not_clobbered() {
        let tmp = tempfile::tempdir().unwrap();
        let newer = "{\"version\":2,\"stats\":{\"units\":[]}}";
        fs::write(tmp.path().join(STATS_FILE), newer).unwrap();
        let mut store = Store::open(Some(tmp.path().to_path_buf()));
        assert_eq!(store.take_warnings().len(), 1);
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
