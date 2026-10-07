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

//! The trainer's speaker: a named sound card, never the system default
//! (see [`crate::output`]).

use crate::audio::AudioSink;

/// A sound card, as the trainer's speaker.
#[cfg(feature = "audio-device")]
pub struct PlaybackSink(cat_signal_audio::AudioPlayback);

#[cfg(feature = "audio-device")]
impl AudioSink for PlaybackSink {
    fn sample_rate_hz(&self) -> u32 {
        self.0.format().sample_rate_hz
    }
    fn write(&mut self, mono: &[f32]) -> Result<usize, String> {
        self.0.write(mono).map_err(|e| e.to_string())
    }
    fn queued(&self) -> usize {
        self.0.queued()
    }
    fn clear(&mut self) {
        self.0.clear();
    }
    fn underruns(&self) -> u64 {
        self.0.underruns()
    }
}

/// Open the named output. Returns the sink and the name the device opened
/// as, which the caller checks against the deny list.
#[cfg(feature = "audio-device")]
pub fn open_sink(spec: &str) -> Result<(PlaybackSink, String), String> {
    use cat_signal_audio::{AudioPlayback, PlaybackConfig};

    let playback = AudioPlayback::open(spec, PlaybackConfig::default())
        .map_err(|e| format!("could not open the audio output {spec:?}: {e}"))?;
    let f = playback.format();
    tracing::info!(
        "Audio output: {} at {} Hz, {} ch{}",
        playback.label(),
        f.sample_rate_hz,
        f.channels,
        if f.rate_substituted {
            " (the device's own rate)"
        } else {
            ""
        }
    );
    let label = playback.label().to_string();
    Ok((PlaybackSink(playback), label))
}

/// A build without sound-card support has no speaker to open, and
/// [`open_sink`] says how to get a build that has. This stands in for the
/// type so the rest of the trainer is the same code in both builds.
#[cfg(not(feature = "audio-device"))]
pub struct PlaybackSink;

#[cfg(not(feature = "audio-device"))]
impl AudioSink for PlaybackSink {
    fn sample_rate_hz(&self) -> u32 {
        48_000
    }
    fn write(&mut self, _: &[f32]) -> Result<usize, String> {
        Err("no audio output in this build".to_string())
    }
    fn queued(&self) -> usize {
        0
    }
    fn clear(&mut self) {}
}

#[cfg(not(feature = "audio-device"))]
pub fn open_sink(_: &str) -> Result<(PlaybackSink, String), String> {
    Err(
        "this build cannot play audio, and `cw-trainer copy` needs a sound card:\n\
         rebuild with `cargo build --release --features audio-device`"
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    #[cfg(not(feature = "audio-device"))]
    #[test]
    fn a_build_without_sound_cards_refuses_and_says_how_to_fix_it() {
        let e = super::open_sink("audio:Headphones").err().unwrap();
        assert!(e.contains("cw-trainer copy"), "{e}");
        assert!(e.contains("--features audio-device"), "{e}");
    }
}
