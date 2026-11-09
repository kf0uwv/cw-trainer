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

//! One *them* over as audio: the band, its stations, lead and tail.

use std::time::Duration;

use cat_morse::scenario::{band_channel, Difficulty, SignalProfile, Transmission};
use cat_morse::{timing, Channel, Script, Signal};

use crate::error::CoreError;

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
) -> Result<Channel, CoreError> {
    let mut signals = Vec::with_capacity(lines.len());
    for (line, start) in lines.iter().zip(starts) {
        let sig = script.signal_for(line).ok_or(CoreError::NoStation)?;
        signals.push(keyed_signal(&line.text, sig, pitch_hz, *start)?);
    }
    band(difficulty, signals, sample_rate, pitch_hz, seed)
}

/// Build the band for one station sending `text`: what [`over_channel`]
/// builds for a single scripted line, for any text (a conversation's
/// replies are composed as it goes, so they have no script line).
pub fn text_channel(
    text: &str,
    signal: &SignalProfile,
    difficulty: &Difficulty,
    sample_rate: u32,
    pitch_hz: f32,
    seed: u64,
) -> Result<Channel, CoreError> {
    let keyed = keyed_signal(text, signal, pitch_hz, Duration::ZERO)?;
    band(difficulty, vec![keyed], sample_rate, pitch_hz, seed)
}

/// `text` keyed with the station's speed and fist, at the operator's pitch
/// plus its offset and at its strength, from `LEAD + start`.
fn keyed_signal(
    text: &str,
    sig: &SignalProfile,
    pitch_hz: f32,
    start: Duration,
) -> Result<Signal, CoreError> {
    let keying = timing::key_text(text, sig.speed, &sig.fist, sig.fist_seed).map_err(|source| {
        CoreError::Key {
            text: text.to_string(),
            source,
        }
    })?;
    Ok(Signal::new(keying, pitch_hz + sig.pitch_offset_hz)
        .at_level(sig.strength_db)
        .starting_at(LEAD + start))
}

fn band(
    difficulty: &Difficulty,
    signals: Vec<Signal>,
    sample_rate: u32,
    pitch_hz: f32,
    seed: u64,
) -> Result<Channel, CoreError> {
    let (mut channel, _report) =
        band_channel(difficulty, signals, sample_rate, pitch_hz, seed).map_err(CoreError::Band)?;
    channel.padding = TAIL;
    Ok(channel)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn any_text_renders_on_a_stations_signal_as_its_scripted_line_would() {
        use cat_morse::scenario::Party;
        use cat_morse::{generate, Callsign, ScenarioConfig, ScenarioKind};

        let d = Difficulty::intermediate();
        let script = generate(&ScenarioConfig {
            kind: ScenarioKind::RagChew,
            my_call: Callsign::parse("KF0UWV").unwrap(),
            difficulty: d.clone(),
            seed: 5,
        })
        .unwrap();
        let line = script
            .transmissions
            .iter()
            .find(|t| matches!(t.from, Party::Them(_)))
            .unwrap();
        let sig = script.signal_for(line).unwrap();
        let scripted =
            over_channel(&script, &[line], &[Duration::ZERO], &d, 8000, 600.0, 4).unwrap();
        let free = text_channel(&line.text, sig, &d, 8000, 600.0, 4).unwrap();
        assert_eq!(free.render().unwrap(), scripted.render().unwrap());
        assert_eq!(free.padding, TAIL);

        // Text that cannot be keyed is an error naming it.
        let e = text_channel("NAME ~", sig, &d, 8000, 600.0, 4).unwrap_err();
        assert!(
            matches!(e, CoreError::Key { ref text, .. } if text == "NAME ~"),
            "{e}"
        );
    }

    #[test]
    fn a_line_with_no_station_is_an_error_not_a_panic() {
        use cat_morse::scenario::{Party, Transmission};
        use cat_morse::{generate, Callsign, ScenarioConfig, ScenarioKind};

        let d = Difficulty::beginner();
        let script = generate(&ScenarioConfig {
            kind: ScenarioKind::RagChew,
            my_call: Callsign::parse("KF0UWV").unwrap(),
            difficulty: d.clone(),
            seed: 3,
        })
        .unwrap();
        let mine = script
            .transmissions
            .iter()
            .find(|t| matches!(t.from, Party::You))
            .unwrap();
        let lines: [&Transmission; 1] = [mine];
        let e = over_channel(&script, &lines, &[Duration::ZERO], &d, 8000, 600.0, 1).unwrap_err();
        assert_eq!(e, CoreError::NoStation);
    }
}
