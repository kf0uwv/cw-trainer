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

//! What a session leaves behind: one record per session, one result per
//! over.
//!
//! Its serialized form is a file format (one record per line of a
//! history file, version 1) and is frozen: fields are only ever appended,
//! with `#[serde(default)]`, so every old line still reads.

use cat_morse::Score;
use serde::{Deserialize, Serialize};

use crate::speed::SpeedSetting;

/// One over's result, as kept in the history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverResult {
    pub qso: u32,
    pub kind: String,
    /// The text sent, or each call for an over several stations sent.
    pub expected: Vec<String>,
    pub copied: String,
    pub score: Score,
    pub agn: u32,
    pub skipped: bool,
    /// Times the speaker ran dry mid-over.
    pub audio_gaps: u32,
    pub char_wpm: f64,
    pub effective_wpm: f64,
}

/// A whole session, as kept in the history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub version: u32,
    pub started: String,
    pub seed: u64,
    pub kind: String,
    pub level: String,
    pub call: String,
    pub pitch_hz: f32,
    /// The output device played to.
    #[serde(default)]
    pub audio_out: String,
    pub start_speed: SpeedSetting,
    pub end_speed: SpeedSetting,
    pub overs: Vec<OverResult>,
    /// Totals over every scored (not skipped) over.
    pub total: Score,
    pub quit_early: bool,
    /// Set when the session was cut short by an error (the error's text);
    /// everything scored before it is still here.
    #[serde(default)]
    pub ended_by_error: Option<String>,
}
