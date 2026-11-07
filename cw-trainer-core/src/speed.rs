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

//! Speed: the setting a QSO is sent at, and the adaptive rule.
//!
//! Over a window of the last [`WINDOW`] scored overs (skips excluded):
//! a CER of at most [`RAISE_CER`] with no AGN raises the speed by
//! [`STEP_WPM`]; a CER of at least [`LOWER_CER`] lowers it. With
//! Farnsworth spacing on, the *effective* speed moves first, until it
//! meets the character speed and Farnsworth falls away. A change takes
//! effect at the next QSO: a station does not change speed mid-contact.

use cat_morse::scenario::Difficulty;
use cat_morse::{Score, Speed};
use serde::{Deserialize, Serialize};

use crate::options::{Farnsworth, Level, MAX_WPM, MIN_WPM};

/// Scored overs the adaptive rule looks at.
pub const WINDOW: usize = 4;
/// At or below this CER, with no AGN in the window, speed goes up.
pub const RAISE_CER: f64 = 0.05;
/// At or above this CER, speed goes down.
pub const LOWER_CER: f64 = 0.15;
/// How much one adaptation moves the speed.
pub const STEP_WPM: u32 = 2;

/// A speed: character speed, and Farnsworth effective speed if spaced out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpeedSetting {
    pub char_wpm: u32,
    pub effective_wpm: Option<u32>,
}

impl SpeedSetting {
    pub fn describe(&self) -> String {
        match self.effective_wpm {
            Some(e) => format!("{} wpm (Farnsworth {e})", self.char_wpm),
            None => format!("{} wpm", self.char_wpm),
        }
    }

    /// The cat-morse speed for this setting.
    pub fn speed(&self) -> Speed {
        let c = f64::from(self.char_wpm);
        match self.effective_wpm {
            Some(e) => Speed::farnsworth(c, f64::from(e)),
            None => Speed::wpm(c),
        }
        .or_else(|_| Speed::wpm(c))
        .expect("char speed within 5..=60 is always valid")
    }
}

/// The current speed and the adaptive rule's memory.
#[derive(Debug, Clone)]
pub struct SpeedState {
    pub current: SpeedSetting,
    adapt: bool,
    window: Vec<(Score, u32)>,
}

impl SpeedState {
    pub fn new(char_wpm: u32, farnsworth: Farnsworth, level: Level, adapt: bool) -> Self {
        let char_wpm = char_wpm.clamp(MIN_WPM, MAX_WPM);
        let effective = match farnsworth {
            Farnsworth::Off => None,
            Farnsworth::Wpm(w) => Some(w),
            Farnsworth::Preset => level
                .difficulty()
                .farnsworth_effective
                .map(|e| e.round() as u32),
        }
        .filter(|e| *e < char_wpm);
        SpeedState {
            current: SpeedSetting {
                char_wpm,
                effective_wpm: effective,
            },
            adapt,
            window: Vec::new(),
        }
    }

    /// Record a scored over; `Some` when the speed changes.
    pub fn record(&mut self, score: &Score, agn: u32) -> Option<SpeedSetting> {
        if !self.adapt {
            return None;
        }
        self.window.push((*score, agn));
        if self.window.len() < WINDOW {
            return None;
        }
        let mut total = Score::default();
        for (s, _) in &self.window {
            total.add(s);
        }
        let any_agn = self.window.iter().any(|(_, a)| *a > 0);
        let cer = total.cer();
        let before = self.current;
        if cer <= RAISE_CER && !any_agn {
            self.raise();
        } else if cer >= LOWER_CER {
            self.lower();
        } else {
            self.window.remove(0);
            return None;
        }
        self.window.clear();
        (self.current != before).then_some(self.current)
    }

    fn raise(&mut self) {
        let c = &mut self.current;
        match c.effective_wpm {
            Some(e) if e + STEP_WPM >= c.char_wpm => c.effective_wpm = None,
            Some(e) => c.effective_wpm = Some(e + STEP_WPM),
            None => c.char_wpm = (c.char_wpm + STEP_WPM).min(MAX_WPM),
        }
    }

    fn lower(&mut self) {
        let c = &mut self.current;
        match c.effective_wpm {
            Some(e) => c.effective_wpm = Some(e.saturating_sub(STEP_WPM).max(MIN_WPM)),
            None => c.char_wpm = c.char_wpm.saturating_sub(STEP_WPM).max(MIN_WPM),
        }
    }
}

/// The level's preset, centred on the current speed.
pub fn difficulty_for(level: Level, speed: SpeedSetting) -> Difficulty {
    let mut d = level.difficulty();
    let s = level.wpm_spread();
    let lo = speed.char_wpm.saturating_sub(s).clamp(MIN_WPM, MAX_WPM);
    let hi = (speed.char_wpm + s).clamp(MIN_WPM, MAX_WPM);
    d.wpm = (lo, hi);
    d.farnsworth_effective = speed.effective_wpm.map(f64::from);
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(cer_errors: usize, expected: usize) -> Score {
        Score {
            expected,
            matches: expected - cer_errors,
            substitutions: cer_errors,
            deletions: 0,
            insertions: 0,
        }
    }

    // --- adaptive speed ---------------------------------------------------

    #[test]
    fn four_clean_overs_raise_the_speed() {
        let mut s = SpeedState::new(20, Farnsworth::Off, Level::Intermediate, true);
        for _ in 0..3 {
            assert_eq!(s.record(&score(0, 20), 0), None);
        }
        let c = s.record(&score(0, 20), 0).expect("raised");
        assert_eq!(c.char_wpm, 22);
    }

    #[test]
    fn a_poor_window_lowers_the_speed() {
        let mut s = SpeedState::new(20, Farnsworth::Off, Level::Intermediate, true);
        for _ in 0..3 {
            s.record(&score(5, 20), 0);
        }
        assert_eq!(s.record(&score(5, 20), 0).unwrap().char_wpm, 18);
    }

    #[test]
    fn an_agn_in_the_window_blocks_a_raise() {
        let mut s = SpeedState::new(20, Farnsworth::Off, Level::Intermediate, true);
        s.record(&score(0, 20), 1);
        for _ in 0..3 {
            assert_eq!(s.record(&score(0, 20), 0), None);
        }
        // The AGN has slid out of the window now.
        assert_eq!(s.record(&score(0, 20), 0).unwrap().char_wpm, 22);
    }

    #[test]
    fn farnsworth_spacing_closes_up_before_the_characters_speed_up() {
        let mut s = SpeedState::new(20, Farnsworth::Wpm(16), Level::Intermediate, true);
        let raise = |s: &mut SpeedState| {
            let mut last = None;
            for _ in 0..WINDOW {
                last = s.record(&score(0, 20), 0);
            }
            last.unwrap()
        };
        assert_eq!(raise(&mut s).effective_wpm, Some(18));
        assert_eq!(raise(&mut s).effective_wpm, None);
        assert_eq!(raise(&mut s).char_wpm, 22);
    }

    #[test]
    fn speed_stays_within_bounds_and_no_adapt_means_no_change() {
        let mut s = SpeedState::new(MAX_WPM, Farnsworth::Off, Level::Advanced, true);
        for _ in 0..WINDOW {
            assert_eq!(s.record(&score(0, 20), 0), None);
        }
        let mut s = SpeedState::new(MIN_WPM, Farnsworth::Off, Level::Beginner, true);
        for _ in 0..WINDOW {
            assert_eq!(s.record(&score(10, 20), 0), None);
        }
        let mut s = SpeedState::new(20, Farnsworth::Off, Level::Beginner, false);
        for _ in 0..10 {
            assert_eq!(s.record(&score(10, 20), 0), None);
        }
    }

    #[test]
    fn the_preset_farnsworth_applies_only_below_the_character_speed() {
        let s = SpeedState::new(20, Farnsworth::Preset, Level::Beginner, true);
        assert_eq!(s.current.effective_wpm, Some(10));
        let s = SpeedState::new(8, Farnsworth::Preset, Level::Beginner, true);
        assert_eq!(s.current.effective_wpm, None);
    }

    #[test]
    fn the_difficulty_is_centred_on_the_current_speed() {
        let d = difficulty_for(
            Level::Advanced,
            SpeedSetting {
                char_wpm: 7,
                effective_wpm: None,
            },
        );
        assert_eq!(d.wpm, (MIN_WPM, 11));
        assert_eq!(d.farnsworth_effective, None);
    }
}
