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

pub use cw_trainer_core::over::{over_channel, LEAD, TAIL};

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
        /// `played.len()` at each clear: where each play of an over ends.
        pub clear_marks: Vec<usize>,
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
                clear_marks: Vec::new(),
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
            self.clear_marks.push(self.played.len());
        }
        fn underruns(&self) -> u64 {
            if let Some(v) = self.underrun_seq.borrow_mut().pop_front() {
                self.last_underruns.set(v);
            }
            self.last_underruns.get()
        }
    }
}
