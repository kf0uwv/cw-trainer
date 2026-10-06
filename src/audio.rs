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

//! Where an over's audio goes, and how one is built.
//!
//! [`AudioSink`] is the speaker as the session sees it: a non-blocking
//! queue at a fixed sample rate. The real one is
//! `cat_signal_audio::AudioPlayback`, wired in `main.rs`; tests use a fake
//! that drains instantly. There is no resampler anywhere, so an over is
//! synthesized at [`AudioSink::sample_rate_hz`] — the rate the device
//! actually accepted.

use std::time::Duration;

use cat_morse::scenario::{band_channel, Difficulty, Transmission};
use cat_morse::{timing, Channel, Script, Signal};

/// A speaker.
pub trait AudioSink {
    /// The rate to render at.
    fn sample_rate_hz(&self) -> u32;
    /// Queue mono samples. Never blocks: returns how many were taken.
    fn write(&mut self, mono: &[f32]) -> Result<usize, String>;
    /// Samples queued and not yet played.
    fn queued(&self) -> usize;
    /// Discard everything queued (AGN, skip, quit).
    fn clear(&mut self);
    /// Times the device ran short. See `cat_signal_audio::AudioPlayback`:
    /// this also ticks once at the natural end of the written audio, so
    /// only an increase while audio is still being fed is a heard gap.
    fn underruns(&self) -> u64 {
        0
    }
}

/// Silence before the first key-down, so the over does not start mid-word.
pub const LEAD: Duration = Duration::from_millis(300);
/// Band noise after the last key-up.
pub const TAIL: Duration = Duration::from_millis(500);

/// Build the band for one over: each line keyed with its station's speed
/// and fist at the operator's pitch plus that station's offset, starting
/// at `starts` (relative to the over), inside the level's band.
pub fn over_channel(
    script: &Script,
    lines: &[&Transmission],
    starts: &[Duration],
    difficulty: &Difficulty,
    sample_rate: u32,
    pitch_hz: f32,
    seed: u64,
) -> Result<Channel, String> {
    let mut signals = Vec::with_capacity(lines.len());
    for (line, start) in lines.iter().zip(starts) {
        let sig = script
            .signal_for(line)
            .ok_or_else(|| "a line with no station".to_string())?;
        let keying = timing::key_text(&line.text, sig.speed, &sig.fist, sig.fist_seed)
            .map_err(|e| format!("cannot key {:?}: {e}", line.text))?;
        signals.push(
            Signal::new(keying, pitch_hz + sig.pitch_offset_hz)
                .at_level(sig.strength_db)
                .starting_at(LEAD + *start),
        );
    }
    let (mut channel, _report) = band_channel(difficulty, signals, sample_rate, pitch_hz, seed)
        .map_err(|e| format!("cannot build the band: {e}"))?;
    channel.padding = TAIL;
    Ok(channel)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A speaker that plays instantly and remembers everything it played.
    ///
    /// Configurable to misbehave like a real one: take only `max_write`
    /// samples per write, fail once `fail_after` samples have been played,
    /// and report a scripted sequence of underrun counts.
    pub struct FakeSink {
        pub rate: u32,
        pub played: Vec<f32>,
        pub clears: u32,
        pub max_write: usize,
        pub fail_after: Option<usize>,
        pub underrun_seq: std::cell::RefCell<std::collections::VecDeque<u64>>,
        last_underruns: std::cell::Cell<u64>,
    }

    impl FakeSink {
        pub fn new(rate: u32) -> Self {
            FakeSink {
                rate,
                played: Vec::new(),
                clears: 0,
                max_write: usize::MAX,
                fail_after: None,
                underrun_seq: Default::default(),
                last_underruns: Default::default(),
            }
        }
    }

    impl AudioSink for FakeSink {
        fn sample_rate_hz(&self) -> u32 {
            self.rate
        }
        fn write(&mut self, mono: &[f32]) -> Result<usize, String> {
            if let Some(n) = self.fail_after {
                if self.played.len() >= n {
                    return Err("device unplugged".to_string());
                }
            }
            let take = mono.len().min(self.max_write);
            self.played.extend_from_slice(&mono[..take]);
            Ok(take)
        }
        fn queued(&self) -> usize {
            0
        }
        fn clear(&mut self) {
            self.clears += 1;
        }
        fn underruns(&self) -> u64 {
            if let Some(v) = self.underrun_seq.borrow_mut().pop_front() {
                self.last_underruns.set(v);
            }
            self.last_underruns.get()
        }
    }

    #[test]
    fn an_over_renders_every_line_inside_the_levels_band() {
        use cat_morse::scenario::Party;
        use cat_morse::{generate, Callsign, ScenarioConfig, ScenarioKind};

        let d = Difficulty::advanced();
        let script = generate(&ScenarioConfig {
            kind: ScenarioKind::RagChew,
            my_call: Callsign::parse("KF0UWV").unwrap(),
            difficulty: d.clone(),
            seed: 3,
        })
        .unwrap();
        let line = script
            .transmissions
            .iter()
            .find(|t| matches!(t.from, Party::Them(_)))
            .unwrap();
        let ch = over_channel(&script, &[line], &[Duration::ZERO], &d, 8000, 600.0, 9).unwrap();
        assert_eq!(ch.sample_rate, 8000);
        assert!(!ch.signals.is_empty());
        assert_eq!(ch.signals[0].start, LEAD);
        assert_eq!(ch.padding, TAIL);
        // Same seed, same band.
        let again = over_channel(&script, &[line], &[Duration::ZERO], &d, 8000, 600.0, 9).unwrap();
        assert_eq!(ch.render().unwrap(), again.render().unwrap());
    }
}
