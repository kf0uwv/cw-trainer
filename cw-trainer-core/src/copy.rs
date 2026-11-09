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

//! A copy session, driven by its caller one step at a time.
//!
//! The session decides everything — which contact comes next, how each
//! over sounds, how a copy scores, when the speed changes, what goes in
//! the record — and the caller does everything that touches the world:
//! it shows a *you* line for its [`hold`](Step::YouLine), plays a *them*
//! over's audio to an output it chose, collects what the operator copied,
//! and stores the stats and the record.
//!
//! ```text
//! new ─▶ next_step ─▶ QsoStart ─▶ YouLine ─▶ ThemOver ─▶ submit ─▶ … ─▶ QsoEnd ─▶ … ─▶ None ─▶ finish
//!                                                quit ─▶ QsoEnd (if a QSO is open) ─▶ None
//! ```
//!
//! # Determinism
//!
//! The session reads no clock and no entropy: the caller passes the seed
//! and the start time in [`CopyOptions`]. The same options and the same
//! outcomes give the same steps and the same record, bit for bit, on every
//! platform (scripts, keying and scores are cat-morse's integer and IEEE
//! arithmetic; this crate adds only `+ - * /` and rounding). Rendered PCM
//! is identical per platform and equal within float tolerance across
//! platforms.

use std::time::Duration;

use cat_morse::scenario::{band_channel, BandConditions, Transmission};
use cat_morse::{
    align, align_unordered, timing, Alignment, Channel, ChannelRenderer, CharStats, Fist, Score,
    ScoreOptions, UnorderedAlignment,
};

use crate::error::CoreError;
use crate::options::{
    CopyOptions, Farnsworth, Level, MAX_PITCH_HZ, MAX_WPM, MIN_PITCH_HZ, MIN_WPM,
};
use crate::over::over_channel;
use crate::plan::{expected_for, over_indices, plan_qso, OverAt, QsoPlan};
use crate::record::{OverResult, RecordedBand, SessionRecord};
use crate::speed::{AdaptRule, SpeedSetting, SpeedState};

/// The lowest sample rate a session renders at: enough for the highest
/// pitch plus a station's offset, with room to spare below Nyquist.
pub const MIN_SAMPLE_RATE_HZ: u32 = 8000;

/// How much a clipped over turns the following overs down.
pub const CLIPPED_GAIN: f32 = 0.8;

/// How long a *you* line is held: as long as it takes to send `text` at
/// `speed` with a perfect fist, or two seconds if it cannot be keyed.
pub fn you_line_hold(text: &str, speed: SpeedSetting) -> Duration {
    timing::key_text(text, speed.speed(), &Fist::perfect(), 0)
        .map(|k| k.total())
        .unwrap_or(Duration::from_secs(2))
}

/// What happens next.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// A QSO begins: number `qso` (from 1) of `of`, of kind `label`.
    QsoStart {
        qso: u32,
        of: u32,
        label: &'static str,
    },
    /// The operator's own line: shown, not played, for `hold`.
    YouLine { text: String, hold: Duration },
    /// One or more stations to play and copy. Call
    /// [`CopySession::submit`] with the outcome before the next step.
    ThemOver(ThemOver),
    /// The QSO is over: a good moment to save the stats. A speed change
    /// made during it takes effect from the next QSO.
    QsoEnd { speed_change: Option<SpeedSetting> },
}

/// A *them* over to play.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemOver {
    /// The QSO (from 1) and the over within it (from 1, *you* lines
    /// counted).
    pub qso: u32,
    pub over: u32,
    /// The first station's character speed.
    pub char_wpm: f64,
    /// Several stations at once: copy everything, in any order.
    pub several: bool,
    /// What was sent: the text, or every word several stations sent.
    pub expected: Vec<String>,
    pub audio: OverAudio,
}

/// An over's audio, ready to render at the caller's output rate.
///
/// It decides nothing about where the audio goes: play it only to an
/// output the operator chose (see the crate docs).
#[derive(Debug, Clone, PartialEq)]
pub struct OverAudio {
    channel: Channel,
}

impl OverAudio {
    /// Audio for a band built here (a conversation's composed overs).
    pub(crate) fn from_channel(channel: Channel) -> Self {
        OverAudio { channel }
    }

    /// A fresh renderer from the start. For AGN, call it again: the PCM
    /// is identical.
    pub fn renderer(&self) -> Result<ChannelRenderer, CoreError> {
        self.channel.renderer().map_err(CoreError::Render)
    }

    /// The rate the PCM is rendered at.
    pub fn sample_rate_hz(&self) -> u32 {
        self.channel.sample_rate
    }

    /// The band itself, gain included (for its duration, say).
    pub fn channel(&self) -> &Channel {
        &self.channel
    }
}

/// How the operator did on a *them* over.
#[derive(Debug, Clone, PartialEq)]
pub enum OverOutcome {
    /// Copy submitted. `agn`: replays asked for; `audio_gaps`: times the
    /// output ran dry while playing; `clipped`: > 0 if any play of this
    /// over clipped (see `ChannelRenderer::clipped`).
    Copied {
        text: String,
        agn: u32,
        audio_gaps: u32,
        clipped: u64,
    },
    /// Given up on, unscored.
    Skipped {
        agn: u32,
        audio_gaps: u32,
        clipped: u64,
    },
}

/// How a copy lined up against what was sent.
#[derive(Debug, Clone, PartialEq)]
pub enum Shown {
    /// One station: in order.
    Ordered(Alignment),
    /// Several stations: in any order.
    Unordered(UnorderedAlignment),
}

/// A submitted over: its result as recorded, and its alignment (`None`
/// for a skip).
#[derive(Debug, Clone, PartialEq)]
pub struct OverReport {
    pub result: OverResult,
    pub shown: Option<Shown>,
}

/// A finished session.
#[derive(Debug, Clone, PartialEq)]
pub struct Finished {
    pub record: SessionRecord,
    /// This session's per-character stats (merge into all-time stats with
    /// `CharStats::merge`).
    pub session_stats: CharStats,
}

/// The over waiting for its outcome.
#[derive(Debug)]
struct Pending {
    expected: Vec<String>,
    several: bool,
    char_wpm: f64,
    effective_wpm: f64,
}

/// The QSO under way.
#[derive(Debug)]
struct Qso {
    index: u32,
    /// The whole QSO runs at the speed it started at.
    speed: SpeedSetting,
    plan: QsoPlan,
    overs: Vec<OverAt>,
    next: usize,
    change: Option<SpeedSetting>,
}

#[derive(Debug)]
enum Cursor {
    /// Before QSO `n` (from 0).
    Before(u32),
    In(Box<Qso>),
    /// `next_step` has returned `None`.
    Done,
}

/// A copy session. See the [module docs](self).
#[derive(Debug)]
pub struct CopySession {
    opts: CopyOptions,
    sample_rate_hz: u32,
    speed: SpeedState,
    start_speed: SpeedSetting,
    session_stats: CharStats,
    overs: Vec<OverResult>,
    total: Score,
    gain: f32,
    cursor: Cursor,
    pending: Option<Pending>,
    quit: bool,
    error: Option<CoreError>,
}

// A session and an over's audio can be handed to another thread (an audio
// callback, a worker): checked at compile time.
const _: () = {
    const fn send<T: Send>() {}
    send::<CopySession>();
    send::<OverAudio>();
};

pub(crate) fn check_rate(hz: u32) -> Result<(), CoreError> {
    if hz < MIN_SAMPLE_RATE_HZ {
        return Err(CoreError::SampleRate(hz));
    }
    Ok(())
}

fn check_options(opts: &CopyOptions) -> Result<(), CoreError> {
    check_settings(
        opts.level,
        opts.pitch_hz,
        opts.adapt.as_ref(),
        opts.band,
        opts.farnsworth,
    )
}

/// The checks every session makes of its settings: the pitch, the
/// adaptive rule, the band conditions and a Farnsworth speed.
pub(crate) fn check_settings(
    level: Level,
    pitch_hz: f32,
    adapt: Option<&AdaptRule>,
    band: Option<BandConditions>,
    farnsworth: Farnsworth,
) -> Result<(), CoreError> {
    let p = pitch_hz;
    if !(p.is_finite() && (MIN_PITCH_HZ..=MAX_PITCH_HZ).contains(&p)) {
        return Err(CoreError::InvalidOptions(format!(
            "pitch {p} Hz is outside {MIN_PITCH_HZ}-{MAX_PITCH_HZ} Hz"
        )));
    }
    if let Some(why) = adapt.and_then(AdaptRule::problem) {
        return Err(CoreError::InvalidOptions(why));
    }
    if let Some(band) = band {
        // cat-morse checks band conditions as it builds a band.
        let mut d = level.difficulty();
        d.band = band;
        band_channel(&d, Vec::new(), MIN_SAMPLE_RATE_HZ, pitch_hz, 0)
            .map_err(|e| CoreError::InvalidOptions(format!("band conditions: {e}")))?;
    }
    if let Farnsworth::Wpm(w) = farnsworth {
        if !(MIN_WPM..=MAX_WPM).contains(&w) {
            return Err(CoreError::InvalidOptions(format!(
                "Farnsworth speed {w} wpm is outside {MIN_WPM}-{MAX_WPM} wpm"
            )));
        }
    }
    Ok(())
}

impl CopySession {
    /// A session rendering at `sample_rate_hz`. Refuses options it cannot
    /// run with and a rate below [`MIN_SAMPLE_RATE_HZ`].
    pub fn new(opts: CopyOptions, sample_rate_hz: u32) -> Result<Self, CoreError> {
        check_rate(sample_rate_hz)?;
        check_options(&opts)?;
        let speed = SpeedState::new(opts.wpm, opts.farnsworth, opts.level, opts.adapt);
        Ok(CopySession {
            start_speed: speed.current,
            speed,
            opts,
            sample_rate_hz,
            session_stats: CharStats::new(),
            overs: Vec::new(),
            total: Score::default(),
            gain: 1.0,
            cursor: Cursor::Before(0),
            pending: None,
            quit: false,
            error: None,
        })
    }

    /// Render overs from the next one on at `hz` (the output changed). It
    /// changes the PCM only, never the record.
    pub fn set_sample_rate(&mut self, hz: u32) -> Result<(), CoreError> {
        check_rate(hz)?;
        self.sample_rate_hz = hz;
        Ok(())
    }

    /// The options the session runs with.
    pub fn options(&self) -> &CopyOptions {
        &self.opts
    }

    /// The current speed (a change shows at [`Step::QsoEnd`]).
    pub fn speed(&self) -> SpeedSetting {
        self.speed.current
    }

    /// This session's per-character stats so far.
    pub fn session_stats(&self) -> &CharStats {
        &self.session_stats
    }

    fn fail(&mut self, e: CoreError) -> CoreError {
        self.pending = None;
        self.error = Some(e.clone());
        e
    }

    /// The next step, or `None` once the session is over.
    ///
    /// After an error the session has failed: every later call returns
    /// [`CoreError::Failed`], and only [`finish`](Self::finish) is left.
    pub fn next_step(&mut self) -> Result<Option<Step>, CoreError> {
        if self.error.is_some() {
            return Err(CoreError::Failed);
        }
        if self.pending.is_some() {
            return Err(CoreError::OverPending);
        }
        match &mut self.cursor {
            Cursor::Done => Ok(None),
            Cursor::Before(q) => {
                let q = *q;
                if self.quit || q >= self.opts.qsos {
                    self.cursor = Cursor::Done;
                    return Ok(None);
                }
                let speed = self.speed.current;
                let plan = match plan_qso(&self.opts, q, speed) {
                    Ok(p) => p,
                    Err(e) => return Err(self.fail(e)),
                };
                let label = plan.label;
                self.cursor = Cursor::In(Box::new(Qso {
                    index: q,
                    speed,
                    overs: over_indices(&plan.script),
                    plan,
                    next: 0,
                    change: None,
                }));
                Ok(Some(Step::QsoStart {
                    qso: q + 1,
                    of: self.opts.qsos,
                    label,
                }))
            }
            Cursor::In(qso) => {
                if self.quit || qso.next >= qso.overs.len() {
                    let step = Step::QsoEnd {
                        speed_change: qso.change,
                    };
                    self.cursor = Cursor::Before(qso.index + 1);
                    return Ok(Some(step));
                }
                let i = qso.next;
                qso.next += 1;
                let script = &qso.plan.script;
                match &qso.overs[i] {
                    OverAt::You(t) => {
                        let text = script.transmissions[*t].text.clone();
                        let hold = you_line_hold(&text, qso.speed);
                        Ok(Some(Step::YouLine { text, hold }))
                    }
                    OverAt::Them { lines, starts } => {
                        let lines: Vec<&Transmission> =
                            lines.iter().map(|k| &script.transmissions[*k]).collect();
                        let seed = qso.plan.rng.fork(1000 + i as u64).next_u64();
                        let built = over_channel(
                            script,
                            &lines,
                            starts,
                            &qso.plan.difficulty,
                            self.sample_rate_hz,
                            self.opts.pitch_hz,
                            seed,
                        );
                        let mut channel = match built {
                            Ok(c) => c,
                            Err(e) => return Err(self.fail(e)),
                        };
                        channel.gain = self.gain;
                        let sig = script
                            .signal_for(lines[0])
                            .map(|s| s.speed)
                            .unwrap_or_else(|| qso.speed.speed());
                        let over = ThemOver {
                            qso: qso.index + 1,
                            over: i as u32 + 1,
                            char_wpm: sig.char_wpm(),
                            several: lines.len() > 1,
                            expected: expected_for(&lines),
                            audio: OverAudio { channel },
                        };
                        self.pending = Some(Pending {
                            expected: over.expected.clone(),
                            several: over.several,
                            char_wpm: sig.char_wpm(),
                            effective_wpm: sig.effective_wpm(),
                        });
                        Ok(Some(Step::ThemOver(over)))
                    }
                }
            }
        }
    }

    /// The outcome of the [`Step::ThemOver`] just stepped: scored (or
    /// recorded as skipped), counted into the stats and the adaptive rule.
    pub fn submit(&mut self, outcome: OverOutcome) -> Result<OverReport, CoreError> {
        if self.error.is_some() {
            return Err(CoreError::Failed);
        }
        let Some(p) = self.pending.take() else {
            return Err(CoreError::NoOverPending);
        };
        let Cursor::In(qso) = &mut self.cursor else {
            unreachable!("an over waits only inside a QSO");
        };
        let (agn, audio_gaps, clipped) = match &outcome {
            OverOutcome::Copied {
                agn,
                audio_gaps,
                clipped,
                ..
            }
            | OverOutcome::Skipped {
                agn,
                audio_gaps,
                clipped,
            } => (*agn, *audio_gaps, *clipped),
        };
        if clipped > 0 {
            self.gain *= CLIPPED_GAIN;
        }
        let mut result = OverResult {
            qso: qso.index + 1,
            kind: qso.plan.label.to_string(),
            expected: p.expected,
            copied: String::new(),
            score: Score::default(),
            agn,
            skipped: false,
            audio_gaps,
            char_wpm: p.char_wpm,
            effective_wpm: p.effective_wpm,
        };
        let text = match outcome {
            OverOutcome::Skipped { .. } => {
                result.skipped = true;
                self.overs.push(result.clone());
                return Ok(OverReport {
                    result,
                    shown: None,
                });
            }
            OverOutcome::Copied { text, .. } => text,
        };
        let score_opts = ScoreOptions::default();
        let aligned = if p.several {
            let refs: Vec<&str> = result.expected.iter().map(String::as_str).collect();
            align_unordered(&refs, &text, &score_opts).map(Shown::Unordered)
        } else {
            align(&result.expected[0], &text, &score_opts).map(Shown::Ordered)
        };
        let shown = match aligned {
            Ok(s) => s,
            Err(e) => return Err(self.fail(CoreError::Align(e))),
        };
        let score = match &shown {
            Shown::Unordered(u) => {
                self.session_stats.record_unordered(u);
                u.score()
            }
            Shown::Ordered(a) => {
                self.session_stats.record(a);
                a.score()
            }
        };
        self.total.add(&score);
        result.copied = text;
        result.score = score;
        self.overs.push(result.clone());
        if let Some(c) = self.speed.record(&score, agn) {
            qso.change = Some(c);
        }
        Ok(OverReport {
            result,
            shown: Some(shown),
        })
    }

    /// The operator quits. Valid at any step: a *them* over waiting for
    /// its outcome is dropped unrecorded, the next step ends the QSO under
    /// way (if any) with [`Step::QsoEnd`], and the one after is `None`.
    /// Once the session has ended (`next_step` gave `None`) it does
    /// nothing: the session was not cut short.
    pub fn quit(&mut self) {
        if matches!(self.cursor, Cursor::Done) {
            return;
        }
        self.quit = true;
        self.pending = None;
    }

    /// End the session and take its record. Valid at any time, also after
    /// an error; `ended_by_error` is the caller's error text (by default
    /// the session's own error, if it failed).
    ///
    /// `quit_early` is set if the operator quit, the session failed, an
    /// error is given, or the session had not yet reached its end.
    pub fn finish(self, ended_by_error: Option<String>) -> Finished {
        let ended_by_error = ended_by_error.or_else(|| self.error.as_ref().map(|e| e.to_string()));
        let reached_end = matches!(self.cursor, Cursor::Done);
        let quit_early =
            self.quit || self.error.is_some() || ended_by_error.is_some() || !reached_end;
        let opts = self.opts;
        Finished {
            record: SessionRecord {
                version: 1,
                started: opts.started,
                seed: opts.seed,
                kind: opts.kind.name().to_string(),
                level: opts.level.name().to_string(),
                call: opts.call.as_str().to_string(),
                pitch_hz: opts.pitch_hz,
                audio_out: opts.audio_out,
                start_speed: self.start_speed,
                end_speed: self.speed.current,
                overs: self.overs,
                total: self.total,
                quit_early,
                ended_by_error,
                adapt_rule: opts.adapt.filter(|r| *r != AdaptRule::default()),
                band: opts
                    .band
                    .filter(|b| *b != opts.level.difficulty().band)
                    .map(RecordedBand::from),
                turns: Vec::new(),
            },
            session_stats: self.session_stats,
        }
    }
}

#[cfg(test)]
mod tests {
    use cat_morse::Callsign;

    use super::*;
    use crate::options::{Kind, Level};

    fn options() -> CopyOptions {
        CopyOptions {
            kind: Kind::RagChew,
            level: Level::Intermediate,
            qsos: 2,
            call: Callsign::parse("KF0UWV").unwrap(),
            wpm: 20,
            farnsworth: Farnsworth::Off,
            pitch_hz: 600.0,
            seed: 7,
            adapt: None,
            band: None,
            started: "2026-11-07T12:00:00Z".to_string(),
            audio_out: "Test speaker".to_string(),
        }
    }

    /// A session whose next *them* over cannot be built: its pitch is
    /// pushed past what the sample rate carries, which `new` would refuse.
    fn failing_at_the_first_them_over() -> (CopySession, CoreError, usize) {
        let mut s = CopySession::new(options(), MIN_SAMPLE_RATE_HZ).unwrap();
        s.opts.pitch_hz = 3990.0;
        let mut steps = 0;
        loop {
            match s.next_step() {
                Ok(Some(_)) => steps += 1,
                Ok(None) => panic!("no them over"),
                Err(e) => return (s, e, steps),
            }
        }
    }

    #[test]
    fn after_an_error_only_finish_is_left_and_it_keeps_the_error() {
        let (mut s, e, steps) = failing_at_the_first_them_over();
        assert!(steps >= 1, "the QSO started");
        assert!(matches!(e, CoreError::Band(_)), "{e}");
        assert!(e.to_string().starts_with("cannot build the band: "), "{e}");
        assert_eq!(s.next_step(), Err(CoreError::Failed));
        assert_eq!(
            s.submit(OverOutcome::Skipped {
                agn: 0,
                audio_gaps: 0,
                clipped: 0
            }),
            Err(CoreError::Failed)
        );
        // As the CLI does: finish with the error's text.
        let r = s.finish(Some(e.to_string())).record;
        assert!(r.quit_early);
        assert_eq!(r.ended_by_error, Some(e.to_string()));
        assert!(r.overs.is_empty());

        // Without a text, the session's own error is kept.
        let (s, e, _) = failing_at_the_first_them_over();
        assert_eq!(s.finish(None).record.ended_by_error, Some(e.to_string()));
    }

    #[test]
    fn what_was_scored_before_an_error_is_kept() {
        let mut s = CopySession::new(options(), MIN_SAMPLE_RATE_HZ).unwrap();
        let mut scored = 0;
        while let Some(step) = s.next_step().unwrap() {
            if let Step::ThemOver(o) = step {
                s.submit(OverOutcome::Copied {
                    text: o.expected.join(" "),
                    agn: 0,
                    audio_gaps: 0,
                    clipped: 0,
                })
                .unwrap();
                scored += 1;
                if scored == 2 {
                    break;
                }
            }
        }
        s.opts.pitch_hz = 3990.0;
        let e = loop {
            match s.next_step() {
                Ok(Some(_)) => {}
                Ok(None) => panic!("ended without the error"),
                Err(e) => break e,
            }
        };
        let r = s.finish(Some(e.to_string())).record;
        assert_eq!(r.overs.len(), 2);
        assert_eq!(r.total.cer(), 0.0);
        assert!(r.quit_early);
    }
}
