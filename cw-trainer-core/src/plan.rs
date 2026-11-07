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

//! Planning a QSO: its scenario, its script, and the overs it is played as.

use std::time::Duration;

use cat_morse::scenario::{ContestExchange, Difficulty, Party, Start, Transmission};
use cat_morse::{generate, Rng, ScenarioConfig, ScenarioKind, Script};

use crate::error::CoreError;
use crate::options::{CopyOptions, Kind, Level};
use crate::speed::{difficulty_for, SpeedSetting};

/// The concrete scenario for one QSO, drawn from `rng`.
pub fn scenario_kind(kind: Kind, level: Level, rng: &mut Rng) -> (ScenarioKind, &'static str) {
    let kind = match kind {
        Kind::Mixed => *rng.pick(&[
            Kind::RagChew,
            Kind::Contest,
            Kind::Pota,
            Kind::Sota,
            Kind::Pileup,
        ]),
        k => k,
    };
    let sk = match kind {
        Kind::RagChew | Kind::Mixed => ScenarioKind::RagChew,
        Kind::Contest => ScenarioKind::Contest {
            exchange: if rng.chance(0.5) {
                ContestExchange::CqZone
            } else {
                ContestExchange::Serial
            },
            cut_numbers: level == Level::Advanced,
        },
        Kind::Pota => ScenarioKind::Pota {
            park_to_park: rng.chance(0.25),
        },
        Kind::Sota => ScenarioKind::Sota {
            summit_to_summit: rng.chance(0.25),
        },
        Kind::Pileup => {
            let (lo, hi) = match level {
                Level::Beginner => (2, 3),
                Level::Intermediate => (3, 5),
                Level::Advanced => (5, 8),
            };
            ScenarioKind::DxPileup {
                callers: rng.range_u32(lo, hi) as u8,
            }
        }
    };
    (sk, kind.name())
}

/// One turn on the air.
#[derive(Debug, Clone, PartialEq)]
pub enum Over<'a> {
    /// The operator's own line: shown, not played.
    You(&'a Transmission),
    /// One or more stations; `starts` are relative to the over.
    Them {
        lines: Vec<&'a Transmission>,
        starts: Vec<Duration>,
    },
}

/// Group a script's transmissions into overs: a run of *them* lines that
/// start with the previous one (a pileup) is one over.
pub fn group_overs(script: &Script) -> Vec<Over<'_>> {
    let t = &script.transmissions;
    over_indices(script)
        .into_iter()
        .map(|o| match o {
            OverAt::You(i) => Over::You(&t[i]),
            OverAt::Them { lines, starts } => Over::Them {
                lines: lines.into_iter().map(|i| &t[i]).collect(),
                starts,
            },
        })
        .collect()
}

/// An [`Over`] by position in [`Script::transmissions`] rather than by
/// reference, so it can be kept beside the script it indexes.
#[derive(Debug, Clone, PartialEq)]
pub enum OverAt {
    You(usize),
    Them {
        lines: Vec<usize>,
        starts: Vec<Duration>,
    },
}

/// [`group_overs`], by position.
pub fn over_indices(script: &Script) -> Vec<OverAt> {
    let mut overs: Vec<OverAt> = Vec::new();
    for (i, t) in script.transmissions.iter().enumerate() {
        match (t.from, t.start, overs.last_mut()) {
            (Party::You, _, _) => overs.push(OverAt::You(i)),
            (
                Party::Them(_),
                Start::WithPrevious { delay },
                Some(OverAt::Them { lines, starts }),
            ) => {
                let at = *starts.last().expect("non-empty over") + delay;
                lines.push(i);
                starts.push(at);
            }
            (Party::Them(_), _, _) => overs.push(OverAt::Them {
                lines: vec![i],
                starts: vec![Duration::ZERO],
            }),
        }
    }
    overs
}

/// What is expected from one *them* over: the text, or (several stations
/// at once) every word that was actually sent, to be copied in any order.
///
/// Every word, not each call once: a pileup caller often sends his call
/// twice (`W1ABC W1ABC`), and a faithful copy of that must score clean —
/// otherwise good copy reads as insertions and drags the speed down.
pub fn expected_for(lines: &[&Transmission]) -> Vec<String> {
    if lines.len() == 1 {
        return vec![lines[0].text.clone()];
    }
    lines
        .iter()
        .flat_map(|l| l.text.split_whitespace().map(String::from))
        .collect()
}

/// Everything one QSO is built from; shared with the tests, which replay
/// it to know what a perfect operator would type.
#[derive(Debug, Clone)]
pub struct QsoPlan {
    pub label: &'static str,
    pub difficulty: Difficulty,
    pub script: Script,
    pub rng: Rng,
}

pub fn plan_qso(opts: &CopyOptions, qso: u32, speed: SpeedSetting) -> Result<QsoPlan, CoreError> {
    let mut rng = Rng::new(opts.seed).fork(u64::from(qso));
    let (kind, label) = scenario_kind(opts.kind, opts.level, &mut rng);
    let mut difficulty = difficulty_for(opts.level, speed);
    if let Some(band) = opts.band {
        difficulty.band = band;
    }
    let script = generate(&ScenarioConfig {
        kind,
        my_call: opts.call.clone(),
        difficulty: difficulty.clone(),
        seed: rng.next_u64(),
    })
    .map_err(CoreError::Generate)?;
    Ok(QsoPlan {
        label,
        difficulty,
        script,
        rng,
    })
}

#[cfg(test)]
mod tests {
    use cat_morse::{align_unordered, Callsign, ScoreOptions};

    use super::*;
    use crate::options::Farnsworth;
    use crate::speed::SpeedState;

    fn options(kind: Kind, qsos: u32, seed: u64) -> CopyOptions {
        CopyOptions {
            kind,
            level: Level::Intermediate,
            qsos,
            call: Callsign::parse("KF0UWV").unwrap(),
            wpm: 20,
            farnsworth: Farnsworth::Off,
            pitch_hz: 600.0,
            seed,
            adapt: None,
            band: None,
            started: "2026-10-06T00:00:00Z".to_string(),
            audio_out: "Test speaker".to_string(),
        }
    }

    #[test]
    fn a_ragchew_alternates_single_station_overs_with_your_lines() {
        let opts = options(Kind::RagChew, 1, 7);
        let plan = plan_qso(
            &opts,
            0,
            SpeedState::new(20, Farnsworth::Off, Level::Intermediate, None).current,
        )
        .unwrap();
        let overs = group_overs(&plan.script);
        assert_eq!(overs.len(), plan.script.transmissions.len());
        assert!(overs.iter().any(|o| matches!(o, Over::You(_))));
        for o in &overs {
            if let Over::Them { lines, starts } = o {
                assert_eq!(lines.len(), 1);
                assert_eq!(starts, &vec![Duration::ZERO]);
            }
        }
    }

    #[test]
    fn pileup_callers_starting_together_are_one_over_with_staggered_starts() {
        let opts = options(Kind::Pileup, 1, 11);
        let speed = SpeedState::new(20, Farnsworth::Off, Level::Intermediate, None).current;
        let plan = plan_qso(&opts, 0, speed).unwrap();
        let overs = group_overs(&plan.script);
        let first = overs
            .iter()
            .find_map(|o| match o {
                Over::Them { lines, starts } if lines.len() > 1 => Some((lines, starts)),
                _ => None,
            })
            .expect("a pileup has a multi-station over");
        let (lines, starts) = first;
        assert_eq!(lines.len(), starts.len());
        assert!(starts.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(starts[0], Duration::ZERO);
        assert!(expected_for(lines).len() >= 2);
    }

    #[test]
    fn a_call_sent_twice_in_a_pileup_is_expected_twice() {
        // The reviewer's case: a faithful copy of what was sent is clean,
        // in any order.
        let line = |text: &str| Transmission {
            from: Party::Them(0),
            text: text.to_string(),
            start: Start::AfterPrevious {
                gap: Duration::ZERO,
            },
        };
        let (a, b) = (line("W1ABC W1ABC"), line("K2XYZ"));
        let expected = expected_for(&[&a, &b]);
        assert_eq!(expected, vec!["W1ABC", "W1ABC", "K2XYZ"]);
        let refs: Vec<&str> = expected.iter().map(String::as_str).collect();
        for copy in ["W1ABC W1ABC K2XYZ", "K2XYZ W1ABC W1ABC"] {
            let u = align_unordered(&refs, copy, &ScoreOptions::default()).unwrap();
            assert_eq!(u.score().cer(), 0.0, "{copy}");
        }
    }

    // --- whole sessions -------------------------------------------------

    #[test]
    fn the_same_seed_plans_the_same_qso_and_each_qso_differs() {
        let opts = options(Kind::Mixed, 3, 42);
        let speed = SpeedState::new(20, Farnsworth::Off, Level::Intermediate, None).current;
        let a = plan_qso(&opts, 1, speed).unwrap();
        let b = plan_qso(&opts, 1, speed).unwrap();
        assert_eq!(a.script, b.script);
        assert_eq!(a.label, b.label);
        let c = plan_qso(&opts, 2, speed).unwrap();
        assert_ne!(a.script, c.script);
    }
}
