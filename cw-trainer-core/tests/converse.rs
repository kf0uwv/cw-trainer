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

//! Whole conversations, driven as an app would drive one.

use cw_trainer_core::cat_morse::Callsign;
use cw_trainer_core::converse::{
    ConversationSession, ConverseOptions, Hint, Opening, ReplyReport, ThemTurn, Turn, KIND,
};
use cw_trainer_core::options::{Farnsworth, Level};
use cw_trainer_core::record::SessionRecord;
use cw_trainer_core::CoreError;

const RATE: u32 = 8000;

fn options(opening: Opening, seed: u64) -> ConverseOptions {
    ConverseOptions {
        call: Callsign::parse("KF0UWV").unwrap(),
        opening,
        level: Level::Beginner,
        wpm: 18,
        farnsworth: Farnsworth::Off,
        pitch_hz: 600.0,
        seed,
        band: None,
        started: "2026-11-08T12:00:00Z".to_string(),
        audio_out: "Test speaker".to_string(),
    }
}

/// The station's call, read off its first over the way an operator
/// would: the word after `DE`.
fn call_in(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let at = words.iter().position(|w| *w == "DE").unwrap();
    words[at + 1].to_string()
}

fn them(s: &mut ConversationSession) -> ThemTurn {
    match s.next_turn().unwrap() {
        Some(Turn::Them(t)) => t,
        other => panic!("expected the station, got {other:?}"),
    }
}

fn you(s: &mut ConversationSession) -> Hint {
    match s.next_turn().unwrap() {
        Some(Turn::You { hint }) => hint,
        other => panic!("expected the operator's turn, got {other:?}"),
    }
}

/// Run a contact with `replies` (each may use `{c}`, the station's call),
/// answering every turn; returns the station's overs and the reports.
fn run(
    opening: Opening,
    seed: u64,
    replies: &[&str],
) -> (Vec<String>, Vec<ReplyReport>, SessionRecord) {
    let mut s = ConversationSession::new(options(opening, seed), RATE).unwrap();
    let mut overs = Vec::new();
    let mut reports = Vec::new();
    let mut c = String::new();
    let mut replies = replies.iter();
    loop {
        match s.next_turn().unwrap() {
            Some(Turn::Them(t)) => {
                if c.is_empty() {
                    c = call_in(&t.text);
                }
                assert!(t.audio.channel().duration_estimate_nonzero());
                overs.push(t.text);
            }
            Some(Turn::You { .. }) => {
                let Some(r) = replies.next() else { break };
                reports.push(s.reply(&r.replace("{c}", &c)).unwrap());
            }
            Some(Turn::End) => {}
            None => break,
        }
    }
    (overs, reports, s.finish(None).record)
}

trait NonEmpty {
    fn duration_estimate_nonzero(&self) -> bool;
}

impl NonEmpty for cw_trainer_core::cat_morse::Channel {
    fn duration_estimate_nonzero(&self) -> bool {
        !self.signals.is_empty() && self.sample_rate == RATE
    }
}

#[test]
fn answering_a_cq_runs_a_whole_rag_chew() {
    let (overs, reports, record) = run(
        Opening::TheyCallCq,
        7,
        &[
            "{c} DE KF0UWV KF0UWV K",
            "{c} DE KF0UWV R TNX UR RST 579 579 NAME MATT MATT QTH DENVER CO <BT> HW? KN",
            "R FB RIG KX3 PWR 5W ANT EFHW <BT> WX SUNNY <BT> KN",
            "R TNX FER QSO 73 <SK> {c} DE KF0UWV TU E E",
        ],
    );
    assert!(overs[0].starts_with("CQ"), "{overs:?}");
    assert!(overs[1].contains("TNX FER CALL"), "{overs:?}");
    assert!(overs.iter().any(|o| o.contains("RIG ")), "{overs:?}");
    assert!(overs.last().unwrap().contains("<SK>"), "{overs:?}");
    assert!(overs.last().unwrap().contains("MATT"), "{overs:?}");
    assert_eq!(reports[0].checks.their_call, Some(true));
    assert_eq!(reports[0].checks.your_call, Some(true));
    assert_eq!(reports[1].checks.report.as_deref(), Some("579"));

    assert_eq!(record.kind, KIND);
    assert!(!record.quit_early, "{record:?}");
    assert_eq!(record.turns.len(), overs.len() + reports.len());
    assert!(!record.turns[0].you);
    assert!(record.overs.is_empty());
}

#[test]
fn calling_cq_runs_a_whole_rag_chew() {
    let mut s = ConversationSession::new(options(Opening::YouCallCq, 3), RATE).unwrap();
    assert_eq!(you(&mut s), Hint::CallCq);
    // Nobody answers a CQ without a readable call.
    assert!(!s.reply("CQ CQ CQ K").unwrap().answered);
    assert_eq!(you(&mut s), Hint::CallCq);
    assert!(s.reply("CQ CQ CQ DE KF0UWV KF0UWV K").unwrap().answered);
    let answer = them(&mut s);
    assert!(answer.text.starts_with("KF0UWV DE "), "{}", answer.text);
    let c = call_in(&answer.text);
    assert_eq!(you(&mut s), Hint::Exchange);
    s.reply(&format!(
        "{c} DE KF0UWV GE TNX FER CALL UR 5NN NAME MATT QTH DENVER KN"
    ))
    .unwrap();
    let exchange = them(&mut s);
    assert!(exchange.text.contains("NAME "), "{}", exchange.text);
    let name = {
        let w: Vec<&str> = exchange.text.split_whitespace().collect();
        w[w.iter().position(|x| *x == "NAME").unwrap() + 1].to_string()
    };
    assert_eq!(you(&mut s), Hint::Chat);
    // Calling the station by its name is checked.
    let r = s.reply(&format!("R TNX {name} RIG KX3 KN")).unwrap();
    assert_eq!(r.checks.their_name, Some(true));
    them(&mut s);
    you(&mut s);
    let r = s.reply("R TNX BOGUS 73 SK").unwrap();
    assert_eq!(r.checks.their_name, Some(false));
    let last = them(&mut s);
    assert!(last.text.contains("<SK>"), "{}", last.text);
    assert_eq!(s.next_turn().unwrap(), Some(Turn::End));
    assert_eq!(s.next_turn().unwrap(), None);
    assert!(!s.finish(None).record.quit_early);
}

#[test]
fn qrs_slows_the_station_and_agn_repeats_its_over() {
    let mut s = ConversationSession::new(options(Opening::TheyCallCq, 11), RATE).unwrap();
    let cq = them(&mut s);
    you(&mut s);
    s.reply("AGN").unwrap();
    let again = them(&mut s);
    assert_eq!(again.text, cq.text);
    assert!(again.repeat);
    you(&mut s);
    let c = call_in(&cq.text);
    s.reply(&format!("{c} DE KF0UWV PSE QRS K")).unwrap();
    let slower = them(&mut s);
    assert!(
        slower.char_wpm < cq.char_wpm,
        "{} {}",
        slower.char_wpm,
        cq.char_wpm
    );
    assert_eq!(s.speed().char_wpm, 16);
    let record = s.finish(None).record;
    assert_eq!(record.end_speed.char_wpm, 16);
    assert!(record.quit_early);
}

#[test]
fn out_of_turn_calls_are_refused() {
    let mut s = ConversationSession::new(options(Opening::TheyCallCq, 1), RATE).unwrap();
    assert_eq!(s.reply("K"), Err(CoreError::NotYourTurn));
    them(&mut s);
    you(&mut s);
    assert_eq!(s.next_turn(), Err(CoreError::ReplyPending));
    s.quit();
    assert_eq!(s.reply("K"), Err(CoreError::NotYourTurn));
    assert_eq!(s.next_turn().unwrap(), Some(Turn::End));
    assert_eq!(s.next_turn().unwrap(), None);
    assert!(s.finish(None).record.quit_early);
}

#[test]
fn a_bad_rate_or_pitch_is_refused() {
    assert_eq!(
        ConversationSession::new(options(Opening::TheyCallCq, 1), 4000).unwrap_err(),
        CoreError::SampleRate(4000)
    );
    let mut o = options(Opening::TheyCallCq, 1);
    o.pitch_hz = 50.0;
    assert!(matches!(
        ConversationSession::new(o, RATE),
        Err(CoreError::InvalidOptions(_))
    ));
}

#[test]
fn the_same_seed_and_replies_give_the_same_contact_and_audio() {
    let replies = [
        "{c} DE KF0UWV K",
        "R UR 599 NAME MATT QTH DENVER KN",
        "NAME? KN",
        "73 SK",
    ];
    let (a, _, ra) = run(Opening::TheyCallCq, 42, &replies);
    let (b, _, rb) = run(Opening::TheyCallCq, 42, &replies);
    assert_eq!(a, b);
    assert_eq!(
        serde_json::to_string(&ra).unwrap(),
        serde_json::to_string(&rb).unwrap()
    );
    let (c, _, _) = run(Opening::TheyCallCq, 43, &replies);
    assert_ne!(a, c, "another seed, another station");

    let render = |seed| {
        let mut s = ConversationSession::new(options(Opening::TheyCallCq, seed), RATE).unwrap();
        let mut r = them(&mut s).audio.renderer().unwrap();
        let mut buf = vec![0.0f32; 4096];
        let n = r.fill(&mut buf);
        buf.truncate(n);
        buf
    };
    assert_eq!(render(42), render(42));
}

#[test]
fn a_conversation_record_round_trips_and_a_copy_record_has_no_turns() {
    let (_, _, record) = run(
        Opening::TheyCallCq,
        5,
        &["{c} DE KF0UWV K", "R 5NN MATT KN", "73 SK"],
    );
    let line = serde_json::to_string(&record).unwrap();
    assert!(line.contains("\"turns\":["), "{line}");
    assert!(line.contains("\"checks\":"), "{line}");
    let back: SessionRecord = serde_json::from_str(&line).unwrap();
    assert_eq!(back, record);

    let mut copy = record.clone();
    copy.turns.clear();
    let line = serde_json::to_string(&copy).unwrap();
    assert!(!line.contains("turns"), "{line}");
}
