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

//! cw-trainer — a radio-neutral Morse trainer for real on-air contacts.
//!
//! `cw-trainer copy` is copy practice with **zero RF**: the PC plays
//! synthesized contacts — rag-chews, contest exchanges, POTA and SOTA
//! activators, DX pileups — through band noise, fading, static and QRM,
//! the operator types what they copy, and the copy is scored per
//! character. Everything about Morse comes from `cat-morse`
//! (radio-cat-rs ADR 0021); this crate is the session, the keyboard, the
//! speaker and the files. It works with any radio-cat-rs radio.
//!
//! # The radio is read once and then let go
//!
//! With `--server host:port` (a radio server's native/console port), the
//! radio's current CW pitch and keyer speed become the session's default
//! tone and speed. That is the only contact copy mode has with a radio,
//! and it is confined to one module that reads the values, drops the
//! connection, and hands back plain [`RadioDefaults`] before the session
//! starts ([`remote`]). Nothing else in the crate names a radio or protocol type, so no
//! write is reachable from copy mode (a test reads these sources to keep
//! it that way).
//!
//! # Known limitations
//!
//! - Two sessions at once: the last to save `stats.json` wins.
//!   `history.jsonl` is append-only and keeps both.

pub mod app;
pub mod audio;
pub mod cli;
pub mod migrate;
pub mod output;
pub mod remote;
pub mod session;
pub mod sink;
pub mod store;
pub mod term;
pub mod time;
pub mod view;

use std::str::FromStr;

use cat_morse::scenario::Difficulty;
use cat_morse::Callsign;

/// The program's name as it appears on the command line.
pub const PROGRAM: &str = "cw-trainer";

/// What `cw-trainer --version` prints: the program name and its version.
pub fn version_line() -> String {
    format!("{PROGRAM} {}", env!("CARGO_PKG_VERSION"))
}

pub use remote::RadioDefaults;

/// The tone used when neither `--pitch` nor the radio says otherwise.
pub const DEFAULT_PITCH_HZ: f32 = 600.0;
/// The speed used when neither `--wpm` nor the radio says otherwise.
pub const DEFAULT_WPM: u32 = 20;
/// Slowest and fastest speeds the trainer will use (cat-morse's range).
pub const MIN_WPM: u32 = 5;
pub const MAX_WPM: u32 = 60;
/// The `--pitch` range: comfortable listening, and well inside any
/// sound card's Nyquist once a station's offset is added.
pub const MIN_PITCH_HZ: f32 = 300.0;
pub const MAX_PITCH_HZ: f32 = 1200.0;

/// What kind of contacts to practise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    RagChew,
    Contest,
    Pota,
    Sota,
    Pileup,
    /// A kind drawn per QSO from the seed.
    Mixed,
}

impl Kind {
    pub const NAMES: &'static str = "ragchew|contest|pota|sota|pileup|mixed";

    pub fn name(self) -> &'static str {
        match self {
            Kind::RagChew => "ragchew",
            Kind::Contest => "contest",
            Kind::Pota => "pota",
            Kind::Sota => "sota",
            Kind::Pileup => "pileup",
            Kind::Mixed => "mixed",
        }
    }
}

impl FromStr for Kind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "ragchew" | "rag-chew" => Ok(Kind::RagChew),
            "contest" => Ok(Kind::Contest),
            "pota" => Ok(Kind::Pota),
            "sota" => Ok(Kind::Sota),
            "pileup" => Ok(Kind::Pileup),
            "mixed" => Ok(Kind::Mixed),
            _ => Err(format!("unknown kind {s:?}; expected {}", Kind::NAMES)),
        }
    }
}

/// How hard the other stations are to copy: cat-morse's presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Beginner,
    Intermediate,
    Advanced,
}

impl Level {
    pub const NAMES: &'static str = "beginner|intermediate|advanced";

    pub fn name(self) -> &'static str {
        match self {
            Level::Beginner => "beginner",
            Level::Intermediate => "intermediate",
            Level::Advanced => "advanced",
        }
    }

    /// The cat-morse preset: fists, strengths, offsets, repeat requests
    /// and band conditions.
    pub fn difficulty(self) -> Difficulty {
        match self {
            Level::Beginner => Difficulty::beginner(),
            Level::Intermediate => Difficulty::intermediate(),
            Level::Advanced => Difficulty::advanced(),
        }
    }

    /// How far, either side of the base speed, stations' speeds spread.
    pub fn wpm_spread(self) -> u32 {
        match self {
            Level::Beginner => 0,
            Level::Intermediate => 2,
            Level::Advanced => 4,
        }
    }
}

impl FromStr for Level {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "beginner" => Ok(Level::Beginner),
            "intermediate" => Ok(Level::Intermediate),
            "advanced" => Ok(Level::Advanced),
            _ => Err(format!("unknown level {s:?}; expected {}", Level::NAMES)),
        }
    }
}

/// `--farnsworth`: the level's own choice, none, or an effective speed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Farnsworth {
    Preset,
    Off,
    Wpm(u32),
}

impl FromStr for Farnsworth {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("off") {
            return Ok(Farnsworth::Off);
        }
        match s.parse::<u32>() {
            Ok(w) if (MIN_WPM..=MAX_WPM).contains(&w) => Ok(Farnsworth::Wpm(w)),
            _ => Err(format!(
                "--farnsworth wants an effective speed {MIN_WPM}-{MAX_WPM} or `off`, got {s:?}"
            )),
        }
    }
}

/// Everything a copy session needs, resolved: no radio in here.
#[derive(Debug, Clone, PartialEq)]
pub struct CopyOptions {
    pub kind: Kind,
    pub level: Level,
    pub qsos: u32,
    pub call: Callsign,
    pub wpm: u32,
    pub farnsworth: Farnsworth,
    pub pitch_hz: f32,
    pub seed: u64,
    pub adapt: bool,
    /// When the session started (ISO 8601 UTC), for the history file.
    pub started: String,
    /// The output device's name, shown in the banner and kept in history.
    pub audio_out: String,
}

/// The tone and speed a session starts at: an explicit flag wins, then
/// what the radio said, then the defaults.
pub fn resolve_tone_and_speed(
    wpm_flag: Option<u32>,
    pitch_flag: Option<f32>,
    radio: &RadioDefaults,
) -> (u32, f32) {
    let wpm = wpm_flag.or(radio.wpm).unwrap_or(DEFAULT_WPM);
    let pitch = pitch_flag.or(radio.pitch_hz).unwrap_or(DEFAULT_PITCH_HZ);
    (wpm, pitch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_line_names_program_and_package_version() {
        assert_eq!(
            version_line(),
            format!("cw-trainer {}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn kinds_and_levels_parse_by_name_and_round_trip() {
        for k in [
            Kind::RagChew,
            Kind::Contest,
            Kind::Pota,
            Kind::Sota,
            Kind::Pileup,
            Kind::Mixed,
        ] {
            assert_eq!(k.name().parse::<Kind>(), Ok(k));
        }
        for l in [Level::Beginner, Level::Intermediate, Level::Advanced] {
            assert_eq!(l.name().parse::<Level>(), Ok(l));
        }
        assert_eq!("POTA".parse::<Kind>(), Ok(Kind::Pota));
    }

    #[test]
    fn an_unknown_kind_or_level_says_what_it_wanted() {
        let e = "dx".parse::<Kind>().unwrap_err();
        assert!(e.contains(Kind::NAMES), "{e}");
        let e = "expert".parse::<Level>().unwrap_err();
        assert!(e.contains(Level::NAMES), "{e}");
    }

    #[test]
    fn farnsworth_takes_a_speed_or_off() {
        assert_eq!("off".parse::<Farnsworth>(), Ok(Farnsworth::Off));
        assert_eq!("12".parse::<Farnsworth>(), Ok(Farnsworth::Wpm(12)));
        assert!("2".parse::<Farnsworth>().is_err());
        assert!("fast".parse::<Farnsworth>().is_err());
    }

    #[test]
    fn a_flag_beats_the_radio_and_the_radio_beats_the_default() {
        let none = RadioDefaults::default();
        assert_eq!(
            resolve_tone_and_speed(None, None, &none),
            (DEFAULT_WPM, DEFAULT_PITCH_HZ)
        );
        let radio = RadioDefaults {
            pitch_hz: Some(800.0),
            wpm: Some(25),
            ..RadioDefaults::default()
        };
        assert_eq!(resolve_tone_and_speed(None, None, &radio), (25, 800.0));
        assert_eq!(
            resolve_tone_and_speed(Some(18), Some(650.0), &radio),
            (18, 650.0)
        );
    }
}
