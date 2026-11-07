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

//! The record's serialized form is frozen: these fixtures were written by
//! the `cw-trainer` CLI before the library existed, and a record must
//! still read and write exactly as they are.

use std::fs;
use std::path::{Path, PathBuf};

use cw_trainer_core::record::SessionRecord;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/records")
}

fn named(suffix: &str) -> Vec<(String, String)> {
    let mut out: Vec<_> = fs::read_dir(fixtures())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(suffix))
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(&p).unwrap())
        })
        .collect();
    out.sort();
    assert!(out.len() >= 10, "fixtures *{suffix} not found: {out:?}");
    out
}

#[test]
fn a_record_round_trips_to_the_same_pretty_bytes() {
    for (name, text) in named(".record.json") {
        let r: SessionRecord = serde_json::from_str(&text).unwrap();
        assert_eq!(
            serde_json::to_string_pretty(&r).unwrap() + "\n",
            text,
            "{name}"
        );
    }
}

#[test]
fn a_history_line_round_trips_to_the_same_bytes() {
    for (name, text) in named(".history.jsonl") {
        for line in text.lines() {
            let r: SessionRecord = serde_json::from_str(line).unwrap();
            assert_eq!(serde_json::to_string(&r).unwrap(), line, "{name}");
        }
    }
}

#[test]
fn the_pretty_record_and_the_history_line_are_the_same_record() {
    for (name, text) in named(".record.json") {
        let stem = name.trim_end_matches(".record.json");
        let history = fs::read_to_string(fixtures().join(format!("{stem}.history.jsonl"))).unwrap();
        let last = history.lines().last().unwrap();
        let a: SessionRecord = serde_json::from_str(&text).unwrap();
        let b: SessionRecord = serde_json::from_str(last).unwrap();
        assert_eq!(a, b, "{name}");
    }
}

#[test]
fn old_history_lines_still_parse() {
    // Without `audio_out` and `ended_by_error`, with `audio_out` only, and
    // with both.
    let text = fs::read_to_string(fixtures().join("old-history.jsonl")).unwrap();
    let records: Vec<SessionRecord> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].audio_out, "");
    assert_eq!(records[0].ended_by_error, None);
    assert_eq!(records[1].audio_out, "Test speaker");
    assert_eq!(
        records[2].ended_by_error.as_deref(),
        Some("audio: device unplugged")
    );
    assert_eq!(records[0].overs, records[2].overs);
}
