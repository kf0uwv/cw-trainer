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

//! The other station in a conversation: who it is, what it has told and
//! been told, and how it answers each of the operator's overs.
//!
//! It works a rag-chew the way `cat_morse`'s scripted ones run (calls,
//! then report, name and QTH, then rig, power, antenna and weather, then
//! 73), but it waits for the operator at every step, answers questions,
//! asks again for what it missed and follows a closing whenever it comes.

use cat_morse::scenario::{Persona, SignalProfile};
use cat_morse::Rng;
use serde::{Deserialize, Serialize};

use crate::converse::heard::{Heard, Topic};

/// After this many of the operator's overs the station starts to close.
pub const MAX_REPLIES: u32 = 10;

const WEATHER: &[&str] = &[
    "SUNNY", "CLOUDY", "RAIN", "SNOW", "WINDY", "CLEAR", "FOG", "HOT",
];
const REPORTS: &[&str] = &["599", "589", "579", "569", "559", "449"];
const GREETINGS: &[&str] = &["GM", "GA", "GE", "HI"];
const CLOSES: &[&str] = &["73 ES GUD DX", "73 ES CUL", "73 GL"];

/// What the operator is expected to send next: a prompt, never a script.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Hint {
    /// Call CQ with your call.
    CallCq,
    /// Answer the station's CQ: its call, `DE`, yours.
    AnswerCq,
    /// Your report, name and QTH.
    Exchange,
    /// Rig, power, antenna, weather, a question, or 73.
    Chat,
    /// 73 and `<SK>`.
    Close,
}

impl Hint {
    /// A short prompt for the operator.
    pub fn describe(self) -> &'static str {
        match self {
            Hint::CallCq => "Call CQ: CQ CQ DE your call, then K",
            Hint::AnswerCq => "Answer the CQ: their call DE your call, then K",
            Hint::Exchange => "Send a report, your name and your QTH, then KN",
            Hint::Chat => "Chat: rig, power, antenna, weather or a question; 73 to close",
            Hint::Close => "Close: 73 and SK",
        }
    }
}

/// Where the contact is, from the station's side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The station is calling CQ.
    Calling,
    /// The station listens for the operator's CQ.
    Listening,
    /// The station answered the operator's CQ.
    Answered,
    /// The station sent its report, name and QTH.
    Exchange,
    /// The station sent its rig and weather too.
    Chat,
    /// The station sent its 73.
    Closing,
    /// The contact is over.
    Done,
}

/// What one side has said.
#[derive(Debug, Clone, Default, PartialEq)]
struct Facts {
    rst: Option<String>,
    name: Option<String>,
    qth: Option<String>,
    rig: Option<String>,
    power: Option<String>,
    antenna: Option<String>,
    weather: Option<String>,
}

/// The other station.
#[derive(Debug, Clone)]
pub struct Station {
    pub persona: Persona,
    pub signal: SignalProfile,
    call: String,
    phase: Phase,
    /// The operator's call, as heard.
    you: Option<String>,
    /// What the operator told.
    heard: Facts,
    /// The report the station gives the operator.
    report: String,
    weather: String,
    told_exchange: bool,
    told_chat: bool,
    /// Asked of the operator already: never asked twice.
    asked: Vec<Topic>,
    /// The station's last over, for `AGN`.
    last: Option<String>,
    replies: u32,
}

fn temperature(p: &Persona, rng: &mut Rng) -> String {
    if p.region.len() == 2 {
        format!("{}F", rng.range_u32(20, 95))
    } else {
        format!("{}C", rng.range_u32(0, 35))
    }
}

fn spaced(parts: &[String]) -> String {
    parts
        .iter()
        .flat_map(|p| p.split_whitespace())
        .collect::<Vec<_>>()
        .join(" ")
}

impl Station {
    /// The station, its report for the operator and its weather drawn
    /// from `rng`. It calls CQ first unless `listening`.
    pub fn new(persona: Persona, signal: SignalProfile, listening: bool, rng: &mut Rng) -> Self {
        let report = rng.pick(REPORTS).to_string();
        let weather = format!("{} {}", rng.pick(WEATHER), temperature(&persona, rng));
        Station {
            call: persona.call.as_str().to_string(),
            persona,
            signal,
            phase: if listening {
                Phase::Listening
            } else {
                Phase::Calling
            },
            you: None,
            heard: Facts::default(),
            report,
            weather,
            told_exchange: false,
            told_chat: false,
            asked: Vec::new(),
            last: None,
            replies: 0,
        }
    }

    pub fn call(&self) -> &str {
        &self.call
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The operator's call as the station heard it.
    pub fn heard_call(&self) -> Option<&str> {
        self.you.as_deref()
    }

    /// The station's name, once it has told it.
    pub fn told_name(&self) -> Option<&str> {
        self.told_exchange.then_some(self.persona.name.as_str())
    }

    /// What the operator should send next.
    pub fn hint(&self) -> Hint {
        match self.phase {
            Phase::Listening => Hint::CallCq,
            Phase::Calling => Hint::AnswerCq,
            Phase::Answered => Hint::Exchange,
            Phase::Exchange if self.missing().is_empty() => Hint::Chat,
            Phase::Exchange => Hint::Exchange,
            Phase::Chat => Hint::Chat,
            Phase::Closing | Phase::Done => Hint::Close,
        }
    }

    /// Of the report, name and QTH, what the operator has not given.
    pub fn missing(&self) -> Vec<Topic> {
        let mut m = Vec::new();
        if self.heard.rst.is_none() {
            m.push(Topic::Rst);
        }
        if self.heard.name.is_none() {
            m.push(Topic::Name);
        }
        if self.heard.qth.is_none() {
            m.push(Topic::Qth);
        }
        m
    }

    /// The opening CQ.
    pub fn cq(&mut self, rng: &mut Rng) -> String {
        let c = &self.call;
        let text = match rng.range_u32(0, 2) {
            0 => format!("CQ CQ CQ DE {c} {c} {c} K"),
            1 => format!("CQ CQ DE {c} {c} K"),
            _ => format!("CQ CQ CQ DE {c} {c} <AR> K"),
        };
        self.last = Some(text.clone());
        text
    }

    fn learn(&mut self, h: &Heard) {
        let call = h
            .from_call
            .clone()
            .or_else(|| h.calls.iter().find(|c| **c != self.call).cloned());
        if let Some(c) = call {
            self.you = Some(c);
        }
        let f = &mut self.heard;
        for (slot, v) in [
            (&mut f.rst, &h.rst),
            (&mut f.name, &h.name),
            (&mut f.qth, &h.qth),
            (&mut f.rig, &h.rig),
            (&mut f.power, &h.power),
            (&mut f.antenna, &h.antenna),
            (&mut f.weather, &h.weather),
        ] {
            if v.is_some() {
                slot.clone_from(v);
            }
        }
    }

    fn you(&self) -> String {
        self.you.clone().unwrap_or_else(|| "?".to_string())
    }

    fn answer(&self, q: Topic, heard: &Heard) -> String {
        let p = &self.persona;
        match q {
            Topic::Call => format!("MY CALL {c} {c}", c = self.call),
            Topic::Rst => format!("UR RST {r} {r}", r = self.report),
            Topic::Name => format!("NAME {n} {n}", n = p.name),
            Topic::Qth => format!("QTH {q} {q}", q = self.qth()),
            Topic::Rig => format!("RIG {}", p.rig),
            Topic::Power => format!("PWR {}W", p.power_w),
            Topic::Antenna => format!("ANT {}", p.antenna),
            Topic::Weather => format!("WX {}", self.weather),
            Topic::Copy if heard.undecoded == 0 => "SOLID CPY".to_string(),
            Topic::Copy => "QSB HR BUT MOSTLY OK".to_string(),
        }
    }

    fn qth(&self) -> String {
        let p = &self.persona;
        if p.region.is_empty() || p.qth == "AT SEA" {
            p.qth.clone()
        } else {
            format!("{}, {}", p.qth, p.region)
        }
    }

    fn exchange(&self) -> String {
        let p = &self.persona;
        format!(
            "UR RST {r} {r} <BT> NAME {n} {n} <BT> QTH {q} {q} <BT> HW CPY?",
            r = self.report,
            n = p.name,
            q = self.qth()
        )
    }

    fn chat(&self) -> String {
        let p = &self.persona;
        format!(
            "RIG {} PWR {}W ANT {} <BT> WX {}",
            p.rig, p.power_w, p.antenna, self.weather
        )
    }

    /// The station's answer to the operator's over, or `None` when nobody
    /// answers (a CQ the station could not read a call in).
    pub fn respond(&mut self, h: &Heard, rng: &mut Rng) -> Option<String> {
        if self.phase == Phase::Done {
            return None;
        }
        self.replies += 1;
        let new_name = h.name.is_some() && self.heard.name != h.name;
        self.learn(h);
        let c = self.call.clone();

        match self.phase {
            Phase::Listening => {
                // Nobody answers a CQ they cannot read a call in.
                if !h.cq || self.you.is_none() {
                    return None;
                }
                self.phase = Phase::Answered;
                let you = self.you();
                let text = if rng.chance(0.5) {
                    format!("{you} DE {c} {c} K")
                } else {
                    format!("{you} DE {c} K")
                };
                self.last = Some(text.clone());
                return Some(text);
            }
            Phase::Calling if self.you.is_none() => {
                // Silence or AGN: call again. Something unreadable: QRZ?
                let text = if (h.words == 0 && h.undecoded == 0) || h.repeat {
                    self.last.clone().unwrap_or_default()
                } else {
                    format!("QRZ? DE {c} K")
                };
                return Some(text);
            }
            _ => {}
        }

        let recognised = h.cq
            || h.to_call.is_some()
            || h.from_call.is_some()
            || !h.calls.is_empty()
            || h.rst.is_some()
            || h.name.is_some()
            || h.qth.is_some()
            || h.rig.is_some()
            || h.power.is_some()
            || h.antenna.is_some()
            || h.weather.is_some()
            || !h.questions.is_empty()
            || h.closing
            || h.acknowledged
            || h.qrs
            || h.qrq;
        let you = self.you();
        if h.repeat && !recognised {
            if let Some(last) = &self.last {
                return Some(last.clone());
            }
        }
        if !recognised {
            return Some(format!("{you} DE {c} PSE AGN <KN>"));
        }

        let mut parts = vec![format!("{you} DE {c}")];
        if h.to_call.as_ref().is_some_and(|t| *t != c) {
            parts.push(format!("MY CALL {c} {c}"));
        }
        if h.qrs {
            parts.push("QRS".to_string());
        }
        if h.qrq {
            parts.push("QRQ".to_string());
        }
        let name = self.heard.name.clone();
        if self.phase == Phase::Calling {
            parts.push(format!("{} TNX FER CALL <BT>", rng.pick(GREETINGS)));
        } else {
            parts.push(rng.pick(&["R", "R R", "RR"]).to_string());
            if let (true, Some(n)) = (new_name, &name) {
                parts.push(format!("{} {n}", rng.pick(&["TNX", "FB", "GE"])));
            }
        }
        if !h.questions.is_empty() {
            let answers: Vec<String> = h.questions.iter().map(|q| self.answer(*q, h)).collect();
            parts.push(answers.join(" <BT> "));
            parts.push("<BT>".to_string());
        }

        let closing = h.closing || self.phase == Phase::Closing;
        if closing && (h.questions.is_empty() || h.closing) {
            let close = rng.pick(CLOSES);
            let thanks = match &name {
                Some(n) => format!("TNX FER FB QSO {n}"),
                None => "TNX FER FB QSO".to_string(),
            };
            parts.push(format!("{thanks} <BT> {close} <SK> {you} DE {c} TU E E"));
            self.phase = Phase::Done;
        } else if !self.told_exchange {
            parts.push(self.exchange());
            self.told_exchange = true;
            self.phase = Phase::Exchange;
            parts.push(format!("<AR> {you} DE {c} <KN>"));
        } else {
            let wanted: Vec<Topic> = self
                .missing()
                .into_iter()
                .filter(|t| !self.asked.contains(t))
                .collect();
            if !wanted.is_empty() {
                let words: Vec<&str> = wanted
                    .iter()
                    .map(|t| match t {
                        Topic::Rst => "RST",
                        Topic::Name => "NAME",
                        _ => "QTH",
                    })
                    .collect();
                parts.push(format!("PSE UR {}?", words.join(" ES ")));
                self.asked.extend(wanted);
            } else if !self.told_chat {
                parts.push(self.chat());
                self.told_chat = true;
                self.phase = Phase::Chat;
            } else if h.questions.is_empty() || self.replies >= MAX_REPLIES {
                let thanks = match &name {
                    Some(n) => format!("TNX FER QSO {n}"),
                    None => "TNX FER QSO".to_string(),
                };
                parts.push(format!("{thanks} <BT> 73 ES GL"));
                self.phase = Phase::Closing;
            }
            if self.replies >= MAX_REPLIES && self.phase != Phase::Closing {
                parts.push("<BT> MUST QRT SN <BT> 73 ES GL".to_string());
                self.phase = Phase::Closing;
            }
            parts.push(format!("<AR> {you} DE {c} <KN>"));
        }
        let text = spaced(&parts);
        self.last = Some(text.clone());
        Some(text)
    }

    /// The last over, for a repeat the caller asks for.
    pub fn last(&self) -> Option<&str> {
        self.last.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use cat_morse::{generate, Callsign, ScenarioConfig, ScenarioKind};

    use super::*;
    use crate::converse::heard::hear;

    fn station(listening: bool) -> (Station, Rng) {
        let script = generate(&ScenarioConfig {
            kind: ScenarioKind::RagChew,
            my_call: Callsign::parse("KF0UWV").unwrap(),
            difficulty: cat_morse::scenario::Difficulty::beginner(),
            seed: 21,
        })
        .unwrap();
        let st = script.stations[0].clone();
        let mut rng = Rng::new(3);
        let s = Station::new(st.persona, st.signal, listening, &mut rng);
        (s, rng)
    }

    #[test]
    fn it_answers_a_reply_to_its_cq_with_the_exchange() {
        let (mut s, mut rng) = station(false);
        let c = s.call().to_string();
        assert!(s.cq(&mut rng).contains(&format!("DE {c}")));
        assert_eq!(s.hint(), Hint::AnswerCq);

        let text = s
            .respond(&hear(&format!("{c} DE KF0UWV KF0UWV K")), &mut rng)
            .unwrap();
        assert!(text.starts_with(&format!("KF0UWV DE {c}")), "{text}");
        assert!(text.contains("TNX FER CALL"), "{text}");
        assert!(
            text.contains(&format!("NAME {n} {n}", n = s.persona.name)),
            "{text}"
        );
        assert!(text.ends_with(&format!("KF0UWV DE {c} <KN>")), "{text}");
        assert_eq!(s.phase(), Phase::Exchange);
        assert_eq!(s.hint(), Hint::Exchange);
        assert_eq!(s.heard_call(), Some("KF0UWV"));
    }

    #[test]
    fn it_asks_once_for_what_it_missed_then_moves_on() {
        let (mut s, mut rng) = station(false);
        let c = s.call().to_string();
        s.cq(&mut rng);
        s.respond(&hear(&format!("{c} DE KF0UWV K")), &mut rng);
        let text = s
            .respond(&hear("R TNX UR 5NN NAME MATT MATT KN"), &mut rng)
            .unwrap();
        assert!(text.contains("PSE UR QTH?"), "{text}");
        assert!(
            text.contains("TNX MATT") || text.contains("FB MATT") || text.contains("GE MATT"),
            "{text}"
        );
        // Not asked again: the chat follows.
        let text = s.respond(&hear("R R KN"), &mut rng).unwrap();
        assert!(!text.contains("PSE UR"), "{text}");
        assert!(text.contains("RIG "), "{text}");
        assert_eq!(s.phase(), Phase::Chat);
    }

    #[test]
    fn questions_are_answered_and_agn_repeats() {
        let (mut s, mut rng) = station(false);
        let c = s.call().to_string();
        s.cq(&mut rng);
        let first = s
            .respond(&hear(&format!("{c} DE KF0UWV K")), &mut rng)
            .unwrap();
        assert_eq!(s.respond(&hear("AGN?"), &mut rng).unwrap(), first);
        let text = s.respond(&hear("NAME? QTH?"), &mut rng).unwrap();
        assert!(
            text.contains(&format!("NAME {n} {n}", n = s.persona.name)),
            "{text}"
        );
        assert!(text.contains("QTH "), "{text}");
    }

    #[test]
    fn garble_gets_pse_agn_and_a_wrong_call_is_corrected() {
        let (mut s, mut rng) = station(false);
        let c = s.call().to_string();
        s.cq(&mut rng);
        assert_eq!(
            s.respond(&hear("[.-.-.-.] [..--]"), &mut rng).unwrap(),
            format!("QRZ? DE {c} K")
        );
        let text = s.respond(&hear("W9XYZ DE KF0UWV K"), &mut rng).unwrap();
        assert!(text.contains(&format!("MY CALL {c} {c}")), "{text}");
        let text = s.respond(&hear("[..-..]"), &mut rng).unwrap();
        assert_eq!(text, format!("KF0UWV DE {c} PSE AGN <KN>"));
    }

    #[test]
    fn it_answers_the_operators_cq_and_follows_a_closing() {
        let (mut s, mut rng) = station(true);
        let c = s.call().to_string();
        assert_eq!(s.hint(), Hint::CallCq);
        assert_eq!(
            s.respond(&hear("CQ CQ K"), &mut rng),
            None,
            "no call, no answer"
        );
        let text = s
            .respond(&hear("CQ CQ DE KF0UWV KF0UWV K"), &mut rng)
            .unwrap();
        assert!(text.starts_with(&format!("KF0UWV DE {c}")), "{text}");
        assert_eq!(s.hint(), Hint::Exchange);
        s.respond(
            &hear(&format!("{c} DE KF0UWV TNX UR 579 NAME MATT QTH DENVER KN")),
            &mut rng,
        );
        let text = s
            .respond(&hear("TNX FER QSO 73 <SK> E E"), &mut rng)
            .unwrap();
        assert!(text.contains("<SK>"), "{text}");
        assert!(text.contains("MATT"), "{text}");
        assert_eq!(s.phase(), Phase::Done);
        assert_eq!(s.respond(&hear("73"), &mut rng), None);
    }

    #[test]
    fn it_closes_once_everything_is_said() {
        let (mut s, mut rng) = station(false);
        let c = s.call().to_string();
        s.cq(&mut rng);
        s.respond(&hear(&format!("{c} DE KF0UWV K")), &mut rng);
        s.respond(&hear("R UR 599 NAME MATT QTH DENVER KN"), &mut rng);
        assert_eq!(s.phase(), Phase::Chat);
        let text = s
            .respond(&hear("FB RIG KX3 PWR 5W ANT EFHW KN"), &mut rng)
            .unwrap();
        assert!(text.contains("73"), "{text}");
        assert_eq!(s.phase(), Phase::Closing);
        assert_eq!(s.hint(), Hint::Close);
        let text = s.respond(&hear("TU 73 SK"), &mut rng).unwrap();
        assert!(text.contains("<SK>"), "{text}");
        assert_eq!(s.phase(), Phase::Done);
    }
}
