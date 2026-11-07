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

//! cw-trainer-core — Morse copy-practice sessions, with no I/O.
//!
//! The session logic of the `cw-trainer` CLI, as a library an app
//! consumer can share: synthesized on-air contacts, grouped into overs,
//! scored per character, with adaptive speed and a serializable record.
//! It has no terminal, sound card, network, file or clock code, and builds
//! for desktop and mobile targets alike. Everything about Morse comes from
//! [`cat_morse`], which is re-exported so a consumer links the very same
//! crate.
//!
//! # A session, start to finish
//!
//! A [`CopySession`] decides; its caller shows, plays, collects the copy
//! and stores (see [`copy`]). Here a perfect operator copies one QSO:
//!
//! ```
//! use cw_trainer_core::cat_morse::Callsign;
//! use cw_trainer_core::options::{CopyOptions, Farnsworth, Kind, Level};
//! use cw_trainer_core::{CopySession, OverOutcome, Step};
//!
//! let options = CopyOptions {
//!     kind: Kind::RagChew,
//!     level: Level::Beginner,
//!     qsos: 1,
//!     call: Callsign::parse("N0CALL").unwrap(),
//!     wpm: 18,
//!     farnsworth: Farnsworth::Preset,
//!     pitch_hz: 600.0,
//!     seed: 1,
//!     adapt: Some(Default::default()),
//!     band: None,
//!     started: cw_trainer_core::time::iso8601_utc(1_700_000_000),
//!     audio_out: "Headphones".to_string(),
//! };
//! let mut session = CopySession::new(options, 48_000)?;
//! while let Some(step) = session.next_step()? {
//!     if let Step::ThemOver(over) = step {
//!         // A real caller plays `over.audio.renderer()?` to the output
//!         // the operator chose, and collects what was copied.
//!         let copy = over.expected.join(" ");
//!         let report = session.submit(OverOutcome::Copied {
//!             text: copy,
//!             agn: 0,
//!             audio_gaps: 0,
//!             clipped: 0,
//!         })?;
//!         assert_eq!(report.result.score.cer(), 0.0);
//!     }
//! }
//! let finished = session.finish(None);
//! assert!(!finished.record.quit_early);
//! # Ok::<(), cw_trainer_core::CoreError>(())
//! ```
//!
//! # Audio safety: every consumer's duty
//!
//! This library produces audio to render; it never chooses or opens an
//! output device. **A consumer must never play it to a device it did not
//! explicitly choose.** On a radio station the system's default output may
//! be wired to a transmitter's audio input, so a sound played there can go
//! out over the air. The `cw-trainer` CLI plays only to an output the
//! operator named (or remembered), checks it against a deny list before
//! and after opening it, and refuses otherwise; a consumer must enforce an
//! equivalent rule on its own platform.

#![forbid(unsafe_code)]

pub use cat_morse;

pub mod copy;
pub mod error;
pub mod options;
pub mod over;
pub mod plan;
pub mod record;
pub mod speed;
pub mod time;

pub use copy::{CopySession, Finished, OverAudio, OverOutcome, OverReport, Shown, Step, ThemOver};
pub use error::CoreError;
