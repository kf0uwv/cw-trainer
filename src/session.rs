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

//! The copy session: QSO after QSO, over after over.
//!
//! For each QSO a script is generated (cat-morse) at the current speed.
//! Its transmissions are grouped into *overs*: a *you* line is shown as a
//! prompt and held for as long as it would take to send; a *them* over —
//! one station, or several calling at once in a pileup — is synthesized
//! and played while the operator types. Enter submits; the copy is scored
//! per character and shown aligned against what was sent.
//!
//! # Adaptive speed
//!
//! Over a window of the last [`WINDOW`] scored overs (skips excluded):
//! a CER of at most [`RAISE_CER`] with no AGN raises the speed by
//! [`STEP_WPM`]; a CER of at least [`LOWER_CER`] lowers it. With
//! Farnsworth spacing on, the *effective* speed moves first, until it
//! meets the character speed and Farnsworth falls away. A change takes
//! effect at the next QSO: a station does not change speed mid-contact.

use std::io;
use std::time::Duration;

use cat_morse::{align, align_unordered, timing, Channel, CharStats, Fist, Score, ScoreOptions};
use serde::{Deserialize, Serialize};

use super::audio::{over_channel, AudioSink};
use super::store::Store;
use super::term::{Key, Terminal};
use super::view::{self, NL};
use super::CopyOptions;
pub use cw_trainer_core::plan::{
    expected_for, group_overs, plan_qso, scenario_kind, Over, QsoPlan,
};
pub use cw_trainer_core::speed::{
    difficulty_for, SpeedSetting, SpeedState, LOWER_CER, RAISE_CER, STEP_WPM, WINDOW,
};
use cw_trainer_core::CoreError;

/// How much audio to keep queued ahead of the speaker.
const QUEUE_AHEAD: Duration = Duration::from_millis(150);
/// One rendering block.
const BLOCK: Duration = Duration::from_millis(20);

/// Everything that can end a session early.
#[derive(Debug, thiserror::Error)]
pub enum CwError {
    #[error("terminal: {0}")]
    Terminal(#[from] io::Error),
    #[error("audio: {0}")]
    Audio(String),
    #[error(transparent)]
    Core(#[from] CoreError),
}

/// One over's result, as kept in the history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverResult {
    pub qso: u32,
    pub kind: String,
    /// The text sent, or each call for an over several stations sent.
    pub expected: Vec<String>,
    pub copied: String,
    pub score: Score,
    pub agn: u32,
    pub skipped: bool,
    /// Times the speaker ran dry mid-over.
    pub audio_gaps: u32,
    pub char_wpm: f64,
    pub effective_wpm: f64,
}

/// A whole session, as kept in the history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub version: u32,
    pub started: String,
    pub seed: u64,
    pub kind: String,
    pub level: String,
    pub call: String,
    pub pitch_hz: f32,
    /// The output device played to.
    #[serde(default)]
    pub audio_out: String,
    pub start_speed: SpeedSetting,
    pub end_speed: SpeedSetting,
    pub overs: Vec<OverResult>,
    /// Totals over every scored (not skipped) over.
    pub total: Score,
    pub quit_early: bool,
    /// Set when the session was cut short by an error (the error's text);
    /// everything scored before it is still here.
    #[serde(default)]
    pub ended_by_error: Option<String>,
}

enum Action {
    Submitted(String),
    Skipped,
    Quit,
}

struct Copied {
    action: Action,
    agn: u32,
    gaps: u32,
    clipped: u64,
}

fn put<T: Terminal + ?Sized>(term: &mut T, text: &str) -> io::Result<()> {
    let out = term.out();
    out.write_all(text.as_bytes())?;
    out.flush()
}

/// Play one over and collect the copy typed while it plays.
fn copy_over<S: AudioSink, T: Terminal>(
    channel: &Channel,
    sink: &mut S,
    term: &mut T,
) -> Result<Copied, CwError> {
    let rate = sink.sample_rate_hz();
    let block = ((rate as u128 * BLOCK.as_millis()) / 1000).max(1) as usize;
    let ahead = ((rate as u128 * QUEUE_AHEAD.as_millis()) / 1000).max(1) as usize;
    let mut typed = String::new();
    let mut agn = 0;
    let mut gaps = 0;
    let mut clipped = 0;
    put(term, "  copy> ")?;

    'play: loop {
        sink.clear();
        let mut renderer = channel
            .renderer()
            .map_err(|e| CwError::Core(CoreError::Render(e)))?;
        let mut buf = vec![0.0f32; block];
        let mut pending: Vec<f32> = Vec::new();
        let mut feeding = true;
        let mut underruns = sink.underruns();

        loop {
            let was_feeding = feeding;
            while feeding && sink.queued() < ahead {
                if pending.is_empty() {
                    let n = renderer.fill(&mut buf);
                    if n == 0 {
                        feeding = false;
                        clipped = renderer.clipped();
                        break;
                    }
                    pending.extend_from_slice(&buf[..n]);
                }
                let took = sink.write(&pending).map_err(CwError::Audio)?;
                pending.drain(..took);
                if took == 0 {
                    break;
                }
            }
            // Only a shortfall while still feeding is a gap the operator
            // heard; the speaker also counts the natural end of the audio,
            // which comes a queue-length after the last write -- so judge
            // by whether this iteration began while still feeding.
            let now = sink.underruns();
            if was_feeding && now > underruns {
                gaps += (now - underruns) as u32;
            }
            underruns = now;

            let wait = if feeding || sink.queued() > 0 {
                Duration::from_millis(10)
            } else {
                Duration::from_millis(100)
            };
            match term.poll_key(wait)? {
                None => {}
                Some(Key::Char(c)) => {
                    typed.push(c);
                    put(term, &c.to_string())?;
                }
                Some(Key::Backspace) => {
                    if typed.pop().is_some() {
                        put(term, "\x08 \x08")?;
                    }
                }
                Some(Key::Enter) => {
                    sink.clear();
                    put(term, NL)?;
                    return Ok(Copied {
                        action: Action::Submitted(typed),
                        agn,
                        gaps,
                        clipped,
                    });
                }
                Some(Key::Agn) => {
                    agn += 1;
                    put(term, &format!("  [AGN]{NL}  copy> {typed}"))?;
                    continue 'play;
                }
                Some(Key::Skip) => {
                    sink.clear();
                    put(term, &format!("  [skipped]{NL}"))?;
                    return Ok(Copied {
                        action: Action::Skipped,
                        agn,
                        gaps,
                        clipped,
                    });
                }
                Some(Key::Quit) => {
                    sink.clear();
                    put(term, NL)?;
                    return Ok(Copied {
                        action: Action::Quit,
                        agn,
                        gaps,
                        clipped,
                    });
                }
            }
        }
    }
}

/// Show the operator's own line for as long as it takes to send at
/// `speed` (the speed the QSO started at). Returns `true` if the operator
/// quit.
///
/// Anything typed meanwhile is **discarded**, deliberately: the operator
/// is sending, not copying, and a keystroke here is not copy of the next
/// over, which has not been heard yet. Enter skips the wait.
fn you_line<T: Terminal>(text: &str, speed: SpeedSetting, term: &mut T) -> Result<bool, CwError> {
    put(term, &format!("  you send> {text}{NL}"))?;
    let hold = timing::key_text(text, speed.speed(), &Fist::perfect(), 0)
        .map(|k| k.total())
        .unwrap_or(Duration::from_secs(2));
    let deadline = term.now() + hold;
    loop {
        let now = term.now();
        if now >= deadline {
            return Ok(false);
        }
        match term.poll_key((deadline - now).min(Duration::from_millis(100)))? {
            Some(Key::Enter) => return Ok(false),
            Some(Key::Quit) => return Ok(true),
            _ => {}
        }
    }
}

/// The session so far: what survives an error.
struct Progress {
    speed: SpeedState,
    session_stats: CharStats,
    overs: Vec<OverResult>,
    total: Score,
    quit: bool,
    gain: f32,
}

/// Run a copy session to the end (or until the operator quits).
///
/// If anything fails part-way — the speaker unplugged, the terminal gone —
/// what was copied is not lost: the stats are saved, the session goes into
/// the history marked `ended_by_error`, the summary is shown if it can be,
/// and then the error is returned.
pub fn run_copy<S: AudioSink, T: Terminal>(
    opts: &CopyOptions,
    sink: &mut S,
    term: &mut T,
    store: &mut Store,
) -> Result<SessionRecord, CwError> {
    let mut p = Progress {
        speed: SpeedState::new(opts.wpm, opts.farnsworth, opts.level, opts.adapt),
        session_stats: CharStats::new(),
        overs: Vec::new(),
        total: Score::default(),
        quit: false,
        gain: 1.0,
    };
    let start_speed = p.speed.current;
    let outcome = run_qsos(opts, sink, term, store, &mut p);
    if outcome.is_err() {
        sink.clear();
    }

    let record = SessionRecord {
        version: 1,
        started: opts.started.clone(),
        seed: opts.seed,
        kind: opts.kind.name().to_string(),
        level: opts.level.name().to_string(),
        call: opts.call.as_str().to_string(),
        pitch_hz: opts.pitch_hz,
        audio_out: opts.audio_out.clone(),
        start_speed,
        end_speed: p.speed.current,
        overs: p.overs,
        total: p.total,
        quit_early: p.quit || outcome.is_err(),
        ended_by_error: outcome.as_ref().err().map(|e| e.to_string()),
    };
    // Saved before anything is printed, so a dead terminal cannot lose it.
    store.save_stats(&p.session_stats);
    store.append_history(&record);

    let mut shown = put(
        term,
        &view::summary(&record, &p.session_stats, &store.all_time(&p.session_stats)),
    );
    for w in store.take_warnings() {
        shown = shown.and_then(|_| put(term, &format!("warning: {w}{NL}")));
    }
    outcome?;
    shown?;
    Ok(record)
}

fn run_qsos<S: AudioSink, T: Terminal>(
    opts: &CopyOptions,
    sink: &mut S,
    term: &mut T,
    store: &mut Store,
    p: &mut Progress,
) -> Result<(), CwError> {
    let score_opts = ScoreOptions::default();
    put(
        term,
        &format!(
            "cw-trainer copy: {} contacts, {}, seed {}, {:.0} Hz, {}{NL}\
             Playing to: {}{NL}\
             Type what you hear while it plays; Enter submits.{NL}\
             Ctrl-R/F5 AGN (replay)   Ctrl-N/F6 skip   Esc/Ctrl-C quit{NL}",
            opts.kind.name(),
            opts.level.name(),
            opts.seed,
            opts.pitch_hz,
            p.speed.current.describe(),
            opts.audio_out
        ),
    )?;
    for w in store.take_warnings() {
        put(term, &format!("warning: {w}{NL}"))?;
    }
    if let Some(last) = store.read_history().last() {
        put(term, &view::last_session(last))?;
    }

    for q in 0..opts.qsos {
        // The whole QSO, "you" lines included, runs at the speed it
        // started at; an adaptation applies from the next one.
        let qso_speed = p.speed.current;
        let plan = plan_qso(opts, q, qso_speed)?;
        put(
            term,
            &format!(
                "{NL}--- QSO {}/{}: {} ---{NL}",
                q + 1,
                opts.qsos,
                plan.label
            ),
        )?;
        let mut change = None;

        for (i, over) in group_overs(&plan.script).iter().enumerate() {
            match over {
                Over::You(t) => {
                    if you_line(&t.text, qso_speed, term)? {
                        p.quit = true;
                    }
                }
                Over::Them { lines, starts } => {
                    let seed = plan.rng.fork(1000 + i as u64).next_u64();
                    let mut channel = over_channel(
                        &plan.script,
                        lines,
                        starts,
                        &plan.difficulty,
                        sink.sample_rate_hz(),
                        opts.pitch_hz,
                        seed,
                    )?;
                    channel.gain = p.gain;
                    let sig = plan
                        .script
                        .signal_for(lines[0])
                        .map(|s| s.speed)
                        .unwrap_or_else(|| qso_speed.speed());
                    let several = lines.len() > 1;
                    put(
                        term,
                        &format!(
                            "{NL}  [QSO {} over {}, {:.0} wpm]{}{NL}",
                            q + 1,
                            i + 1,
                            sig.char_wpm(),
                            if several {
                                " several stations: copy everything you hear, any order"
                            } else {
                                ""
                            }
                        ),
                    )?;
                    let copied = copy_over(&channel, sink, term)?;
                    if copied.clipped > 0 {
                        p.gain *= 0.8;
                    }
                    let expected = expected_for(lines);
                    let mut result = OverResult {
                        qso: q + 1,
                        kind: plan.label.to_string(),
                        expected: expected.clone(),
                        copied: String::new(),
                        score: Score::default(),
                        agn: copied.agn,
                        skipped: false,
                        audio_gaps: copied.gaps,
                        char_wpm: sig.char_wpm(),
                        effective_wpm: sig.effective_wpm(),
                    };
                    match copied.action {
                        Action::Quit => {
                            p.quit = true;
                        }
                        Action::Skipped => {
                            result.skipped = true;
                            p.overs.push(result);
                        }
                        Action::Submitted(text) => {
                            let (score, shown) = if several {
                                let refs: Vec<&str> = expected.iter().map(String::as_str).collect();
                                let u = align_unordered(&refs, &text, &score_opts)
                                    .map_err(|e| CwError::Core(CoreError::Align(e)))?;
                                p.session_stats.record_unordered(&u);
                                (u.score(), view::unordered_result(&u, copied.agn))
                            } else {
                                let a = align(&expected[0], &text, &score_opts)
                                    .map_err(|e| CwError::Core(CoreError::Align(e)))?;
                                p.session_stats.record(&a);
                                (a.score(), view::over_result(&a, copied.agn))
                            };
                            // Recorded before it is shown: a failed print
                            // must not lose a scored over.
                            p.total.add(&score);
                            result.copied = text;
                            result.score = score;
                            p.overs.push(result);
                            if let Some(c) = p.speed.record(&score, copied.agn) {
                                change = Some(c);
                            }
                            put(term, &shown)?;
                            if copied.gaps > 0 {
                                put(
                                    term,
                                    &format!(
                                        "  (the audio dropped out {} time(s) during this over){NL}",
                                        copied.gaps
                                    ),
                                )?;
                            }
                        }
                    }
                }
            }
            if p.quit {
                break;
            }
        }

        store.save_stats(&p.session_stats);
        if let Some(c) = change {
            put(term, &format!("  speed now {}{NL}", c.describe()))?;
        }
        if p.quit {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
pub mod tests {
    use cat_morse::Callsign;

    use super::*;
    use crate::audio::tests::FakeSink;
    use crate::term::tests::ScriptedTerminal;
    use crate::{Farnsworth, Kind, Level};

    pub fn options(kind: Kind, qsos: u32, seed: u64) -> CopyOptions {
        CopyOptions {
            kind,
            level: Level::Intermediate,
            qsos,
            call: Callsign::parse("KF0UWV").unwrap(),
            wpm: 20,
            farnsworth: Farnsworth::Off,
            pitch_hz: 600.0,
            seed,
            adapt: false,
            started: "2026-10-06T00:00:00Z".to_string(),
            audio_out: "Test speaker".to_string(),
        }
    }

    pub fn sample_record() -> SessionRecord {
        SessionRecord {
            version: 1,
            started: "2026-10-06T00:00:00Z".to_string(),
            seed: 1,
            kind: "ragchew".to_string(),
            level: "beginner".to_string(),
            call: "KF0UWV".to_string(),
            pitch_hz: 600.0,
            audio_out: "Test speaker".to_string(),
            start_speed: SpeedSetting {
                char_wpm: 20,
                effective_wpm: Some(10),
            },
            end_speed: SpeedSetting {
                char_wpm: 22,
                effective_wpm: None,
            },
            overs: vec![OverResult {
                qso: 1,
                kind: "ragchew".to_string(),
                expected: vec!["CQ DE W1AW".to_string()],
                copied: "CQ DE W1AQ".to_string(),
                score: align("CQ DE W1AW", "CQ DE W1AQ", &ScoreOptions::default())
                    .unwrap()
                    .score(),
                agn: 1,
                skipped: false,
                audio_gaps: 0,
                char_wpm: 20.0,
                effective_wpm: 20.0,
            }],
            total: Score::default(),
            quit_early: false,
            ended_by_error: None,
        }
    }

    /// The keys a perfect operator presses for QSO `qso` (adapt off).
    fn perfect_keys(opts: &CopyOptions, qso: u32) -> Vec<(Duration, Key)> {
        let speed = SpeedState::new(opts.wpm, opts.farnsworth, opts.level, false).current;
        let plan = plan_qso(opts, qso, speed).unwrap();
        let mut keys = Vec::new();
        for over in group_overs(&plan.script) {
            match over {
                Over::You(_) => keys.push((Duration::ZERO, Key::Enter)),
                Over::Them { lines, .. } => {
                    let text = expected_for(&lines).join(" ");
                    keys.extend(ScriptedTerminal::typed(&[&text]));
                }
            }
        }
        keys
    }

    fn run(opts: &CopyOptions, keys: Vec<(Duration, Key)>) -> (SessionRecord, FakeSink, String) {
        let mut sink = FakeSink::new(8000);
        let mut term = ScriptedTerminal::new(keys);
        let mut store = Store::open(None);
        let r = run_copy(opts, &mut sink, &mut term, &mut store).unwrap();
        (r, sink, term.text())
    }

    // --- grouping -------------------------------------------------------

    #[test]
    fn the_banner_names_cw_trainer_not_ts570d() {
        let opts = options(Kind::RagChew, 1, 7);
        let (_, _, text) = run(&opts, perfect_keys(&opts, 1));
        assert!(text.starts_with("cw-trainer copy: ragchew"), "{text}");
        assert!(!text.contains("ts570d"), "{text}");
    }

    #[test]
    fn a_library_error_keeps_its_exact_text() {
        // History files store this text; the CLI adds nothing to it.
        use cat_morse::{CodecError, ScenarioError};
        for e in [
            CoreError::NoStation,
            CoreError::Generate(ScenarioError::InvalidSetting("x")),
            CoreError::Align(CodecError::UnterminatedProsign { offset: 1 }),
        ] {
            assert_eq!(CwError::from(e.clone()).to_string(), e.to_string());
        }
    }

    // --- whole sessions -------------------------------------------------

    #[test]
    fn a_perfect_operator_scores_zero_cer() {
        let opts = options(Kind::RagChew, 1, 7);
        let (r, sink, out) = run(&opts, perfect_keys(&opts, 0));
        assert!(!r.quit_early, "{out}");
        assert!(!r.overs.is_empty());
        assert!(r.overs.iter().all(|o| !o.skipped && o.agn == 0));
        assert_eq!(r.total.cer(), 0.0, "{out}");
        assert!(r.total.expected > 0);
        assert!(!sink.played.is_empty());
        assert!(out.contains("Session summary"));
    }

    #[test]
    fn copying_nothing_is_all_misses() {
        let opts = options(Kind::Contest, 1, 5);
        let keys: Vec<_> = perfect_keys(&opts, 0)
            .into_iter()
            .filter(|(_, k)| !matches!(k, Key::Char(_)))
            .collect();
        let (r, _, _) = run(&opts, keys);
        assert!(!r.quit_early);
        assert_eq!(r.total.matches, 0);
        assert_eq!(r.total.deletions, r.total.expected);
        assert_eq!(r.total.cer(), 1.0);
    }

    #[test]
    fn agn_replays_the_identical_audio_and_is_counted() {
        let opts = options(Kind::RagChew, 1, 7);
        let mut keys = perfect_keys(&opts, 0);
        let (_, once, _) = run(&opts, keys.clone());
        // AGN at the start of the first *them* over: find its first char.
        let first_char = keys
            .iter()
            .position(|(_, k)| matches!(k, Key::Char(_)))
            .unwrap();
        keys.insert(first_char, (Duration::ZERO, Key::Agn));
        let (r, twice, _) = run(&opts, keys);
        let first = r.overs.iter().find(|o| o.agn > 0).expect("an AGN");
        assert_eq!(first.agn, 1);
        assert_eq!(r.total.cer(), 0.0, "AGN does not cost characters");
        // The replay adds exactly one more copy of that over's audio.
        let extra = twice.played.len() - once.played.len();
        assert!(extra > 0);
        assert!(twice.clears > once.clears);
    }

    #[test]
    fn a_skipped_over_is_recorded_but_not_scored() {
        let opts = options(Kind::RagChew, 1, 7);
        let mut keys = perfect_keys(&opts, 0);
        // Replace the first them-over's copy with a skip.
        let first_char = keys
            .iter()
            .position(|(_, k)| matches!(k, Key::Char(_)))
            .unwrap();
        let enter = first_char
            + keys[first_char..]
                .iter()
                .position(|(_, k)| *k == Key::Enter)
                .unwrap();
        keys.splice(first_char..=enter, [(Duration::ZERO, Key::Skip)]);
        let (r, _, _) = run(&opts, keys);
        assert_eq!(r.overs.iter().filter(|o| o.skipped).count(), 1);
        assert_eq!(r.total.cer(), 0.0);
        let skipped = r.overs.iter().find(|o| o.skipped).unwrap();
        assert_eq!(skipped.score, Score::default());
    }

    #[test]
    fn enter_mid_over_scores_what_was_typed() {
        let opts = options(Kind::RagChew, 1, 7);
        let mut keys = perfect_keys(&opts, 0);
        // Drop the last three characters typed in the first them over.
        let enter = keys
            .iter()
            .enumerate()
            .find(|(i, (_, k))| *k == Key::Enter && *i > 0 && matches!(keys[i - 1].1, Key::Char(_)))
            .map(|(i, _)| i)
            .unwrap();
        keys.drain(enter - 3..enter);
        let (r, _, _) = run(&opts, keys);
        let first = r.overs.iter().find(|o| !o.skipped).unwrap();
        assert_eq!(first.score.deletions, 3);
    }

    #[test]
    fn quitting_still_yields_a_record_and_saves_it() {
        let tmp = tempfile::tempdir().unwrap();
        let opts = options(Kind::RagChew, 3, 7);
        let mut keys = perfect_keys(&opts, 0);
        keys.push((Duration::ZERO, Key::Quit));
        let mut sink = FakeSink::new(8000);
        let mut term = ScriptedTerminal::new(keys);
        let mut store = Store::open(Some(tmp.path().to_path_buf()));
        let r = run_copy(&opts, &mut sink, &mut term, &mut store).unwrap();
        assert!(r.quit_early);
        assert!(r.overs.iter().all(|o| o.qso == 1));
        assert_eq!(store.read_history(), vec![r]);
        assert!(tmp.path().join("stats.json").exists());
    }

    #[test]
    fn a_pileup_is_scored_in_any_order() {
        let opts = options(Kind::Pileup, 1, 11);
        let speed = SpeedState::new(20, Farnsworth::Off, Level::Intermediate, false).current;
        let plan = plan_qso(&opts, 0, speed).unwrap();
        let mut keys = Vec::new();
        for over in group_overs(&plan.script) {
            match over {
                Over::You(_) => keys.push((Duration::ZERO, Key::Enter)),
                Over::Them { lines, .. } => {
                    let mut calls = expected_for(&lines);
                    calls.reverse();
                    keys.extend(ScriptedTerminal::typed(&[&calls.join(" ")]));
                }
            }
        }
        let (r, _, out) = run(&opts, keys);
        assert!(r.overs.iter().any(|o| o.expected.len() > 1));
        assert_eq!(r.total.cer(), 0.0, "{out}");
        assert!(out.contains("copy everything you hear"));
    }

    #[test]
    fn a_you_line_waits_for_its_sending_time_or_enter() {
        let speed = SpeedSetting {
            char_wpm: 20,
            effective_wpm: None,
        };
        // No key: the clock runs out the hold time.
        let mut term = ScriptedTerminal::new(vec![(Duration::from_secs(600), Key::Enter)]);
        let start = term.now();
        assert!(!you_line("CQ CQ DE KF0UWV K", speed, &mut term).unwrap());
        let held = term.now() - start;
        let expect = timing::key_text("CQ CQ DE KF0UWV K", speed.speed(), &Fist::perfect(), 0)
            .unwrap()
            .total();
        assert!(held >= expect && held < expect + Duration::from_millis(200));
        // Typing is ignored; Esc quits.
        let mut term = ScriptedTerminal::new(vec![
            (Duration::ZERO, Key::Char('X')),
            (Duration::ZERO, Key::Quit),
        ]);
        assert!(you_line("TU", speed, &mut term).unwrap());
    }

    #[test]
    fn a_speaker_that_takes_little_at_a_time_still_gets_every_sample_in_order() {
        let opts = options(Kind::RagChew, 1, 7);
        let (_, whole, _) = run(&opts, perfect_keys(&opts, 0));
        let mut sink = FakeSink::new(8000);
        sink.max_write = 37;
        let mut term = ScriptedTerminal::new(perfect_keys(&opts, 0));
        let r = run_copy(&opts, &mut sink, &mut term, &mut Store::open(None)).unwrap();
        assert_eq!(r.total.cer(), 0.0);
        assert_eq!(sink.played, whole.played);
    }

    #[test]
    fn only_a_shortfall_while_feeding_counts_as_a_gap() {
        // 0 at the start; 1 during the feeding iteration (a real gap);
        // 2 afterwards (the natural end of the audio, not a gap).
        let opts = options(Kind::RagChew, 1, 7);
        let mut sink = FakeSink::new(8000);
        sink.underrun_seq.borrow_mut().extend([0, 1, 2]);
        let mut term = ScriptedTerminal::new(perfect_keys(&opts, 0));
        let r = run_copy(&opts, &mut sink, &mut term, &mut Store::open(None)).unwrap();
        assert_eq!(r.overs[0].audio_gaps, 1);
        let total: u32 = r.overs.iter().map(|o| o.audio_gaps).sum();
        assert_eq!(total, 1, "the end-of-audio tick is not a gap");
    }

    #[test]
    fn an_audio_failure_mid_session_keeps_what_was_copied() {
        let tmp = tempfile::tempdir().unwrap();
        let opts = options(Kind::RagChew, 2, 7);
        // Enough audio for about one over, then the device dies.
        let per_over = {
            let mut s = FakeSink::new(8000);
            let mut t = ScriptedTerminal::new(perfect_keys(&opts, 0));
            let r = run_copy(&opts, &mut s, &mut t, &mut Store::open(None)).unwrap();
            assert!(r.overs.len() > 2);
            s.played.len() / r.overs.len()
        };
        let mut sink = FakeSink::new(8000);
        sink.fail_after = Some(per_over + 1);
        let mut term = ScriptedTerminal::new(perfect_keys(&opts, 0));
        let mut store = Store::open(Some(tmp.path().to_path_buf()));
        let err = run_copy(&opts, &mut sink, &mut term, &mut store).unwrap_err();
        assert!(matches!(err, CwError::Audio(_)), "{err}");
        let history = store.read_history();
        assert_eq!(history.len(), 1);
        let r = &history[0];
        assert!(r.ended_by_error.as_deref().unwrap().contains("unplugged"));
        assert!(r.quit_early);
        assert!(!r.overs.is_empty(), "the overs before the failure are kept");
        assert!(tmp.path().join("stats.json").exists());
        assert!(term.text().contains("Session summary"));
    }

    #[test]
    fn a_terminal_failure_mid_session_keeps_what_was_copied() {
        let tmp = tempfile::tempdir().unwrap();
        let opts = options(Kind::RagChew, 2, 7);
        let mut keys = perfect_keys(&opts, 0);
        keys.truncate(keys.len() / 2);
        let mut term = ScriptedTerminal::new(keys);
        term.fail_when_done = true;
        let mut store = Store::open(Some(tmp.path().to_path_buf()));
        let err = run_copy(&opts, &mut FakeSink::new(8000), &mut term, &mut store).unwrap_err();
        assert!(matches!(err, CwError::Terminal(_)), "{err}");
        let r = &store.read_history()[0];
        assert!(r.ended_by_error.as_deref().unwrap().contains("went away"));
        assert!(!r.overs.is_empty());
        assert!(tmp.path().join("stats.json").exists());
    }

    #[test]
    fn same_seed_same_keys_same_session() {
        let opts = options(Kind::Mixed, 2, 99);
        let mut keys = perfect_keys(&opts, 0);
        keys.extend(perfect_keys(&opts, 1));
        let (a, sa, _) = run(&opts, keys.clone());
        let (b, sb, _) = run(&opts, keys);
        assert_eq!(a, b);
        assert_eq!(sa.played, sb.played);
    }

    #[test]
    fn adaptation_is_announced_and_carried_into_the_record() {
        let mut opts = options(Kind::RagChew, 1, 7);
        opts.adapt = true;
        let (r, _, out) = run(&opts, perfect_keys(&opts, 0));
        let scored = r.overs.iter().filter(|o| !o.skipped).count();
        assert!(
            scored >= WINDOW,
            "seed 7 should give a long enough rag-chew"
        );
        assert!(r.end_speed.char_wpm > r.start_speed.char_wpm, "{out}");
        assert!(out.contains("speed now"));
    }
}
