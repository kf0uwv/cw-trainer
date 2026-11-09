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

//! Conversation practice: a rag-chew in which the other station answers
//! what the operator actually sent.
//!
//! The session plays the other station; the operator's overs come in as
//! text (decoded from their keying by the caller), worded however they
//! like. The station reads each one ([`heard`]), answers questions, asks
//! again for what it missed, corrects a miscopied call, slows down on
//! `QRS` and follows a closing whenever it comes ([`station`]).
//!
//! ```text
//! new ─▶ next_turn ─▶ Them ─▶ next_turn ─▶ You ─▶ reply ─▶ next_turn ─▶ Them ─▶ … ─▶ End ─▶ None ─▶ finish
//! ```
//!
//! With [`Opening::YouCallCq`] the first turn is the operator's. A reply
//! nobody answers (a CQ with no readable call) leads straight to the
//! operator's next turn.
//!
//! # Determinism
//!
//! As for a copy session: the caller supplies the seed and the start
//! time, and the same options and replies give the same overs, audio and
//! record on every platform.
//!
//! # Audio
//!
//! The session renders only the other station, inside the level's band,
//! for the caller to play to an output the operator chose. The operator's
//! own keying never passes through it.

pub mod heard;
pub mod station;

use cat_morse::scenario::{BandConditions, Difficulty};
use cat_morse::{generate, Callsign, CharStats, Rng, ScenarioConfig, ScenarioKind, Score};
use serde::{Deserialize, Serialize};

pub use heard::{hear, Handover, Heard, Topic};
pub use station::{Hint, Phase, Station};

use crate::copy::{check_rate, check_settings, Finished, OverAudio};
use crate::error::CoreError;
use crate::options::{Farnsworth, Level, MAX_WPM, MIN_WPM};
use crate::over::text_channel;
use crate::record::{ConverseTurn, RecordedBand, SessionRecord};
use crate::speed::{difficulty_for, SpeedSetting, SpeedState, STEP_WPM};

/// The `kind` a conversation's record is kept under.
pub const KIND: &str = "conversation";

/// Who calls CQ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opening {
    /// The station calls CQ; the operator answers.
    TheyCallCq,
    /// The operator calls CQ; the station answers.
    YouCallCq,
}

/// Everything a conversation needs: no radio in here.
#[derive(Debug, Clone, PartialEq)]
pub struct ConverseOptions {
    /// The operator's call.
    pub call: Callsign,
    pub opening: Opening,
    /// How hard the other station is to copy (fist, strength, band).
    pub level: Level,
    /// The station's speed; `QRS`/`QRQ` move it.
    pub wpm: u32,
    pub farnsworth: Farnsworth,
    pub pitch_hz: f32,
    pub seed: u64,
    /// Band conditions in place of the level's own.
    pub band: Option<BandConditions>,
    /// When the session started (ISO 8601 UTC), for the history.
    pub started: String,
    /// The output device's name, kept in the history.
    pub audio_out: String,
}

/// How the operator's over read.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Checks {
    /// The station's call, as sent before `DE`, was right (`None`: not sent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub their_call: Option<bool>,
    /// The operator's own call, as sent after `DE`, was right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub your_call: Option<bool>,
    /// The name the operator called the station by was its name (`None`:
    /// no name used, or the station had not given it yet).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub their_name: Option<bool>,
    /// The report sent, as digits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handover: Option<Handover>,
    /// Characters the decoder could not read.
    #[serde(default)]
    pub undecoded: u32,
}

/// The station's over, to play.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemTurn {
    /// The station's overs so far, this one included (from 1).
    pub over: u32,
    /// What it sends (show it after it has played, if at all).
    pub text: String,
    pub char_wpm: f64,
    /// A repeat the operator asked for (`AGN`).
    pub repeat: bool,
    pub audio: OverAudio,
}

/// What happens next.
#[derive(Debug, Clone, PartialEq)]
pub enum Turn {
    /// The station sends: play it, then call `next_turn`.
    Them(ThemTurn),
    /// The operator's turn: call [`ConversationSession::reply`].
    You { hint: Hint },
    /// The contact is over (the next call gives `None`).
    End,
}

/// A reply, as the station took it.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplyReport {
    pub heard: Heard,
    pub checks: Checks,
    /// The station answers (else the operator goes again).
    pub answered: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Next {
    /// The station sends `text`.
    Them { text: String, repeat: bool },
    /// The operator's turn.
    You,
    /// Waiting for `reply`.
    Waiting,
    /// `End` is next.
    End,
    /// `next` has returned `None`.
    Done,
}

/// A conversation. See the [module docs](self).
#[derive(Debug)]
pub struct ConversationSession {
    opts: ConverseOptions,
    sample_rate_hz: u32,
    station: Station,
    difficulty: Difficulty,
    rng: Rng,
    start_speed: SpeedSetting,
    speed: SpeedSetting,
    next: Next,
    them_overs: u32,
    replies: u32,
    turns: Vec<ConverseTurn>,
    quit: bool,
    error: Option<CoreError>,
}

const _: () = {
    const fn send<T: Send>() {}
    send::<ConversationSession>();
};

impl ConversationSession {
    /// A conversation rendering at `sample_rate_hz`. Refuses options it
    /// cannot run with and a rate below the minimum.
    pub fn new(opts: ConverseOptions, sample_rate_hz: u32) -> Result<Self, CoreError> {
        check_rate(sample_rate_hz)?;
        check_settings(opts.level, opts.pitch_hz, None, opts.band, opts.farnsworth)?;
        let speed = SpeedState::new(opts.wpm, opts.farnsworth, opts.level, None).current;
        let mut difficulty = difficulty_for(opts.level, speed);
        if let Some(band) = opts.band {
            difficulty.band = band;
        }
        let rng = Rng::new(opts.seed);
        // The station comes from a scripted rag-chew's: its call, name,
        // QTH, rig and fist from cat-morse's own pools.
        let script = generate(&ScenarioConfig {
            kind: ScenarioKind::RagChew,
            my_call: opts.call.clone(),
            difficulty: difficulty.clone(),
            seed: rng.fork(0).next_u64(),
        })
        .map_err(CoreError::Generate)?;
        let them = script
            .stations
            .into_iter()
            .next()
            .ok_or(CoreError::NoStation)?;
        let mut signal = them.signal;
        signal.speed = speed.speed();
        let listening = opts.opening == Opening::YouCallCq;
        let mut station = Station::new(them.persona, signal, listening, &mut rng.fork(1));
        let next = if listening {
            Next::You
        } else {
            Next::Them {
                text: station.cq(&mut rng.fork(2)),
                repeat: false,
            }
        };
        Ok(ConversationSession {
            opts,
            sample_rate_hz,
            station,
            difficulty,
            rng,
            start_speed: speed,
            speed,
            next,
            them_overs: 0,
            replies: 0,
            turns: Vec::new(),
            quit: false,
            error: None,
        })
    }

    /// Render overs from the next one on at `hz`.
    pub fn set_sample_rate(&mut self, hz: u32) -> Result<(), CoreError> {
        check_rate(hz)?;
        self.sample_rate_hz = hz;
        Ok(())
    }

    pub fn options(&self) -> &ConverseOptions {
        &self.opts
    }

    /// The station's speed now (`QRS` and `QRQ` move it).
    pub fn speed(&self) -> SpeedSetting {
        self.speed
    }

    /// What the operator should send next.
    pub fn hint(&self) -> Hint {
        self.station.hint()
    }

    fn fail(&mut self, e: CoreError) -> CoreError {
        self.error = Some(e.clone());
        e
    }

    /// The next turn, or `None` once the contact is over.
    pub fn next_turn(&mut self) -> Result<Option<Turn>, CoreError> {
        if self.error.is_some() {
            return Err(CoreError::Failed);
        }
        if self.quit && !matches!(self.next, Next::Done) {
            self.next = Next::End;
        }
        match std::mem::replace(&mut self.next, Next::Waiting) {
            Next::Waiting => Err(CoreError::ReplyPending),
            Next::Done => {
                self.next = Next::Done;
                Ok(None)
            }
            Next::End => {
                self.next = Next::Done;
                Ok(Some(Turn::End))
            }
            Next::You => Ok(Some(Turn::You {
                hint: self.station.hint(),
            })),
            Next::Them { text, repeat } => {
                self.them_overs += 1;
                let seed = self.rng.fork(1000 + u64::from(self.them_overs)).next_u64();
                let mut channel = match text_channel(
                    &text,
                    &self.station.signal,
                    &self.difficulty,
                    self.sample_rate_hz,
                    self.opts.pitch_hz,
                    seed,
                ) {
                    Ok(c) => c,
                    Err(e) => return Err(self.fail(e)),
                };
                channel.gain = 1.0;
                self.turns.push(ConverseTurn {
                    you: false,
                    text: text.clone(),
                    checks: None,
                    repeat,
                });
                self.next = if self.station.phase() == Phase::Done {
                    Next::End
                } else {
                    Next::You
                };
                Ok(Some(Turn::Them(ThemTurn {
                    over: self.them_overs,
                    text,
                    char_wpm: self.station.signal.speed.char_wpm(),
                    repeat,
                    audio: OverAudio::from_channel(channel),
                })))
            }
        }
    }

    /// The operator's over, as decoded. Valid only on their turn.
    pub fn reply(&mut self, sent: &str) -> Result<ReplyReport, CoreError> {
        if self.error.is_some() {
            return Err(CoreError::Failed);
        }
        if self.next != Next::Waiting || self.quit {
            return Err(CoreError::NotYourTurn);
        }
        self.replies += 1;
        let heard = hear(sent);
        let checks = Checks {
            their_call: heard.to_call.as_ref().map(|c| c == self.station.call()),
            your_call: heard
                .from_call
                .as_ref()
                .map(|c| c == self.opts.call.as_str()),
            their_name: self.station.told_name().and_then(|n| {
                (!heard.addressed.is_empty()).then(|| heard.addressed.iter().any(|a| a == n))
            }),
            report: heard.rst.clone(),
            handover: heard.handover,
            undecoded: heard.undecoded,
        };
        self.turns.push(ConverseTurn {
            you: true,
            text: heard.text.clone(),
            checks: Some(checks.clone()),
            repeat: false,
        });
        if heard.qrs || heard.qrq {
            self.change_speed(heard.qrs);
        }
        let repeat_only = heard.repeat && self.station.last().is_some();
        let before = self.station.last().map(str::to_string);
        let mut rng = self.rng.fork(2000 + u64::from(self.replies));
        let answer = self.station.respond(&heard, &mut rng);
        let answered = answer.is_some();
        self.next = match answer {
            Some(text) => Next::Them {
                repeat: repeat_only && before.as_deref() == Some(text.as_str()),
                text,
            },
            None => Next::You,
        };
        Ok(ReplyReport {
            heard,
            checks,
            answered,
        })
    }

    fn change_speed(&mut self, slower: bool) {
        let c = self.speed.char_wpm;
        let c = if slower {
            c.saturating_sub(STEP_WPM).max(MIN_WPM)
        } else {
            (c + STEP_WPM).min(MAX_WPM)
        };
        self.speed = SpeedSetting {
            char_wpm: c,
            effective_wpm: self.speed.effective_wpm.filter(|e| *e < c),
        };
        self.station.signal.speed = self.speed.speed();
    }

    /// The operator ends the contact: the next call gives [`Turn::End`]
    /// (unless it is already over), then `None`.
    pub fn quit(&mut self) {
        if !matches!(self.next, Next::Done | Next::End) {
            self.quit = true;
        }
    }

    /// End the session and take its record. Valid at any time.
    pub fn finish(self, ended_by_error: Option<String>) -> Finished {
        let ended_by_error = ended_by_error.or_else(|| self.error.as_ref().map(|e| e.to_string()));
        let reached_end = matches!(self.next, Next::Done | Next::End) && !self.quit;
        let quit_early = self.quit || ended_by_error.is_some() || !reached_end;
        let opts = self.opts;
        Finished {
            record: SessionRecord {
                version: 1,
                started: opts.started,
                seed: opts.seed,
                kind: KIND.to_string(),
                level: opts.level.name().to_string(),
                call: opts.call.as_str().to_string(),
                pitch_hz: opts.pitch_hz,
                audio_out: opts.audio_out,
                start_speed: self.start_speed,
                end_speed: self.speed,
                overs: Vec::new(),
                total: Score::default(),
                quit_early,
                ended_by_error,
                adapt_rule: None,
                band: opts
                    .band
                    .filter(|b| *b != opts.level.difficulty().band)
                    .map(RecordedBand::from),
                turns: self.turns,
            },
            session_stats: CharStats::new(),
        }
    }
}
