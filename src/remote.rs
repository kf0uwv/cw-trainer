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

//! The only radio contact copy mode has: one read over the native
//! protocol, then the connection is gone.
//!
//! **Copy practice never transmits and never changes the radio's state.**
//! This file is where that is enforced:
//!
//! - [`CwDefaultsSource`] is the whole view copy mode gets of a radio's
//!   server, and it has one read method and nothing else.
//! - [`read_defaults`] connects, takes the source **by value**, reads once
//!   and drops it before returning, so the connection is closed before a
//!   session exists.
//! - Nothing else in the crate names a radio or protocol type (a
//!   structural test keeps it that way).
//!
//! Anything the server could not give, or gave out of range — including
//! not being reachable at all — becomes a warning and the local default,
//! and is never "corrected" on the radio. Copy practice always runs.

use thiserror::Error;

use crate::{MAX_PITCH_HZ, MAX_WPM, MIN_PITCH_HZ, MIN_WPM};

/// What the radio's server said about CW, if anything.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RadioDefaults {
    pub pitch_hz: Option<f32>,
    pub wpm: Option<u32>,
    /// Readings that could not be used, in words an operator can act on.
    pub warnings: Vec<String>,
    /// Facts worth one line that are not problems (e.g. "no CW here").
    pub notes: Vec<String>,
}

/// One CW reading, in the server's own terms.
///
/// Mirrors cat-native's wire types field for field so the protocol-backed
/// source maps straight in: `pitch_hz`/`keyer_wpm` are `RadioState.cw`'s
/// current values, and the ranges are `CapabilitiesWire.cw`'s advertised
/// `pitch` (Hz) and `keyer_wpm`. `None` anywhere: not reported.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CwReading {
    pub pitch_hz: Option<u32>,
    pub keyer_wpm: Option<u16>,
    /// Advertised pitch range, inclusive, in Hz.
    pub pitch_range_hz: Option<(u32, u32)>,
    /// Advertised keyer range, inclusive, in wpm.
    pub keyer_wpm_range: Option<(u16, u16)>,
}

/// Why the server could not be read.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum RemoteError {
    #[error("could not reach the radio server at {addr}: {reason}")]
    Connect { addr: String, reason: String },
    #[error("the radio server did not answer the CW read: {0}")]
    Read(String),
}

/// Read-only access to the radio's CW settings, as copy mode sees them.
///
/// Deliberately one read and nothing else: no method here can write.
pub trait CwDefaultsSource {
    /// The CW reading, or `Ok(None)` when the server/radio has no CW
    /// capability (an older server, or a radio without CW).
    fn read_cw(&mut self) -> Result<Option<CwReading>, RemoteError>;
}

/// Connect, read once, let go.
///
/// `connect` is called exactly once; the source it yields is consumed
/// here and dropped before this returns.
pub fn read_defaults<S, F>(connect: F) -> RadioDefaults
where
    S: CwDefaultsSource,
    F: FnOnce() -> Result<S, RemoteError>,
{
    let mut out = RadioDefaults::default();
    let answer = match connect() {
        Ok(mut source) => {
            let answer = source.read_cw();
            drop(source);
            answer
        }
        Err(e) => Err(e),
    };
    let reading = match answer {
        Ok(Some(r)) => r,
        Ok(None) => {
            out.notes.push(
                "the radio server reports no CW capability; using the local defaults".to_string(),
            );
            return out;
        }
        Err(e) => {
            out.warnings.push(format!("{e}; using the local defaults"));
            return out;
        }
    };

    // The advertised range, narrowed to what the trainer can play.
    let (lo, hi) = reading
        .pitch_range_hz
        .unwrap_or((MIN_PITCH_HZ as u32, MAX_PITCH_HZ as u32));
    let (lo, hi) = (lo.max(MIN_PITCH_HZ as u32), hi.min(MAX_PITCH_HZ as u32));
    match reading.pitch_hz {
        Some(hz) if (lo..=hi).contains(&hz) => out.pitch_hz = Some(hz as f32),
        Some(hz) => out.warnings.push(format!(
            "the radio reported a CW pitch of {hz} Hz, outside {lo}-{hi} Hz; using the default tone"
        )),
        None => out
            .warnings
            .push("the radio did not report its CW pitch; using the default tone".to_string()),
    }

    let (lo, hi) = reading
        .keyer_wpm_range
        .map(|(a, b)| (u32::from(a), u32::from(b)))
        .unwrap_or((MIN_WPM, MAX_WPM));
    let (lo, hi) = (lo.max(MIN_WPM), hi.min(MAX_WPM));
    match reading.keyer_wpm.map(u32::from) {
        Some(w) if (lo..=hi).contains(&w) => out.wpm = Some(w),
        Some(w) => out.warnings.push(format!(
            "the radio reported a keyer speed of {w} wpm, outside {lo}-{hi}; using the default speed"
        )),
        None => out
            .warnings
            .push("the radio did not report its keyer speed; using the default speed".to_string()),
    }
    out
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;

    /// Answers from a script; logs every call and when it is dropped.
    struct FakeSource {
        answer: Result<Option<CwReading>, RemoteError>,
        log: Rc<RefCell<Vec<&'static str>>>,
    }

    impl CwDefaultsSource for FakeSource {
        fn read_cw(&mut self) -> Result<Option<CwReading>, RemoteError> {
            self.log.borrow_mut().push("read_cw");
            self.answer.clone()
        }
    }

    impl Drop for FakeSource {
        fn drop(&mut self) {
            self.log.borrow_mut().push("drop");
        }
    }

    fn read(answer: Result<Option<CwReading>, RemoteError>) -> (RadioDefaults, Vec<&'static str>) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let connects = Cell::new(0);
        let source = FakeSource {
            answer,
            log: log.clone(),
        };
        let d = read_defaults(|| {
            connects.set(connects.get() + 1);
            Ok(source)
        });
        assert_eq!(connects.get(), 1, "connect exactly once");
        let calls = log.borrow().clone();
        (d, calls)
    }

    fn reading(pitch: u32, wpm: u16) -> CwReading {
        CwReading {
            pitch_hz: Some(pitch),
            keyer_wpm: Some(wpm),
            pitch_range_hz: Some((300, 1000)),
            keyer_wpm_range: Some((10, 60)),
        }
    }

    #[test]
    fn good_readings_become_the_defaults_after_one_read_and_a_drop() {
        let (d, calls) = read(Ok(Some(reading(800, 25))));
        assert_eq!(d.pitch_hz, Some(800.0));
        assert_eq!(d.wpm, Some(25));
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
        // One read, no retry, and the connection is gone on return.
        assert_eq!(calls, ["read_cw", "drop"]);
    }

    #[test]
    fn no_cw_capability_is_a_note_not_a_warning() {
        let (d, calls) = read(Ok(None));
        assert_eq!((d.pitch_hz, d.wpm), (None, None));
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
        assert_eq!(d.notes.len(), 1, "{:?}", d.notes);
        assert!(d.notes[0].contains("local defaults"), "{:?}", d.notes);
        assert_eq!(calls, ["read_cw", "drop"]);
    }

    #[test]
    fn a_failed_read_is_one_warning_and_no_retry() {
        let (d, calls) = read(Err(RemoteError::Read("timed out".into())));
        assert_eq!((d.pitch_hz, d.wpm), (None, None));
        assert_eq!(d.warnings.len(), 1, "{:?}", d.warnings);
        assert!(d.warnings[0].contains("timed out"), "{:?}", d.warnings);
        assert_eq!(calls, ["read_cw", "drop"]);
    }

    #[test]
    fn values_the_radio_could_not_report_are_warnings() {
        let r = CwReading {
            pitch_hz: None,
            keyer_wpm: None,
            ..reading(0, 0)
        };
        let (d, _) = read(Ok(Some(r)));
        assert_eq!((d.pitch_hz, d.wpm), (None, None));
        assert_eq!(d.warnings.len(), 2, "{:?}", d.warnings);
    }

    #[test]
    fn out_of_range_readings_are_reported_and_never_corrected() {
        // Outside what the server advertises.
        let (d, calls) = read(Ok(Some(reading(1100, 5))));
        assert_eq!((d.pitch_hz, d.wpm), (None, None));
        assert_eq!(d.warnings.len(), 2, "{:?}", d.warnings);
        assert_eq!(calls, ["read_cw", "drop"]);
        // Inside the advertised range but outside what the trainer plays.
        let wide = CwReading {
            pitch_range_hz: Some((100, 3000)),
            keyer_wpm_range: Some((1, 99)),
            ..reading(2000, 80)
        };
        let (d, _) = read(Ok(Some(wide)));
        assert_eq!((d.pitch_hz, d.wpm), (None, None));
        assert_eq!(d.warnings.len(), 2, "{:?}", d.warnings);
    }

    #[test]
    fn with_no_advertised_range_the_trainer_limits_apply() {
        let r = CwReading {
            pitch_hz: Some(700),
            keyer_wpm: Some(30),
            pitch_range_hz: None,
            keyer_wpm_range: None,
        };
        let (d, _) = read(Ok(Some(r)));
        assert_eq!((d.pitch_hz, d.wpm), (Some(700.0), Some(30)));
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    }

    #[test]
    fn an_unreachable_server_is_a_warning_and_the_local_defaults() {
        let d = read_defaults(|| -> Result<FakeSource, RemoteError> {
            Err(RemoteError::Connect {
                addr: "radio:4532".into(),
                reason: "connection refused".into(),
            })
        });
        assert_eq!((d.pitch_hz, d.wpm), (None, None));
        assert_eq!(d.warnings.len(), 1, "{:?}", d.warnings);
        assert!(d.warnings[0].contains("radio:4532"), "{:?}", d.warnings);
        assert!(d.warnings[0].contains("local defaults"), "{:?}", d.warnings);
    }
}
