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

//! Why a session cannot go on.

use cat_morse::{CodecError, ScenarioError, SynthError, TimingError};

use crate::copy::MIN_SAMPLE_RATE_HZ;

/// Everything the library can fail with.
///
/// The Display texts are part of the history format: a session cut short
/// keeps its error's text, so they never change.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CoreError {
    /// cat-morse could not generate the QSO's script.
    #[error("cannot generate a contact: {0}")]
    Generate(ScenarioError),
    /// An over's audio could not be rendered.
    #[error("cannot render: {0}")]
    Render(SynthError),
    /// A line of the script could not be keyed.
    #[error("cannot key {text:?}: {source}")]
    Key { text: String, source: TimingError },
    /// The band an over is copied through could not be built.
    #[error("cannot build the band: {0}")]
    Band(ScenarioError),
    /// A *them* line the script has no station for.
    #[error("a line with no station")]
    NoStation,
    /// What was sent does not encode, so the copy cannot be scored.
    #[error("{0}")]
    Align(CodecError),
    /// A sample rate too low to carry the session's pitch.
    #[error("sample rate {0} Hz is below the minimum of {min} Hz", min = MIN_SAMPLE_RATE_HZ)]
    SampleRate(u32),
    /// Options a session cannot run with.
    #[error("invalid options: {0}")]
    InvalidOptions(String),
    /// [`next_step`](crate::copy::CopySession::next_step) while a *them*
    /// over still waits for its outcome.
    #[error("a them over is still waiting for its outcome")]
    OverPending,
    /// [`submit`](crate::copy::CopySession::submit) with no *them* over
    /// waiting.
    #[error("no them over is waiting for an outcome")]
    NoOverPending,
    /// The session already failed; only `finish` is left.
    #[error("the session has already failed")]
    Failed,
}

#[cfg(test)]
mod tests {
    use cat_morse::{CodecError, ScenarioError, SynthError, TimingError};

    use super::*;

    // These strings are frozen: a session cut short stores its error's
    // text in the history file, and old and new texts must read alike.

    #[test]
    fn a_contact_that_cannot_be_generated() {
        let e = CoreError::Generate(ScenarioError::InvalidSetting("x out of range"));
        assert_eq!(
            e.to_string(),
            "cannot generate a contact: invalid scenario setting: x out of range"
        );
    }

    #[test]
    fn an_over_that_cannot_be_rendered() {
        let e = CoreError::Render(SynthError::ZeroSampleRate);
        assert_eq!(e.to_string(), "cannot render: sample rate must be non-zero");
    }

    #[test]
    fn a_line_that_cannot_be_keyed() {
        let e = CoreError::Key {
            text: "CQ DE W1AW".to_string(),
            source: TimingError::InvalidWpm(2.0),
        };
        assert_eq!(
            e.to_string(),
            "cannot key \"CQ DE W1AW\": words per minute must be in [3, 100], got 2"
        );
    }

    #[test]
    fn a_band_that_cannot_be_built() {
        let e = CoreError::Band(ScenarioError::InvalidSetting("pitch"));
        assert_eq!(
            e.to_string(),
            "cannot build the band: invalid scenario setting: pitch"
        );
    }

    #[test]
    fn a_copy_that_cannot_be_aligned_is_the_bare_codec_error() {
        let codec = CodecError::UnterminatedProsign { offset: 3 };
        let e = CoreError::Align(codec.clone());
        assert_eq!(e.to_string(), codec.to_string());
        assert_eq!(e.to_string(), "unterminated prosign starting at byte 3");
    }

    #[test]
    fn a_line_with_no_station() {
        assert_eq!(CoreError::NoStation.to_string(), "a line with no station");
    }
}
