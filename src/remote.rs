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

use std::time::Duration;

use cat_native::{Connection, CwCapabilityWire, CwState, Streams};
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

/// How long connecting **and** the protocol handshake may take, together.
/// A server that accepts and never answers costs at most this.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// A radio's server, over the native protocol (ADR 0023's CW read).
///
/// Holds a connection that asked for no streams; the only things ever
/// asked of it are the handshake's capabilities and one state read.
pub struct ServerSource(Connection);

impl ServerSource {
    /// Connect and handshake, both bounded by `timeout`.
    pub fn connect(addr: &str, timeout: Duration) -> Result<Self, RemoteError> {
        Connection::connect_timeout(addr, Streams::none(), timeout)
            .map(Self)
            .map_err(|e| RemoteError::Connect {
                addr: addr.to_string(),
                reason: e.to_string(),
            })
    }
}

impl CwDefaultsSource for ServerSource {
    fn read_cw(&mut self) -> Result<Option<CwReading>, RemoteError> {
        // Never probe: a server whose Welcome carried no `cw` is not asked.
        let Some(cap) = self.0.cw_capability().cloned() else {
            return Ok(None);
        };
        let state = self
            .0
            .cw_state()
            .map_err(|e| RemoteError::Read(e.to_string()))?;
        Ok(Some(reading_from(&cap, state.as_ref())))
    }
}

/// What the server advertised and reported, in the trainer's terms.
pub fn reading_from(cap: &CwCapabilityWire, state: Option<&CwState>) -> CwReading {
    CwReading {
        pitch_hz: state.and_then(|s| s.pitch_hz),
        keyer_wpm: state.and_then(|s| s.keyer_wpm),
        pitch_range_hz: cap.pitch.as_ref().map(|r| (r.min_hz, r.max_hz)),
        keyer_wpm_range: cap.keyer_wpm.as_ref().map(|r| (r.min, r.max)),
    }
}

/// `--server host:port`: the radio's CW defaults, read once.
pub fn read_server_defaults(addr: &str) -> RadioDefaults {
    read_server_defaults_within(addr, CONNECT_TIMEOUT)
}

/// [`read_server_defaults`] with the connect/handshake bound given.
pub fn read_server_defaults_within(addr: &str, timeout: Duration) -> RadioDefaults {
    read_defaults(|| ServerSource::connect(addr, timeout))
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

    // ------------------------------------------------------------------
    // Over the native protocol
    // ------------------------------------------------------------------

    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Instant;

    use cat_native::{
        decode_frame, encode_frame, CapabilitiesWire, FrameKind, HzRange, ModeId, RadioState,
        WpmRange,
    };

    fn cw_cap() -> CwCapabilityWire {
        CwCapabilityWire {
            pitch: Some(HzRange {
                min_hz: 400,
                max_hz: 1000,
                step_hz: Some(50),
            }),
            pitch_writable: true,
            keyer_wpm: Some(WpmRange::new(10, 60)),
            keyer_wpm_writable: true,
            break_in: vec![],
            send_text: None,
            tx_gate: None,
        }
    }

    fn cw_state(pitch: Option<u32>, wpm: Option<u16>) -> CwState {
        CwState {
            pitch_hz: pitch,
            keyer_wpm: wpm,
            break_in: None,
            armed: None,
            sender: Default::default(),
            restore: None,
            id_owed: None,
        }
    }

    #[test]
    fn the_wire_maps_straight_into_a_reading() {
        let r = reading_from(&cw_cap(), Some(&cw_state(Some(750), Some(22))));
        assert_eq!(
            r,
            CwReading {
                pitch_hz: Some(750),
                keyer_wpm: Some(22),
                pitch_range_hz: Some((400, 1000)),
                keyer_wpm_range: Some((10, 60)),
            }
        );
        // CW advertised but not polled yet: ranges only.
        let r = reading_from(&cw_cap(), None);
        assert_eq!((r.pitch_hz, r.keyer_wpm), (None, None));
        assert_eq!(r.pitch_range_hz, Some((400, 1000)));
    }

    /// A one-connection server speaking the native protocol from a script:
    /// a Welcome advertising `cw`, then a State for any command. Returns
    /// its address and a handle yielding every control message it got, as
    /// JSON, and whether the client closed the connection afterwards.
    fn scripted_server(
        cw: Option<CwCapabilityWire>,
        state_cw: Option<CwState>,
    ) -> (
        String,
        std::thread::JoinHandle<(Vec<serde_json::Value>, bool)>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let handle = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut caps: CapabilitiesWire = cat_native::testing::stub_capabilities();
            caps.cw = cw;
            let state = RadioState {
                vfo_a_hz: 7_030_000,
                vfo_b_hz: 7_030_000,
                mode: ModeId::CwUpper,
                split: false,
                transmitting: false,
                memory_channel: None,
                if_shift_hz: None,
                filter_width_hz: None,
                meters: vec![],
                levels: None,
                cw: state_cw,
            };
            let mut got = Vec::new();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = match sock.read(&mut chunk) {
                    Ok(0) => return (got, true),
                    Ok(n) => n,
                    Err(_) => return (got, false),
                };
                buf.extend_from_slice(&chunk[..n]);
                while let Ok((FrameKind::Control, payload, used)) = decode_frame(&buf) {
                    let msg: serde_json::Value = serde_json::from_slice(payload).unwrap();
                    buf.drain(..used);
                    let reply = if msg["type"] == "hello" {
                        serde_json::json!({
                            "type": "welcome",
                            "version": cat_native::PROTOCOL_VERSION,
                            "capabilities": caps,
                        })
                    } else {
                        let mut v = serde_json::to_value(&state).unwrap();
                        v["type"] = "state".into();
                        v
                    };
                    got.push(msg);
                    let bytes = serde_json::to_vec(&reply).unwrap();
                    sock.write_all(&encode_frame(FrameKind::Control, &bytes))
                        .unwrap();
                }
            }
        });
        (addr, handle)
    }

    fn kinds(got: &[serde_json::Value]) -> Vec<String> {
        got.iter()
            .map(|m| match m["cmd"].as_str() {
                Some(c) => format!("{}:{c}", m["type"].as_str().unwrap_or("?")),
                None => m["type"].as_str().unwrap_or("?").to_string(),
            })
            .collect()
    }

    #[test]
    fn a_cw_server_is_read_once_and_let_go() {
        let (addr, server) = scripted_server(Some(cw_cap()), Some(cw_state(Some(750), Some(22))));
        let d = read_server_defaults(&addr);
        assert_eq!((d.pitch_hz, d.wpm), (Some(750.0), Some(22)));
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
        let (got, closed) = server.join().unwrap();
        // The handshake and one state read: nothing that could write.
        assert_eq!(kinds(&got), ["hello", "command:read_state"]);
        assert!(closed, "the connection is closed before the session");
    }

    #[test]
    fn a_cw_server_out_of_its_own_range_is_a_warning() {
        let (addr, server) = scripted_server(Some(cw_cap()), Some(cw_state(Some(1100), Some(22))));
        let d = read_server_defaults(&addr);
        assert_eq!((d.pitch_hz, d.wpm), (None, Some(22)));
        assert_eq!(d.warnings.len(), 1, "{:?}", d.warnings);
        server.join().unwrap();
    }

    #[test]
    fn cat_natives_stub_server_without_cw_is_a_note_and_never_written_to() {
        let host = cat_native::testing::StubHost::new();
        let addr = cat_native::testing::serve_stub(host.clone());
        let d = read_server_defaults(&addr);
        assert_eq!((d.pitch_hz, d.wpm), (None, None));
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
        assert_eq!(d.notes.len(), 1, "{:?}", d.notes);
        assert!(host.applied().is_empty(), "{:?}", host.applied());
    }

    #[test]
    fn an_unreachable_server_is_a_warning_naming_it() {
        let addr = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().to_string()
        }; // closed again: connecting is refused
        let d = read_server_defaults(&addr);
        assert_eq!((d.pitch_hz, d.wpm), (None, None));
        assert_eq!(d.warnings.len(), 1, "{:?}", d.warnings);
        assert!(d.warnings[0].contains(&addr), "{:?}", d.warnings);
    }

    #[test]
    fn a_server_that_never_answers_costs_the_timeout_not_forever() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let start = Instant::now();
        let d = read_server_defaults_within(&addr, Duration::from_millis(300));
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "{:?}",
            start.elapsed()
        );
        assert_eq!(d.warnings.len(), 1, "{:?}", d.warnings);
        assert!(d.warnings[0].contains(&addr), "{:?}", d.warnings);
        drop(l);
    }
}
