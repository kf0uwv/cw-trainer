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
//! (radio-cat-rs ADR 0021), and the session itself — options, contacts,
//! scoring, adaptive speed, records — from the `cw-trainer-core` library
//! in this repository, which has no I/O of its own; this crate is the
//! keyboard, the speaker and the files. It works with any radio-cat-rs
//! radio.
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
pub mod view;

#[cfg(test)]
mod golden;

pub use cw_trainer_core::options::{
    CopyOptions, Farnsworth, Kind, Level, DEFAULT_PITCH_HZ, DEFAULT_WPM, MAX_PITCH_HZ, MAX_WPM,
    MIN_PITCH_HZ, MIN_WPM,
};
pub use cw_trainer_core::time;

/// The program's name as it appears on the command line.
pub const PROGRAM: &str = "cw-trainer";

/// What `cw-trainer --version` prints: the program name and its version.
pub fn version_line() -> String {
    format!("{PROGRAM} {}", env!("CARGO_PKG_VERSION"))
}

pub use remote::RadioDefaults;

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
