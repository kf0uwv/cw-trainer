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

//! Golden tests: whole copy sessions against committed fixtures.
//!
//! Each scenario runs a seeded session with a scripted operator (perfect
//! copy, typos, AGN, skips, partial copy, quitting) on doubles only — a
//! fake speaker and a scripted terminal, never a sound card — and compares
//! byte for byte with fixtures:
//!
//! - `<name>.record.json` and `<name>.history.jsonl`: the session record
//!   (pretty) and the history file written, in
//!   `cw-trainer-core/tests/fixtures/records/`, shared with the library's
//!   own tests (which also check that old history lines still parse);
//! - in `tests/fixtures/golden/`, `<name>.transcript.txt`: everything the
//!   terminal showed (`\r\n`), and `<name>.stats.json`: the file written;
//! - `<name>.pcm.json`: length (exact) and RMS (to a tolerance) of each
//!   play of an over.
//!
//! The fixtures were recorded from the code before the session logic
//! moved into a library; they prove the move changed nothing. To record
//! them again (only for a deliberate behaviour change), run the tests with
//! `CW_TRAINER_BLESS=1`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use cat_morse::Callsign;
use serde::{Deserialize, Serialize};

use crate::audio::tests::FakeSink;
use crate::session::{
    expected_for, group_overs, plan_qso, run_copy, Over, SessionRecord, SpeedSetting, SpeedState,
};
use crate::store::Store;
use crate::term::tests::ScriptedTerminal;
use crate::term::Key;
use crate::{AdaptRule, CopyOptions, Farnsworth, Kind, Level};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/golden")
}

/// Records and history lines are shared with the library's own tests.
fn record_fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("cw-trainer-core/tests/fixtures/records")
}

fn bless() -> bool {
    std::env::var_os("CW_TRAINER_BLESS").is_some_and(|v| v == "1")
}

/// Compare `actual` with the fixture `name`, or record it when blessing.
fn check(name: &str, actual: &str) {
    check_in(&fixtures(), name, actual)
}

fn check_in(dir: &Path, name: &str, actual: &str) {
    let path = dir.join(name);
    if bless() {
        fs::create_dir_all(dir).unwrap();
        fs::write(&path, actual).unwrap();
        return;
    }
    let expected = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (record with CW_TRAINER_BLESS=1)", path.display()));
    assert!(
        expected == actual,
        "{name} differs from its fixture\n--- expected\n{expected}\n--- actual\n{actual}"
    );
}

fn options(kind: Kind, level: Level, qsos: u32, seed: u64) -> CopyOptions {
    CopyOptions {
        kind,
        level,
        qsos,
        call: Callsign::parse("KF0UWV").unwrap(),
        wpm: 20,
        farnsworth: Farnsworth::Off,
        pitch_hz: 600.0,
        seed,
        adapt: None,
        band: None,
        started: "2026-11-07T12:00:00Z".to_string(),
        audio_out: "Test speaker".to_string(),
    }
}

/// How the scripted operator copies.
#[derive(Clone, Copy)]
enum Style {
    /// Every over copied perfectly (a pileup in reverse order).
    Perfect,
    /// A cycle: perfect, typo fixed with backspace plus a typo left in,
    /// AGN then perfect, skip, partial (last three dropped), an extra word.
    Mixed,
}

/// Where the operator quits, if anywhere: `(qso, them over)` or
/// `(qso, you line)`, both counted from 0 within the QSO.
#[derive(Clone, Copy, Default)]
struct Quits {
    them: Option<(u32, usize)>,
    you: Option<(u32, usize)>,
}

fn typed(text: &str) -> Vec<(Duration, Key)> {
    text.chars()
        .map(|c| (Duration::ZERO, Key::Char(c)))
        .collect()
}

fn enter() -> (Duration, Key) {
    (Duration::ZERO, Key::Enter)
}

/// One them over's keys for cycle step `n`.
fn copy_keys(expected: &[String], several: bool, style: Style, n: usize) -> Vec<(Duration, Key)> {
    let text = if several {
        let mut words = expected.to_vec();
        words.reverse();
        words.join(" ")
    } else {
        expected.join(" ")
    };
    let mut keys = Vec::new();
    let step = match style {
        Style::Perfect => 0,
        Style::Mixed => n % 6,
    };
    match step {
        0 => keys.extend(typed(&text)),
        1 => {
            // A wrong first character, corrected; a wrong last one, left.
            let mut chars: Vec<char> = text.chars().collect();
            let wrong = |c: char| if c == 'E' { 'T' } else { 'E' };
            keys.push((Duration::ZERO, Key::Char(wrong(chars[0]))));
            keys.push((Duration::ZERO, Key::Backspace));
            let last = chars.len() - 1;
            chars[last] = wrong(chars[last]);
            keys.extend(typed(&chars.iter().collect::<String>()));
        }
        2 => {
            keys.push((Duration::ZERO, Key::Agn));
            keys.extend(typed(&text));
        }
        3 => {
            keys.push((Duration::ZERO, Key::Skip));
            return keys;
        }
        4 => {
            let keep = text.chars().count().saturating_sub(3);
            keys.extend(typed(&text.chars().take(keep).collect::<String>()));
        }
        _ => {
            keys.extend(typed(&text));
            keys.extend(typed(" QRZ"));
        }
    }
    keys.push(enter());
    keys
}

/// The operator's keys for QSOs `0..qsos`, each planned at `speeds[q]`.
fn operator(
    opts: &CopyOptions,
    speeds: &[SpeedSetting],
    style: Style,
    quits: Quits,
) -> Vec<(Duration, Key)> {
    let mut keys = Vec::new();
    let mut n = 0;
    for (q, speed) in speeds.iter().enumerate() {
        let q = q as u32;
        let plan = plan_qso(opts, q, *speed).unwrap();
        let (mut them, mut you) = (0, 0);
        for over in group_overs(&plan.script) {
            match over {
                Over::You(_) => {
                    if quits.you == Some((q, you)) {
                        keys.push((Duration::ZERO, Key::Quit));
                        return keys;
                    }
                    keys.push(enter());
                    you += 1;
                }
                Over::Them { lines, .. } => {
                    if quits.them == Some((q, them)) {
                        keys.push((Duration::ZERO, Key::Quit));
                        return keys;
                    }
                    let expected = expected_for(&lines);
                    keys.extend(copy_keys(&expected, lines.len() > 1, style, n));
                    them += 1;
                    n += 1;
                }
            }
        }
    }
    keys
}

fn fixed_speeds(opts: &CopyOptions) -> Vec<SpeedSetting> {
    let s = SpeedState::new(opts.wpm, opts.farnsworth, opts.level, None).current;
    vec![s; opts.qsos as usize]
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Play {
    samples: usize,
    rms: f64,
}

/// Each play of an over: the speaker's audio between clears.
fn plays(sink: &FakeSink) -> Vec<Play> {
    let mut out = Vec::new();
    let mut from = 0;
    let mut marks = sink.clear_marks.clone();
    marks.push(sink.played.len());
    for to in marks {
        if to > from {
            let s = &sink.played[from..to];
            let sum: f64 = s.iter().map(|x| f64::from(*x) * f64::from(*x)).sum();
            out.push(Play {
                samples: s.len(),
                rms: (sum / s.len() as f64).sqrt(),
            });
        }
        from = to;
    }
    out
}

fn check_plays(name: &str, actual: &[Play]) {
    let path = fixtures().join(format!("{name}.pcm.json"));
    if bless() {
        fs::create_dir_all(fixtures()).unwrap();
        fs::write(&path, serde_json::to_string_pretty(actual).unwrap() + "\n").unwrap();
        return;
    }
    let expected: Vec<Play> = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(expected.len(), actual.len(), "{name}: number of plays");
    for (i, (e, a)) in expected.iter().zip(actual).enumerate() {
        assert_eq!(e.samples, a.samples, "{name}: play {i} length");
        // PCM is bit-identical per platform, within float tolerance across.
        let tol = 1e-4 * e.rms.abs() + 1e-9;
        assert!(
            (e.rms - a.rms).abs() <= tol,
            "{name}: play {i} rms {} vs {}",
            e.rms,
            a.rms
        );
    }
}

/// How a scenario's session is set up to end.
#[derive(Clone, Copy, Default)]
struct Faults {
    /// The speaker fails after this many samples.
    audio_after: Option<usize>,
    /// The terminal fails when the keys run out (instead of quitting).
    terminal: bool,
}

/// Run one session into `dir` and check every fixture of `name`.
fn golden(
    name: &str,
    dir: &Path,
    opts: &CopyOptions,
    keys: Vec<(Duration, Key)>,
    faults: Faults,
) -> SessionRecord {
    let mut sink = FakeSink::new(8000);
    sink.fail_after = faults.audio_after;
    let mut term = ScriptedTerminal::new(keys);
    term.fail_when_done = faults.terminal;
    let mut store = Store::open(Some(dir.to_path_buf()));
    let result = run_copy(opts, &mut sink, &mut term, &mut store);
    let record = match result {
        Ok(r) => r,
        Err(_) => store
            .read_history()
            .pop()
            .expect("a failed session is still kept"),
    };

    check_in(
        &record_fixtures(),
        &format!("{name}.record.json"),
        &(serde_json::to_string_pretty(&record).unwrap() + "\n"),
    );
    check(&format!("{name}.transcript.txt"), &term.text());
    check(
        &format!("{name}.stats.json"),
        &fs::read_to_string(dir.join("stats.json")).unwrap(),
    );
    check_in(
        &record_fixtures(),
        &format!("{name}.history.jsonl"),
        &fs::read_to_string(dir.join("history.jsonl")).unwrap(),
    );
    check_plays(name, &plays(&sink));
    record
}

fn simple(name: &str, opts: CopyOptions, style: Style, quits: Quits) -> SessionRecord {
    let tmp = tempfile::tempdir().unwrap();
    let keys = operator(&opts, &fixed_speeds(&opts), style, quits);
    golden(name, tmp.path(), &opts, keys, Faults::default())
}

#[test]
fn golden_ragchew_mixed_operator() {
    let r = simple(
        "ragchew",
        options(Kind::RagChew, Level::Intermediate, 2, 7),
        Style::Mixed,
        Quits::default(),
    );
    assert!(!r.quit_early);
    assert!(r.overs.iter().any(|o| o.skipped));
    assert!(r.overs.iter().any(|o| o.agn > 0));
}

#[test]
fn golden_contest_advanced() {
    simple(
        "contest",
        options(Kind::Contest, Level::Advanced, 2, 5),
        Style::Mixed,
        Quits::default(),
    );
}

#[test]
fn golden_pota_beginner_preset_farnsworth() {
    let mut opts = options(Kind::Pota, Level::Beginner, 1, 3);
    opts.farnsworth = Farnsworth::Preset;
    let r = simple("pota", opts, Style::Mixed, Quits::default());
    assert!(r.start_speed.effective_wpm.is_some());
}

#[test]
fn golden_sota_farnsworth_wpm() {
    let mut opts = options(Kind::Sota, Level::Intermediate, 1, 4);
    opts.farnsworth = Farnsworth::Wpm(15);
    opts.pitch_hz = 700.0;
    simple("sota", opts, Style::Mixed, Quits::default());
}

#[test]
fn golden_pileup_any_order() {
    let r = simple(
        "pileup",
        options(Kind::Pileup, Level::Advanced, 2, 11),
        Style::Mixed,
        Quits::default(),
    );
    assert!(r.overs.iter().any(|o| o.expected.len() > 1));
}

#[test]
fn golden_mixed_quit_in_a_them_over() {
    let r = simple(
        "mixed-quit",
        options(Kind::Mixed, Level::Intermediate, 3, 99),
        Style::Mixed,
        Quits {
            them: Some((1, 2)),
            you: None,
        },
    );
    assert!(r.quit_early);
}

#[test]
fn golden_quit_in_a_you_line() {
    let r = simple(
        "you-quit",
        options(Kind::RagChew, Level::Beginner, 2, 8),
        Style::Perfect,
        Quits {
            them: None,
            you: Some((0, 1)),
        },
    );
    assert!(r.quit_early);
}

#[test]
fn golden_adaptive_speed_across_qsos() {
    let mut opts = options(Kind::RagChew, Level::Intermediate, 2, 7);
    opts.adapt = Some(AdaptRule::default());
    // QSO 2 is planned at the speed QSO 1 ends on, so learn it first.
    let first = {
        let tmp = tempfile::tempdir().unwrap();
        let mut one = opts.clone();
        one.qsos = 1;
        let keys = operator(&one, &fixed_speeds(&one), Style::Perfect, Quits::default());
        run_copy(
            &one,
            &mut FakeSink::new(8000),
            &mut ScriptedTerminal::new(keys),
            &mut Store::open(Some(tmp.path().to_path_buf())),
        )
        .unwrap()
    };
    let speeds = [first.start_speed, first.end_speed];
    let tmp = tempfile::tempdir().unwrap();
    let keys = operator(&opts, &speeds, Style::Perfect, Quits::default());
    let r = golden("adapt", tmp.path(), &opts, keys, Faults::default());
    assert!(r.end_speed.char_wpm > r.start_speed.char_wpm);
}

#[test]
fn golden_audio_failure() {
    let opts = options(Kind::RagChew, Level::Intermediate, 2, 7);
    let tmp = tempfile::tempdir().unwrap();
    let keys = operator(&opts, &fixed_speeds(&opts), Style::Mixed, Quits::default());
    let r = golden(
        "audio-failure",
        tmp.path(),
        &opts,
        keys,
        Faults {
            // During the fourth play, after three overs are copied.
            audio_after: Some(1_300_000),
            terminal: false,
        },
    );
    assert!(r.ended_by_error.is_some());
    assert!(r.overs.len() >= 2, "the overs before the failure are kept");
}

#[test]
fn golden_terminal_failure() {
    let opts = options(Kind::Contest, Level::Intermediate, 2, 6);
    let tmp = tempfile::tempdir().unwrap();
    let mut keys = operator(&opts, &fixed_speeds(&opts), Style::Mixed, Quits::default());
    keys.truncate(keys.len() / 2);
    let r = golden(
        "terminal-failure",
        tmp.path(),
        &opts,
        keys,
        Faults {
            audio_after: None,
            terminal: true,
        },
    );
    assert!(r.ended_by_error.is_some());
}

#[test]
fn golden_second_session_recalls_the_first_and_merges_stats() {
    let tmp = tempfile::tempdir().unwrap();
    let first = options(Kind::RagChew, Level::Intermediate, 1, 7);
    let keys = operator(
        &first,
        &fixed_speeds(&first),
        Style::Mixed,
        Quits::default(),
    );
    run_copy(
        &first,
        &mut FakeSink::new(8000),
        &mut ScriptedTerminal::new(keys),
        &mut Store::open(Some(tmp.path().to_path_buf())),
    )
    .unwrap();
    let mut second = options(Kind::Pota, Level::Intermediate, 1, 12);
    second.started = "2026-11-07T13:00:00Z".to_string();
    let keys = operator(
        &second,
        &fixed_speeds(&second),
        Style::Mixed,
        Quits::default(),
    );
    golden("second", tmp.path(), &second, keys, Faults::default());
}
