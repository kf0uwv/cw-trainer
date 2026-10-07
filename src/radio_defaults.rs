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

//! The only radio contact copy mode has: two reads.
//!
//! **Copy practice never transmits and never changes the radio's state.**
//! This file is where that is enforced:
//!
//! - [`CwReadings`] is the whole view copy mode gets of a radio, and it has
//!   two query methods and nothing else.
//! - [`read_defaults`] takes the radio **by value** and drops it before
//!   returning, so the connection is closed before a session exists.
//! - Nothing else under `src/cw/` names a radio type; a test in this file
//!   reads their sources to keep it that way.
//!
//! A reading the radio could not give, or gave out of range, becomes a
//! warning and the default — and is never "corrected" on the radio.

use async_trait::async_trait;
use cat_transport_core::{CatSession, TransportError};
use radio::{cw_pitch_hz, RadioResult, Ts570d, CW_PITCH_INDEX_MAX};

/// The keyer speeds a TS-570D reports (`KS010;`..`KS060;`).
const KS_RANGE: std::ops::RangeInclusive<u8> = 10..=60;

/// Read-only access to the two settings copy mode follows.
#[async_trait(?Send)]
pub trait CwReadings {
    /// `PT;` — the sidetone pitch index (0..=12).
    async fn cw_pitch_index(&mut self) -> RadioResult<u8>;
    /// `KS;` — the keyer speed in wpm.
    async fn keyer_speed(&mut self) -> RadioResult<u8>;
}

#[async_trait(?Send)]
impl<S> CwReadings for Ts570d<S>
where
    S: CatSession<Error = TransportError>,
{
    async fn cw_pitch_index(&mut self) -> RadioResult<u8> {
        self.get_cw_pitch().await
    }

    async fn keyer_speed(&mut self) -> RadioResult<u8> {
        self.get_keyer_speed().await
    }
}

/// What the radio said, if anything.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RadioDefaults {
    pub pitch_hz: Option<f32>,
    pub wpm: Option<u32>,
    /// Readings that could not be used, in words an operator can act on.
    pub warnings: Vec<String>,
}

/// Read `PT` and `KS` once each, then let the radio go.
///
/// Takes `radio` by value: when this returns, the caller no longer has a
/// radio, so a copy session cannot reach one.
pub async fn read_defaults<R: CwReadings>(mut radio: R) -> RadioDefaults {
    let mut out = RadioDefaults::default();

    match radio.cw_pitch_index().await {
        Ok(i) if i <= CW_PITCH_INDEX_MAX => out.pitch_hz = Some(cw_pitch_hz(i)),
        Ok(i) => out.warnings.push(format!(
            "the radio reported CW pitch index {i}, outside 0-{CW_PITCH_INDEX_MAX}; using the default tone"
        )),
        Err(e) => out
            .warnings
            .push(format!("could not read the CW pitch (PT): {e}; using the default tone")),
    }

    match radio.keyer_speed().await {
        Ok(w) if KS_RANGE.contains(&w) => out.wpm = Some(u32::from(w)),
        Ok(w) => out.warnings.push(format!(
            "the radio reported keyer speed {w} wpm, outside 10-60; using the default speed"
        )),
        Err(e) => out.warnings.push(format!(
            "could not read the keyer speed (KS): {e}; using the default speed"
        )),
    }

    drop(radio);
    out
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    use async_trait::async_trait;
    use cat_transport_core::{Transport, TransportError};
    use cat_transport_serial::SerialCatSession;
    use radio::Ts570d;

    use super::*;

    /// Records every byte written; answers from a script.
    struct RecordingTransport {
        written: Rc<RefCell<Vec<u8>>>,
        reads: VecDeque<u8>,
    }

    #[async_trait(?Send)]
    impl Transport for RecordingTransport {
        async fn write(&mut self, data: &[u8]) -> Result<usize, TransportError> {
            self.written.borrow_mut().extend_from_slice(data);
            Ok(data.len())
        }

        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
            match self.reads.pop_front() {
                Some(b) => {
                    buf[0] = b;
                    Ok(1)
                }
                None => Ok(0),
            }
        }

        async fn flush(&mut self) -> Result<(), TransportError> {
            Ok(())
        }
    }

    /// A real `Ts570d` over the recording double, and the shared log of
    /// what it wrote (which outlives the radio, since `read_defaults`
    /// consumes it).
    fn radio(
        answers: &str,
    ) -> (
        Ts570d<SerialCatSession<RecordingTransport>>,
        Rc<RefCell<Vec<u8>>>,
    ) {
        let written = Rc::new(RefCell::new(Vec::new()));
        let t = RecordingTransport {
            written: written.clone(),
            reads: answers.bytes().collect(),
        };
        (Ts570d::new(SerialCatSession::new(t)), written)
    }

    fn written(log: &Rc<RefCell<Vec<u8>>>) -> String {
        String::from_utf8(log.borrow().clone()).unwrap()
    }

    #[test]
    fn copy_mode_writes_exactly_two_read_only_queries() {
        // THE safety test: the bytes on the wire are `PT;` and `KS;` and
        // nothing else -- no set, no TX, no KY.
        let (r, log) = radio("PT08;KS025;");
        let d = futures::executor::block_on(read_defaults(r));
        assert_eq!(written(&log), "PT;KS;");
        assert_eq!(d.pitch_hz, Some(800.0));
        assert_eq!(d.wpm, Some(25));
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    }

    #[test]
    fn a_silent_radio_still_gets_only_the_two_queries_and_the_defaults_stand() {
        let (r, log) = radio("");
        let d = futures::executor::block_on(read_defaults(r));
        assert_eq!(written(&log), "PT;KS;");
        assert_eq!(d.pitch_hz, None);
        assert_eq!(d.wpm, None);
        assert_eq!(d.warnings.len(), 2, "{:?}", d.warnings);
    }

    #[test]
    fn garbage_answers_are_warnings_not_retries() {
        let (r, log) = radio("XX;??;");
        let d = futures::executor::block_on(read_defaults(r));
        assert_eq!(written(&log), "PT;KS;");
        assert_eq!((d.pitch_hz, d.wpm), (None, None));
        assert_eq!(d.warnings.len(), 2);
    }

    #[test]
    fn out_of_range_readings_are_reported_and_never_corrected_on_the_radio() {
        let (r, log) = radio("PT13;KS005;");
        let d = futures::executor::block_on(read_defaults(r));
        assert_eq!(written(&log), "PT;KS;");
        assert_eq!((d.pitch_hz, d.wpm), (None, None));
        assert_eq!(d.warnings.len(), 2, "{:?}", d.warnings);
    }

    #[test]
    fn the_session_side_of_cw_never_names_a_radio() {
        // Structural half of the invariant: the radio is reachable only
        // from this file. A later edit threading it into the session would
        // make a write reachable from copy mode; this catches that.
        // Every file in src/cw/ except this one, read at test time so a
        // file added later is scanned without anyone remembering to.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cw");
        let sources: Vec<(String, String)> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "rs"))
            .filter(|p| p.file_name().is_some_and(|n| n != "radio_defaults.rs"))
            .map(|p| {
                let name = p.file_name().unwrap().to_string_lossy().into_owned();
                (name, std::fs::read_to_string(&p).unwrap())
            })
            .collect();
        assert!(
            sources.len() >= 7,
            "expected the cw sources in {}, found {}",
            dir.display(),
            sources.len()
        );
        let forbidden = [
            "radio::",
            "Ts570d",
            "CatSession",
            "cat_transport",
            "send_cw",
            "transmit(",
            "set_keyer_speed",
            "set_cw_pitch",
        ];
        for (name, src) in &sources {
            for word in forbidden {
                assert!(
                    !src.contains(word),
                    "src/cw/{name} mentions `{word}`; only radio_defaults.rs may touch the radio"
                );
            }
        }
    }
}
