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

//! `CopySession` as a consumer drives it: steps, outcomes, records.
//!
//! Nothing here plays audio: an over's PCM is rendered into a buffer and
//! compared, never sent to a device.

use std::fs;
use std::path::Path;
use std::time::Duration;

use cw_trainer_core::cat_morse::{timing, Callsign, Fist};
use cw_trainer_core::copy::{
    you_line_hold, CopySession, OverAudio, OverOutcome, Shown, Step, ThemOver, MIN_SAMPLE_RATE_HZ,
};
use cw_trainer_core::options::{CopyOptions, Farnsworth, Kind, Level};
use cw_trainer_core::plan::{group_overs, plan_qso, Over};
use cw_trainer_core::record::SessionRecord;
use cw_trainer_core::speed::{SpeedSetting, SpeedState};
use cw_trainer_core::CoreError;

const RATE: u32 = 8000;

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
        adapt: false,
        started: "2026-11-07T12:00:00Z".to_string(),
        audio_out: "Test speaker".to_string(),
    }
}

fn session(opts: &CopyOptions) -> CopySession {
    CopySession::new(opts.clone(), RATE).unwrap()
}

fn render(audio: &OverAudio) -> Vec<f32> {
    let mut r = audio.renderer().unwrap();
    let mut out = Vec::new();
    let mut buf = [0.0f32; 1000];
    loop {
        let n = r.fill(&mut buf);
        if n == 0 {
            return out;
        }
        out.extend_from_slice(&buf[..n]);
    }
}

/// What a perfect operator types: the text, or a pileup in reverse order.
fn perfect(over: &ThemOver) -> String {
    if over.several {
        let mut w = over.expected.clone();
        w.reverse();
        w.join(" ")
    } else {
        over.expected.join(" ")
    }
}

fn copied(text: String) -> OverOutcome {
    OverOutcome::Copied {
        text,
        agn: 0,
        audio_gaps: 0,
        clipped: 0,
    }
}

/// Every step to the end, copying each over perfectly.
fn drive_perfectly(s: &mut CopySession) -> Vec<Step> {
    let mut steps = Vec::new();
    while let Some(step) = s.next_step().unwrap() {
        if let Step::ThemOver(o) = &step {
            s.submit(copied(perfect(o))).unwrap();
        }
        steps.push(step);
    }
    steps
}

fn fixture_record(name: &str) -> SessionRecord {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/records")
        .join(format!("{name}.record.json"));
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

// --- the steps ------------------------------------------------------------

#[test]
fn a_ragchew_steps_through_each_qsos_overs_in_order() {
    let opts = options(Kind::RagChew, Level::Intermediate, 2, 7);
    let speed = SpeedState::new(20, Farnsworth::Off, Level::Intermediate, false).current;
    let mut s = session(&opts);
    let steps = drive_perfectly(&mut s);
    let mut i = 0;
    for q in 0..2u32 {
        let plan = plan_qso(&opts, q, speed).unwrap();
        match &steps[i] {
            Step::QsoStart { qso, of, label } => {
                assert_eq!((*qso, *of, *label), (q + 1, 2, plan.label));
            }
            other => panic!("expected QsoStart, got {other:?}"),
        }
        i += 1;
        for (n, over) in group_overs(&plan.script).iter().enumerate() {
            match (over, &steps[i]) {
                (Over::You(t), Step::YouLine { text, hold }) => {
                    assert_eq!(text, &t.text);
                    assert_eq!(*hold, you_line_hold(text, speed));
                }
                (Over::Them { lines, .. }, Step::ThemOver(o)) => {
                    assert_eq!((o.qso, o.over), (q + 1, n as u32 + 1));
                    assert_eq!(o.expected, vec![lines[0].text.clone()]);
                    assert!(!o.several);
                    assert_eq!(o.audio.sample_rate_hz(), RATE);
                }
                (o, st) => panic!("over {o:?} became {st:?}"),
            }
            i += 1;
        }
        assert!(matches!(steps[i], Step::QsoEnd { speed_change: None }));
        i += 1;
    }
    assert_eq!(i, steps.len());
    assert_eq!(s.next_step().unwrap(), None, "None stays None");
    let f = s.finish(None);
    assert!(!f.record.quit_early);
    assert_eq!(f.record.total.cer(), 0.0);
}

#[test]
fn a_you_line_is_held_for_as_long_as_it_takes_to_send() {
    let speed = SpeedSetting {
        char_wpm: 20,
        effective_wpm: None,
    };
    let text = "CQ CQ DE KF0UWV K";
    let expect = timing::key_text(text, speed.speed(), &Fist::perfect(), 0)
        .unwrap()
        .total();
    assert_eq!(you_line_hold(text, speed), expect);
    // Text that cannot be keyed is held for two seconds, not an error.
    assert_eq!(you_line_hold("<NOPE", speed), Duration::from_secs(2));
}

#[test]
fn a_pileup_is_one_over_scored_in_any_order() {
    let opts = options(Kind::Pileup, Level::Advanced, 1, 11);
    let mut s = session(&opts);
    let mut saw_several = false;
    while let Some(step) = s.next_step().unwrap() {
        if let Step::ThemOver(o) = step {
            let report = s.submit(copied(perfect(&o))).unwrap();
            assert_eq!(report.result.score.cer(), 0.0);
            match report.shown.unwrap() {
                Shown::Unordered(u) => {
                    assert!(o.several);
                    saw_several = true;
                    assert_eq!(u.score(), report.result.score);
                }
                Shown::Ordered(a) => {
                    assert!(!o.several);
                    assert_eq!(a.score(), report.result.score);
                }
            }
        }
    }
    assert!(saw_several);
}

#[test]
fn agn_renders_the_identical_audio_again() {
    let opts = options(Kind::RagChew, Level::Advanced, 1, 7);
    let mut s = session(&opts);
    loop {
        if let Step::ThemOver(o) = s.next_step().unwrap().unwrap() {
            let once = render(&o.audio);
            assert!(!once.is_empty());
            assert_eq!(once, render(&o.audio));
            assert_eq!(once.len(), o.audio.channel().len_samples());
            break;
        }
    }
}

#[test]
fn a_skip_is_recorded_unscored_and_shows_nothing() {
    let opts = options(Kind::RagChew, Level::Intermediate, 1, 7);
    let mut s = session(&opts);
    let first = loop {
        if let Step::ThemOver(o) = s.next_step().unwrap().unwrap() {
            break o;
        }
    };
    let report = s
        .submit(OverOutcome::Skipped {
            agn: 1,
            audio_gaps: 2,
            clipped: 0,
        })
        .unwrap();
    assert!(report.shown.is_none());
    assert!(report.result.skipped);
    assert_eq!((report.result.agn, report.result.audio_gaps), (1, 2));
    assert_eq!(report.result.expected, first.expected);
    assert_eq!(report.result.score.expected, 0);
    drive_perfectly(&mut s);
    let f = s.finish(None);
    assert_eq!(f.record.overs[0], report.result);
    assert_eq!(f.record.total.cer(), 0.0, "a skip costs nothing");
}

// --- quitting and misuse ---------------------------------------------------

/// Steps until `stop` says to quit, then quits; returns what follows.
fn quit_when(stop: impl Fn(&Step) -> bool) -> (Vec<Step>, Vec<Option<Step>>, SessionRecord) {
    let opts = options(Kind::RagChew, Level::Intermediate, 2, 7);
    let mut s = session(&opts);
    let mut before = Vec::new();
    loop {
        let step = s.next_step().unwrap().expect("quit before the end");
        let quit = stop(&step);
        if !quit {
            if let Step::ThemOver(o) = &step {
                s.submit(copied(perfect(o))).unwrap();
            }
        }
        before.push(step);
        if quit {
            break;
        }
    }
    s.quit();
    let after = vec![s.next_step().unwrap(), s.next_step().unwrap()];
    let record = s.finish(None).record;
    (before, after, record)
}

#[test]
fn quitting_in_a_them_over_ends_the_qso_unrecorded_then_the_session() {
    let (before, after, r) = quit_when(|s| matches!(s, Step::ThemOver(o) if o.over > 2));
    assert!(matches!(after[0], Some(Step::QsoEnd { .. })));
    assert_eq!(after[1], None);
    assert!(r.quit_early);
    let submitted = before
        .iter()
        .filter(|s| matches!(s, Step::ThemOver(_)))
        .count();
    assert_eq!(
        r.overs.len(),
        submitted - 1,
        "the quit over is not recorded"
    );
}

#[test]
fn quitting_at_a_you_line_or_a_qso_start_ends_the_qso_then_the_session() {
    for stop in [
        (|s: &Step| matches!(s, Step::YouLine { .. })) as fn(&Step) -> bool,
        |s: &Step| matches!(s, Step::QsoStart { qso: 2, .. }),
    ] {
        let (_, after, r) = quit_when(stop);
        assert!(matches!(after[0], Some(Step::QsoEnd { .. })));
        assert_eq!(after[1], None);
        assert!(r.quit_early);
    }
}

#[test]
fn quitting_between_qsos_or_before_the_first_ends_at_once() {
    let (_, after, r) = quit_when(|s| matches!(s, Step::QsoEnd { .. }));
    assert_eq!(after, vec![None, None], "no QSO is open to end");
    assert!(r.quit_early);

    let mut s = session(&options(Kind::RagChew, Level::Intermediate, 2, 7));
    s.quit();
    assert_eq!(s.next_step().unwrap(), None);
    let r = s.finish(None).record;
    assert!(r.quit_early && r.overs.is_empty());
}

#[test]
fn submitting_with_no_over_waiting_is_an_error_that_changes_nothing() {
    let mut s = session(&options(Kind::RagChew, Level::Intermediate, 1, 7));
    assert_eq!(
        s.submit(copied("CQ".into())).unwrap_err(),
        CoreError::NoOverPending
    );
    assert!(matches!(
        s.next_step().unwrap(),
        Some(Step::QsoStart { .. })
    ));
    assert_eq!(
        s.submit(copied("CQ".into())).unwrap_err(),
        CoreError::NoOverPending
    );
    drive_perfectly(&mut s);
    assert!(!s.finish(None).record.quit_early);
}

#[test]
fn stepping_past_a_waiting_over_is_an_error_that_changes_nothing() {
    let mut s = session(&options(Kind::RagChew, Level::Intermediate, 1, 7));
    let o = loop {
        if let Step::ThemOver(o) = s.next_step().unwrap().unwrap() {
            break o;
        }
    };
    assert_eq!(s.next_step().unwrap_err(), CoreError::OverPending);
    assert_eq!(s.next_step().unwrap_err(), CoreError::OverPending);
    // The over is still there to submit, and the session goes on.
    assert_eq!(
        s.submit(copied(perfect(&o))).unwrap().result.score.cer(),
        0.0
    );
    drive_perfectly(&mut s);
    let r = s.finish(None).record;
    assert!(!r.quit_early);
    assert_eq!(r.total.cer(), 0.0);
}

// --- finishing ---------------------------------------------------------------

#[test]
fn finishing_early_or_with_an_error_is_quitting_early() {
    let opts = options(Kind::RagChew, Level::Intermediate, 1, 7);
    let mut s = session(&opts);
    s.next_step().unwrap();
    assert!(s.finish(None).record.quit_early, "before the session's end");

    let mut s = session(&opts);
    drive_perfectly(&mut s);
    let f = s.finish(Some("audio: device unplugged".to_string()));
    assert!(f.record.quit_early);
    assert_eq!(
        f.record.ended_by_error.as_deref(),
        Some("audio: device unplugged")
    );
}

#[test]
fn the_record_carries_the_options_and_the_session_stats() {
    let mut opts = options(Kind::Contest, Level::Advanced, 1, 5);
    opts.pitch_hz = 650.0;
    let mut s = session(&opts);
    drive_perfectly(&mut s);
    let stats = s.session_stats().clone();
    let f = s.finish(None);
    let r = &f.record;
    assert_eq!(r.version, 1);
    assert_eq!(
        (r.seed, r.kind.as_str(), r.level.as_str(), r.call.as_str()),
        (5, "contest", "advanced", "KF0UWV")
    );
    assert_eq!((r.pitch_hz, r.audio_out.as_str()), (650.0, "Test speaker"));
    assert_eq!(r.started, "2026-11-07T12:00:00Z");
    assert_eq!(f.session_stats, stats);
    assert!(!stats.weakest(1).is_empty());
}

// --- speed, gain, sample rate ------------------------------------------------

#[test]
fn a_speed_change_shows_at_qso_end_and_applies_from_the_next_qso() {
    let mut opts = options(Kind::RagChew, Level::Intermediate, 2, 7);
    opts.adapt = true;
    let mut s = session(&opts);
    let start = s.speed();
    let steps = drive_perfectly(&mut s);
    let end = steps
        .iter()
        .position(|st| matches!(st, Step::QsoEnd { .. }))
        .unwrap();
    let Step::QsoEnd {
        speed_change: Some(changed),
    } = steps[end]
    else {
        panic!("seed 7 raises the speed in QSO 1: {:?}", steps[end]);
    };
    assert!(changed.char_wpm > start.char_wpm);
    // Every you line of QSO 1 is held at the speed it started at, even
    // after the change; QSO 2's are held at the new speed.
    for (i, st) in steps.iter().enumerate() {
        if let Step::YouLine { text, hold } = st {
            let at = if i < end { start } else { changed };
            assert_eq!(*hold, you_line_hold(text, at), "step {i}");
        }
    }
    let r = s.finish(None).record;
    assert_eq!(r.start_speed, start);
    assert!(r.end_speed.char_wpm >= changed.char_wpm);
}

/// The gain of each them over when the outcomes report `clipped`.
fn gains(clipped: &[u64], skip: bool) -> Vec<f32> {
    // The gain is the session's, across QSOs.
    let mut s = session(&options(Kind::RagChew, Level::Intermediate, 2, 7));
    let mut out = Vec::new();
    let mut n = 0;
    while let Some(step) = s.next_step().unwrap() {
        if let Step::ThemOver(o) = step {
            out.push(o.audio.channel().gain);
            let c = clipped.get(n).copied().unwrap_or(0);
            let outcome = if skip {
                OverOutcome::Skipped {
                    agn: 0,
                    audio_gaps: 0,
                    clipped: c,
                }
            } else {
                OverOutcome::Copied {
                    text: perfect(&o),
                    agn: 0,
                    audio_gaps: 0,
                    clipped: c,
                }
            };
            s.submit(outcome).unwrap();
            n += 1;
        }
    }
    out
}

#[test]
fn a_clipped_over_turns_the_next_ones_down() {
    for skip in [false, true] {
        let g = gains(&[0, 3, 0, 1], skip);
        assert!(g.len() >= 5, "{g:?}");
        assert_eq!(g[..5], [1.0, 1.0, 0.8, 0.8, 0.8 * 0.8], "skip {skip}");
    }
}

#[test]
fn a_new_sample_rate_applies_from_the_next_over_and_not_to_the_record() {
    let opts = options(Kind::RagChew, Level::Intermediate, 1, 7);
    let mut a = session(&opts);
    let ra = {
        drive_perfectly(&mut a);
        a.finish(None).record
    };
    let mut b = session(&opts);
    let mut rates = Vec::new();
    while let Some(step) = b.next_step().unwrap() {
        if let Step::ThemOver(o) = step {
            let rate = o.audio.sample_rate_hz();
            rates.push(rate);
            b.set_sample_rate(48_000).unwrap();
            assert_eq!(o.audio.sample_rate_hz(), rate, "this over is unchanged");
            b.submit(copied(perfect(&o))).unwrap();
        }
    }
    assert_eq!(rates[0], RATE);
    assert!(rates[1..].iter().all(|r| *r == 48_000), "{rates:?}");
    assert_eq!(b.finish(None).record, ra);
}

// --- validation ----------------------------------------------------------------

#[test]
fn a_session_refuses_a_rate_too_low_to_play_its_pitch() {
    let opts = options(Kind::RagChew, Level::Intermediate, 1, 7);
    for rate in [0, 1, MIN_SAMPLE_RATE_HZ - 1] {
        assert_eq!(
            CopySession::new(opts.clone(), rate).err(),
            Some(CoreError::SampleRate(rate))
        );
    }
    assert!(CopySession::new(opts.clone(), MIN_SAMPLE_RATE_HZ).is_ok());
    let mut s = session(&opts);
    assert_eq!(s.set_sample_rate(0), Err(CoreError::SampleRate(0)));
}

#[test]
fn a_session_refuses_options_out_of_range() {
    let base = options(Kind::RagChew, Level::Intermediate, 1, 7);
    let mut bad = Vec::new();
    for pitch in [f32::NAN, 299.0, 1201.0, f32::INFINITY] {
        let mut o = base.clone();
        o.pitch_hz = pitch;
        bad.push(o);
    }
    for w in [0, 4, 61] {
        let mut o = base.clone();
        o.farnsworth = Farnsworth::Wpm(w);
        bad.push(o);
    }
    for o in bad {
        assert!(
            matches!(
                CopySession::new(o.clone(), RATE),
                Err(CoreError::InvalidOptions(_))
            ),
            "{o:?}"
        );
    }
}

// --- determinism and the CLI's own records -----------------------------------

#[test]
fn the_same_seed_and_outcomes_give_the_same_steps_and_record() {
    let opts = options(Kind::Mixed, Level::Intermediate, 3, 99);
    let mut a = session(&opts);
    let mut b = session(&opts);
    assert_eq!(drive_perfectly(&mut a), drive_perfectly(&mut b));
    assert_eq!(a.finish(None), b.finish(None));
}

/// The CLI's scripted operator (its golden tests), as outcomes: a cycle of
/// perfect, a wrong last character, AGN, skip, last three dropped, an
/// extra word; you lines continue at once.
fn cli_operator(o: &ThemOver, n: usize, cycle: bool) -> OverOutcome {
    let text = perfect(o);
    let copy = |text: String, agn| OverOutcome::Copied {
        text,
        agn,
        audio_gaps: 0,
        clipped: 0,
    };
    if !cycle {
        return copy(text, 0);
    }
    match n % 6 {
        0 => copy(text, 0),
        1 => {
            let mut c: Vec<char> = text.chars().collect();
            let last = c.len() - 1;
            c[last] = if c[last] == 'E' { 'T' } else { 'E' };
            copy(c.into_iter().collect(), 0)
        }
        2 => copy(text, 1),
        3 => OverOutcome::Skipped {
            agn: 0,
            audio_gaps: 0,
            clipped: 0,
        },
        4 => {
            let keep = text.chars().count().saturating_sub(3);
            copy(text.chars().take(keep).collect(), 0)
        }
        _ => copy(format!("{text} QRZ"), 0),
    }
}

/// Drive like the CLI's golden scenario; `quit_at` is `(qso, them over)`
/// or `(qso, you line)` as there, from 0.
fn as_the_cli(
    opts: &CopyOptions,
    cycle: bool,
    quit_them: Option<(u32, usize)>,
    quit_you: Option<(u32, usize)>,
) -> SessionRecord {
    let mut s = session(opts);
    let (mut q, mut them, mut you, mut n) = (0, 0, 0, 0);
    while let Some(step) = s.next_step().unwrap() {
        match step {
            Step::QsoStart { qso, .. } => {
                (q, them, you) = (qso - 1, 0, 0);
            }
            Step::YouLine { .. } => {
                if quit_you == Some((q, you)) {
                    s.quit();
                }
                you += 1;
            }
            Step::ThemOver(o) => {
                if quit_them == Some((q, them)) {
                    s.quit();
                    continue;
                }
                s.submit(cli_operator(&o, n, cycle)).unwrap();
                them += 1;
                n += 1;
            }
            Step::QsoEnd { .. } => {}
        }
    }
    s.finish(None).record
}

#[test]
fn driven_as_the_cli_drives_it_the_record_is_the_clis_record() {
    let ragchew = options(Kind::RagChew, Level::Intermediate, 2, 7);
    assert_eq!(
        as_the_cli(&ragchew, true, None, None),
        fixture_record("ragchew")
    );
    let pileup = options(Kind::Pileup, Level::Advanced, 2, 11);
    assert_eq!(
        as_the_cli(&pileup, true, None, None),
        fixture_record("pileup")
    );
    let contest = options(Kind::Contest, Level::Advanced, 2, 5);
    assert_eq!(
        as_the_cli(&contest, true, None, None),
        fixture_record("contest")
    );
    let mixed = options(Kind::Mixed, Level::Intermediate, 3, 99);
    assert_eq!(
        as_the_cli(&mixed, true, Some((1, 2)), None),
        fixture_record("mixed-quit")
    );
    let you = options(Kind::RagChew, Level::Beginner, 2, 8);
    assert_eq!(
        as_the_cli(&you, false, None, Some((0, 1))),
        fixture_record("you-quit")
    );
    let mut adapt = options(Kind::RagChew, Level::Intermediate, 2, 7);
    adapt.adapt = true;
    assert_eq!(
        as_the_cli(&adapt, false, None, None),
        fixture_record("adapt")
    );
}

// --- threads -------------------------------------------------------------------

#[test]
fn a_session_and_its_audio_can_move_to_another_thread() {
    fn send<T: Send>() {}
    send::<CopySession>();
    send::<OverAudio>();
    send::<Step>();
}
