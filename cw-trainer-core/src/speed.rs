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
//! The default [`AdaptRule`] (the CLI's): over a window of the last
//! [`WINDOW`] scored overs (skips excluded):
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

/// The adaptive-speed rule. Its [`Default`] is the CLI's rule: the
/// constants above.
///
/// `min_wpm`/`max_wpm` bound the *character* speed: a session starts at
/// its requested speed clamped into them, and adaptation never leaves
/// them. A Farnsworth effective speed is only ever lowered to
/// [`MIN_WPM`], and closes up to the character speed before that rises.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AdaptRule {
    /// Scored overs looked at (skips excluded); at least 1.
    pub window: u32,
    /// At or below this CER, with no AGN in the window, speed goes up.
    /// At least 0.
    pub raise_cer: f64,
    /// At or above this CER, speed goes down. Greater than `raise_cer`,
    /// at most 1.
    pub lower_cer: f64,
    /// How much one adaptation moves the speed: 1 to `MAX_WPM - MIN_WPM`.
    pub step_wpm: u32,
    /// The slowest character speed, at least [`MIN_WPM`].
    pub min_wpm: u32,
    /// The fastest character speed, at most [`MAX_WPM`].
    pub max_wpm: u32,
}

impl Default for AdaptRule {
    fn default() -> Self {
        AdaptRule {
            window: WINDOW as u32,
            raise_cer: RAISE_CER,
            lower_cer: LOWER_CER,
            step_wpm: STEP_WPM,
            min_wpm: MIN_WPM,
            max_wpm: MAX_WPM,
        }
    }
}

impl AdaptRule {
    /// Why this rule cannot be used, if it cannot.
    pub fn problem(&self) -> Option<String> {
        let r = self;
        if r.window == 0 {
            return Some("the adaptive window must be at least 1 over".to_string());
        }
        if r.step_wpm == 0 || r.step_wpm > MAX_WPM - MIN_WPM {
            return Some(format!(
                "the adaptive step must be 1-{} wpm, got {}",
                MAX_WPM - MIN_WPM,
                r.step_wpm
            ));
        }
        if !(r.raise_cer.is_finite() && r.lower_cer.is_finite()) {
            return Some(format!(
                "the adaptive CERs must be finite, got {} and {}",
                r.raise_cer, r.lower_cer
            ));
        }
        if r.raise_cer < 0.0 || r.lower_cer > 1.0 {
            return Some(format!(
                "the adaptive CERs must lie within 0-1, got {} and {}",
                r.raise_cer, r.lower_cer
            ));
        }
        if r.raise_cer >= r.lower_cer {
            return Some(format!(
                "the raise CER {} must be below the lower CER {}",
                r.raise_cer, r.lower_cer
            ));
        }
        if !(MIN_WPM..=MAX_WPM).contains(&r.min_wpm)
            || !(MIN_WPM..=MAX_WPM).contains(&r.max_wpm)
            || r.min_wpm > r.max_wpm
        {
            return Some(format!(
                "the adaptive speeds {}-{} wpm must lie within {MIN_WPM}-{MAX_WPM} wpm, \
                 slowest first",
                r.min_wpm, r.max_wpm
            ));
        }
        None
    }
}

/// The current speed and the adaptive rule's memory.
#[derive(Debug, Clone)]
pub struct SpeedState {
    pub current: SpeedSetting,
    rule: Option<AdaptRule>,
    window: Vec<(Score, u32)>,
}

impl SpeedState {
    /// The speed a session starts at; `rule` `None` keeps it fixed.
    pub fn new(
        char_wpm: u32,
        farnsworth: Farnsworth,
        level: Level,
        rule: Option<AdaptRule>,
    ) -> Self {
        let (lo, hi) = rule.map_or((MIN_WPM, MAX_WPM), |r| (r.min_wpm, r.max_wpm));
        let char_wpm = char_wpm.clamp(lo, hi);
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
            rule,
            window: Vec::new(),
        }
    }

    /// Record a scored over; `Some` when the speed changes.
    pub fn record(&mut self, score: &Score, agn: u32) -> Option<SpeedSetting> {
        let rule = self.rule?;
        self.window.push((*score, agn));
        if self.window.len() < rule.window as usize {
            return None;
        }
        let mut total = Score::default();
        for (s, _) in &self.window {
            total.add(s);
        }
        let any_agn = self.window.iter().any(|(_, a)| *a > 0);
        let cer = total.cer();
        let before = self.current;
        if cer <= rule.raise_cer && !any_agn {
            self.raise(&rule);
        } else if cer >= rule.lower_cer {
            self.lower(&rule);
        } else {
            self.window.remove(0);
            return None;
        }
        self.window.clear();
        (self.current != before).then_some(self.current)
    }

    fn raise(&mut self, rule: &AdaptRule) {
        let c = &mut self.current;
        let step = rule.step_wpm;
        match c.effective_wpm {
            Some(e) if e.saturating_add(step) >= c.char_wpm => c.effective_wpm = None,
            Some(e) => c.effective_wpm = Some(e.saturating_add(step)),
            None => c.char_wpm = c.char_wpm.saturating_add(step).min(rule.max_wpm),
        }
    }

    fn lower(&mut self, rule: &AdaptRule) {
        let c = &mut self.current;
        let step = rule.step_wpm;
        match c.effective_wpm {
            Some(e) => c.effective_wpm = Some(e.saturating_sub(step).max(MIN_WPM)),
            None => c.char_wpm = c.char_wpm.saturating_sub(step).max(rule.min_wpm),
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
    fn a_step_wider_than_the_whole_speed_range_is_refused() {
        let ok = AdaptRule {
            step_wpm: MAX_WPM - MIN_WPM,
            ..AdaptRule::default()
        };
        assert_eq!(ok.problem(), None);
        for step in [MAX_WPM - MIN_WPM + 1, u32::MAX] {
            let r = AdaptRule {
                step_wpm: step,
                ..AdaptRule::default()
            };
            assert!(r.problem().is_some(), "step {step}");
        }
    }

    #[test]
    fn a_cer_exactly_at_a_threshold_moves_the_speed() {
        // 1/20 and 3/20 are the very doubles 0.05 and 0.15.
        assert_eq!(score(1, 20).cer(), RAISE_CER);
        assert_eq!(score(3, 20).cer(), LOWER_CER);
        let mut s = SpeedState::new(
            20,
            Farnsworth::Off,
            Level::Intermediate,
            Some(AdaptRule::default()),
        );
        let mut last = None;
        for _ in 0..WINDOW {
            last = s.record(&score(1, 20), 0);
        }
        assert_eq!(last.unwrap().char_wpm, 22, "cer == raise_cer raises");
        let mut s = SpeedState::new(
            20,
            Farnsworth::Off,
            Level::Intermediate,
            Some(AdaptRule::default()),
        );
        for _ in 0..WINDOW {
            last = s.record(&score(3, 20), 0);
        }
        assert_eq!(last.unwrap().char_wpm, 18, "cer == lower_cer lowers");
    }

    #[test]
    fn cer_thresholds_outside_zero_to_one_are_refused() {
        let d = AdaptRule::default();
        for r in [
            AdaptRule {
                raise_cer: -0.01,
                ..d
            },
            AdaptRule {
                lower_cer: 1.01,
                ..d
            },
        ] {
            assert!(r.problem().is_some(), "{r:?}");
        }
        assert_eq!(
            AdaptRule {
                raise_cer: 0.0,
                lower_cer: 1.0,
                ..d
            }
            .problem(),
            None
        );
    }

    #[test]
    fn a_huge_step_never_overflows_or_turns_a_raise_into_a_drop() {
        // The arithmetic's own guard, for a rule that skipped `problem`.
        let rule = AdaptRule {
            window: 1,
            step_wpm: u32::MAX,
            ..AdaptRule::default()
        };
        let mut s = SpeedState::new(20, Farnsworth::Off, Level::Intermediate, Some(rule));
        assert_eq!(s.record(&score(0, 20), 0).unwrap().char_wpm, MAX_WPM);
        let mut s = SpeedState::new(20, Farnsworth::Off, Level::Intermediate, Some(rule));
        assert_eq!(s.record(&score(10, 20), 0).unwrap().char_wpm, MIN_WPM);
        // Farnsworth: the effective speed closes up at once.
        let mut s = SpeedState::new(20, Farnsworth::Wpm(10), Level::Intermediate, Some(rule));
        let c = s.record(&score(0, 20), 0).unwrap();
        assert_eq!((c.char_wpm, c.effective_wpm), (20, None));
        let mut s = SpeedState::new(20, Farnsworth::Wpm(10), Level::Intermediate, Some(rule));
        assert_eq!(
            s.record(&score(10, 20), 0).unwrap().effective_wpm,
            Some(MIN_WPM)
        );
    }

    #[test]
    fn four_clean_overs_raise_the_speed() {
        let mut s = SpeedState::new(
            20,
            Farnsworth::Off,
            Level::Intermediate,
            Some(AdaptRule::default()),
        );
        for _ in 0..3 {
            assert_eq!(s.record(&score(0, 20), 0), None);
        }
        let c = s.record(&score(0, 20), 0).expect("raised");
        assert_eq!(c.char_wpm, 22);
    }

    #[test]
    fn a_poor_window_lowers_the_speed() {
        let mut s = SpeedState::new(
            20,
            Farnsworth::Off,
            Level::Intermediate,
            Some(AdaptRule::default()),
        );
        for _ in 0..3 {
            s.record(&score(5, 20), 0);
        }
        assert_eq!(s.record(&score(5, 20), 0).unwrap().char_wpm, 18);
    }

    #[test]
    fn an_agn_in_the_window_blocks_a_raise() {
        let mut s = SpeedState::new(
            20,
            Farnsworth::Off,
            Level::Intermediate,
            Some(AdaptRule::default()),
        );
        s.record(&score(0, 20), 1);
        for _ in 0..3 {
            assert_eq!(s.record(&score(0, 20), 0), None);
        }
        // The AGN has slid out of the window now.
        assert_eq!(s.record(&score(0, 20), 0).unwrap().char_wpm, 22);
    }

    #[test]
    fn farnsworth_spacing_closes_up_before_the_characters_speed_up() {
        let mut s = SpeedState::new(
            20,
            Farnsworth::Wpm(16),
            Level::Intermediate,
            Some(AdaptRule::default()),
        );
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
        let mut s = SpeedState::new(
            MAX_WPM,
            Farnsworth::Off,
            Level::Advanced,
            Some(AdaptRule::default()),
        );
        for _ in 0..WINDOW {
            assert_eq!(s.record(&score(0, 20), 0), None);
        }
        let mut s = SpeedState::new(
            MIN_WPM,
            Farnsworth::Off,
            Level::Beginner,
            Some(AdaptRule::default()),
        );
        for _ in 0..WINDOW {
            assert_eq!(s.record(&score(10, 20), 0), None);
        }
        let mut s = SpeedState::new(20, Farnsworth::Off, Level::Beginner, None);
        for _ in 0..10 {
            assert_eq!(s.record(&score(10, 20), 0), None);
        }
    }

    #[test]
    fn the_preset_farnsworth_applies_only_below_the_character_speed() {
        let s = SpeedState::new(
            20,
            Farnsworth::Preset,
            Level::Beginner,
            Some(AdaptRule::default()),
        );
        assert_eq!(s.current.effective_wpm, Some(10));
        let s = SpeedState::new(
            8,
            Farnsworth::Preset,
            Level::Beginner,
            Some(AdaptRule::default()),
        );
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
