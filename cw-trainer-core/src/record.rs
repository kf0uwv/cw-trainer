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

use cat_morse::scenario::{BandConditions, QrmOdds, QrnOdds, QsbOdds};
use cat_morse::Score;
use serde::{Deserialize, Serialize};

use crate::converse::Checks;
use crate::speed::{AdaptRule, SpeedSetting};

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
    /// The adaptive rule, when it was not the default one (absent from
    /// the line otherwise, so a default session writes what it always did).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapt_rule: Option<AdaptRule>,
    /// The band conditions, when they were not the level's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub band: Option<RecordedBand>,
    /// A conversation's overs, both stations', in order (absent for a
    /// copy session, so a copy record writes what it always did).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turns: Vec<ConverseTurn>,
}

/// One over of a conversation, as kept in the history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConverseTurn {
    /// The operator's over (else the other station's).
    pub you: bool,
    /// What was sent: the station's text, or the operator's as decoded.
    pub text: String,
    /// How the operator's over read (absent for the station's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks: Option<Checks>,
    /// The station's over was a repeat asked for (`AGN`).
    #[serde(default, skip_serializing_if = "is_false")]
    pub repeat: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Band conditions as a record keeps them: cat-morse's
/// [`BandConditions`], field for field and written the same, but owned by
/// this format. Every field reads with a default, so a line stays readable
/// whatever cat-morse's own type later gains or drops.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordedBand {
    pub snr_db: Option<(f32, f32)>,
    pub qsb: Option<RecordedQsb>,
    pub qrn: Option<RecordedQrn>,
    pub qrm: Option<RecordedQrm>,
    pub rx_bandwidth_hz: Option<f32>,
}

/// cat-morse's `QsbOdds`, as a record keeps it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordedQsb {
    pub chance: f64,
    pub depth_db: (f32, f32),
    pub period_s: (f32, f32),
}

/// cat-morse's `QrnOdds`, as a record keeps it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordedQrn {
    pub chance: f64,
    pub crashes_per_sec: (f32, f32),
    pub level_db: (f32, f32),
    pub decay_ms: (u32, u32),
}

/// cat-morse's `QrmOdds`, as a record keeps it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordedQrm {
    pub chance: f64,
    pub offset_hz: (f32, f32),
    pub level_db: (f32, f32),
}

impl From<BandConditions> for RecordedBand {
    fn from(b: BandConditions) -> Self {
        RecordedBand {
            snr_db: b.snr_db,
            qsb: b.qsb.map(|q| RecordedQsb {
                chance: q.chance,
                depth_db: q.depth_db,
                period_s: q.period_s,
            }),
            qrn: b.qrn.map(|q| RecordedQrn {
                chance: q.chance,
                crashes_per_sec: q.crashes_per_sec,
                level_db: q.level_db,
                decay_ms: q.decay_ms,
            }),
            qrm: b.qrm.map(|q| RecordedQrm {
                chance: q.chance,
                offset_hz: q.offset_hz,
                level_db: q.level_db,
            }),
            rx_bandwidth_hz: b.rx_bandwidth_hz,
        }
    }
}

impl From<RecordedBand> for BandConditions {
    fn from(b: RecordedBand) -> Self {
        BandConditions {
            snr_db: b.snr_db,
            qsb: b.qsb.map(|q| QsbOdds {
                chance: q.chance,
                depth_db: q.depth_db,
                period_s: q.period_s,
            }),
            qrn: b.qrn.map(|q| QrnOdds {
                chance: q.chance,
                crashes_per_sec: q.crashes_per_sec,
                level_db: q.level_db,
                decay_ms: q.decay_ms,
            }),
            qrm: b.qrm.map(|q| QrmOdds {
                chance: q.chance,
                offset_hz: q.offset_hz,
                level_db: q.level_db,
            }),
            rx_bandwidth_hz: b.rx_bandwidth_hz,
        }
    }
}
