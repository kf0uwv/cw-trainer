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

//! Configurable session rules: the adaptive-speed rule and a band
//! override, and how they show in the record.

use std::fs;
use std::path::{Path, PathBuf};

use cw_trainer_core::cat_morse::scenario::BandConditions;
use cw_trainer_core::cat_morse::Callsign;
use cw_trainer_core::copy::{CopySession, OverOutcome, Step};
use cw_trainer_core::options::{CopyOptions, Farnsworth, Kind, Level, MAX_WPM, MIN_WPM};
use cw_trainer_core::record::{RecordedBand, SessionRecord};
use cw_trainer_core::speed::{AdaptRule, LOWER_CER, RAISE_CER, STEP_WPM, WINDOW};
use cw_trainer_core::CoreError;

fn records() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/records")
}

fn options(adapt: Option<AdaptRule>) -> CopyOptions {
    CopyOptions {
        kind: Kind::RagChew,
        level: Level::Intermediate,
        qsos: 2,
        call: Callsign::parse("KF0UWV").unwrap(),
        wpm: 20,
        farnsworth: Farnsworth::Off,
        pitch_hz: 600.0,
        seed: 7,
        adapt,
        band: None,
        started: "2026-11-07T12:00:00Z".to_string(),
        audio_out: "Test speaker".to_string(),
    }
}

/// Copy every over perfectly; the record and each over's band.
fn run(opts: CopyOptions) -> (SessionRecord, Vec<Option<f32>>) {
    let mut s = CopySession::new(opts, 8000).unwrap();
    let mut noise = Vec::new();
    while let Some(step) = s.next_step().unwrap() {
        if let Step::ThemOver(o) = step {
            noise.push(o.audio.channel().noise.map(|n| n.snr_db));
            s.submit(OverOutcome::Copied {
                text: o.expected.join(" "),
                agn: 0,
                audio_gaps: 0,
                clipped: 0,
            })
            .unwrap();
        }
    }
    (s.finish(None).record, noise)
}

#[test]
fn the_default_rule_is_the_clis_rule() {
    let r = AdaptRule::default();
    assert_eq!(r.window as usize, WINDOW);
    assert_eq!((r.raise_cer, r.lower_cer), (RAISE_CER, LOWER_CER));
    assert_eq!(r.step_wpm, STEP_WPM);
    assert_eq!((r.min_wpm, r.max_wpm), (MIN_WPM, MAX_WPM));
}

#[test]
fn the_default_rule_writes_the_very_bytes_the_cli_wrote() {
    let (r, _) = run(options(Some(AdaptRule::default())));
    let fixture = fs::read_to_string(records().join("adapt.record.json")).unwrap();
    assert_eq!(serde_json::to_string_pretty(&r).unwrap() + "\n", fixture);
    let line = fs::read_to_string(records().join("adapt.history.jsonl")).unwrap();
    assert_eq!(serde_json::to_string(&r).unwrap() + "\n", line);
}

#[test]
fn a_non_default_rule_is_recorded_as_a_trailing_field_only() {
    let rule = AdaptRule {
        window: 2,
        step_wpm: 3,
        ..AdaptRule::default()
    };
    let (r, _) = run(options(Some(rule)));
    assert_eq!(r.adapt_rule, Some(rule));
    let mut without = r.clone();
    without.adapt_rule = None;
    let plain = serde_json::to_string(&without).unwrap();
    let with = serde_json::to_string(&r).unwrap();
    let rule_json = serde_json::to_string(&rule).unwrap();
    assert_eq!(
        with,
        format!("{},\"adapt_rule\":{rule_json}}}", &plain[..plain.len() - 1])
    );
    let back: SessionRecord = serde_json::from_str(&with).unwrap();
    assert_eq!(back, r);
}

#[test]
fn a_shorter_window_and_a_bigger_step_adapt_sooner_and_further() {
    let (default, _) = run(options(Some(AdaptRule::default())));
    let (custom, _) = run(options(Some(AdaptRule {
        window: 2,
        step_wpm: 3,
        ..AdaptRule::default()
    })));
    assert!(custom.end_speed.char_wpm > default.end_speed.char_wpm);
    assert_eq!((custom.end_speed.char_wpm - 20) % 3, 0);
}

#[test]
fn no_rule_keeps_the_speed() {
    let (r, _) = run(options(None));
    assert_eq!(r.start_speed, r.end_speed);
    assert_eq!(r.adapt_rule, None);
}

#[test]
fn the_start_speed_is_clamped_into_the_rules_bounds() {
    let rule = AdaptRule {
        min_wpm: 25,
        max_wpm: 30,
        ..AdaptRule::default()
    };
    let (r, _) = run(options(Some(rule)));
    assert_eq!(r.start_speed.char_wpm, 25);
    assert!(r.end_speed.char_wpm <= 30);
    let low = AdaptRule {
        min_wpm: 10,
        max_wpm: 15,
        ..AdaptRule::default()
    };
    assert_eq!(run(options(Some(low))).0.start_speed.char_wpm, 15);
}

#[test]
fn an_invalid_rule_is_refused() {
    let d = AdaptRule::default();
    let bad = [
        AdaptRule { window: 0, ..d },
        AdaptRule { step_wpm: 0, ..d },
        AdaptRule {
            raise_cer: f64::NAN,
            ..d
        },
        AdaptRule {
            lower_cer: f64::NAN,
            ..d
        },
        AdaptRule {
            raise_cer: f64::INFINITY,
            ..d
        },
        AdaptRule {
            raise_cer: 0.2,
            lower_cer: 0.2,
            ..d
        },
        AdaptRule {
            raise_cer: 0.3,
            lower_cer: 0.2,
            ..d
        },
        AdaptRule {
            min_wpm: MIN_WPM - 1,
            ..d
        },
        AdaptRule {
            max_wpm: MAX_WPM + 1,
            ..d
        },
        AdaptRule {
            min_wpm: 30,
            max_wpm: 20,
            ..d
        },
    ];
    for rule in bad {
        assert!(
            matches!(
                CopySession::new(options(Some(rule)), 8000),
                Err(CoreError::InvalidOptions(_))
            ),
            "{rule:?}"
        );
    }
    assert!(CopySession::new(options(Some(d)), 8000).is_ok());
}

#[test]
fn a_band_override_replaces_the_levels_band_and_is_recorded() {
    let (preset, noise) = run(options(None));
    assert!(noise.iter().all(|n| n.is_some()), "intermediate is noisy");
    assert_eq!(preset.band, None);

    let mut opts = options(None);
    opts.band = Some(BandConditions::clean());
    let (clean, noise) = run(opts);
    assert!(noise.iter().all(|n| n.is_none()), "{noise:?}");
    assert_eq!(
        clean.band,
        Some(RecordedBand::from(BandConditions::clean()))
    );
    let mut without = clean.clone();
    without.band = None;
    let plain = serde_json::to_string(&without).unwrap();
    assert_eq!(
        serde_json::to_string(&clean).unwrap(),
        format!(
            "{},\"band\":{}}}",
            &plain[..plain.len() - 1],
            serde_json::to_string(&BandConditions::clean()).unwrap()
        )
    );
}

#[test]
fn a_band_override_equal_to_the_preset_is_not_recorded() {
    let mut opts = options(None);
    opts.band = Some(Level::Intermediate.difficulty().band);
    let (r, _) = run(opts);
    assert_eq!(r.band, None);
    assert_eq!(r, run(options(None)).0);
}

#[test]
fn an_invalid_band_is_refused() {
    let mut opts = options(None);
    opts.band = Some(BandConditions {
        snr_db: Some((20.0, 10.0)),
        ..BandConditions::clean()
    });
    assert!(matches!(
        CopySession::new(opts, 8000),
        Err(CoreError::InvalidOptions(_))
    ));
}

#[test]
fn old_lines_read_with_no_rule_and_no_band() {
    let text = fs::read_to_string(records().join("old-history.jsonl")).unwrap();
    for line in text.lines() {
        let r: SessionRecord = serde_json::from_str(line).unwrap();
        assert_eq!((r.adapt_rule, r.band), (None, None));
    }
}

#[test]
fn unknown_fields_from_a_later_version_are_tolerated() {
    let text = fs::read_to_string(records().join("ragchew.history.jsonl")).unwrap();
    let line = text.lines().next().unwrap();
    let later = format!("{},\"from_the_future\":[1,2]}}", &line[..line.len() - 1]);
    let r: SessionRecord = serde_json::from_str(&later).unwrap();
    assert_eq!(r, serde_json::from_str::<SessionRecord>(line).unwrap());
}

#[test]
fn a_recorded_band_writes_cat_morses_own_shape_byte_for_byte() {
    // Every impairment present, so every field is written.
    for level in [Level::Beginner, Level::Intermediate, Level::Advanced] {
        let band = level.difficulty().band;
        assert_eq!(
            serde_json::to_string(&RecordedBand::from(band)).unwrap(),
            serde_json::to_string(&band).unwrap(),
            "{level:?}"
        );
        assert_eq!(BandConditions::from(RecordedBand::from(band)), band);
    }
}

#[test]
fn a_recorded_band_missing_fields_still_reads() {
    // A line written by a version whose band had fewer fields (or that a
    // later band type would not read) keeps the whole record readable.
    let text = fs::read_to_string(records().join("ragchew.history.jsonl")).unwrap();
    let line = text.lines().next().unwrap();
    let with_band = format!(
        "{},\"band\":{{\"snr_db\":[9.0,11.0],\"qsb\":{{\"chance\":0.4}}}}}}",
        &line[..line.len() - 1]
    );
    let r: SessionRecord = serde_json::from_str(&with_band).unwrap();
    let band = r.band.unwrap();
    assert_eq!(band.snr_db, Some((9.0, 11.0)));
    assert_eq!(band.qsb.unwrap().chance, 0.4);
    assert_eq!(
        (band.qrn, band.qrm, band.rx_bandwidth_hz),
        (None, None, None)
    );
}
