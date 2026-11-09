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

//! What the other station makes of an operator's over: free text, as a
//! Morse decoder writes it, read for calls, the report, name, QTH, rig,
//! power, antenna and weather, questions, repeat and speed requests, a
//! closing and the hand-over.
//!
//! It reads the way an operator copies: by keyword and position, never
//! demanding a fixed wording. `NAME BOB BOB`, `OP HR BOB` and `NM BOB`
//! all give the name BOB; `5NN` is a report of 599.

use cat_morse::Callsign;
use serde::{Deserialize, Serialize};

/// Something asked about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    Call,
    Rst,
    Name,
    Qth,
    Rig,
    Power,
    Antenna,
    Weather,
    /// `HW?`, `HW CPY?`: how is the copy.
    Copy,
}

/// How an over hands back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Handover {
    /// `K`: anyone (or you) go ahead.
    K,
    /// `KN`, `<KN>`: only the station worked.
    Kn,
    /// `BK`: a quick break.
    Bk,
    /// `<AR>`, `AR`: end of message.
    Ar,
    /// `<SK>`, `SK`: end of contact.
    Sk,
}

/// What was heard in one over. Every field is optional: an operator
/// sends what they choose.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Heard {
    /// The over as read: upper case, one space between tokens.
    pub text: String,
    /// `CQ` was sent.
    pub cq: bool,
    /// The call before `DE`: who the over is for.
    pub to_call: Option<String>,
    /// The call after `DE`: who sent it.
    pub from_call: Option<String>,
    /// Every call heard outside a field, in order, each once.
    pub calls: Vec<String>,
    /// A signal report, as digits (`5NN` reads `599`).
    pub rst: Option<String>,
    pub name: Option<String>,
    pub qth: Option<String>,
    pub rig: Option<String>,
    pub power: Option<String>,
    pub antenna: Option<String>,
    pub weather: Option<String>,
    /// Asked about, in order, each once.
    pub questions: Vec<Topic>,
    /// `AGN`, `PSE RPT`, a lone `?`: repeat the last over.
    pub repeat: bool,
    pub qrs: bool,
    pub qrq: bool,
    /// `73`, `CUL`, `<SK>`, `TNX FER QSO`, ...: the operator is closing.
    pub closing: bool,
    /// `R`, `RR`, `FB`, `TNX`, `QSL`: the last over was received.
    pub acknowledged: bool,
    /// The words after `TNX`, `FB`, `DR`, `GM`, ... that are not
    /// abbreviations: the name the operator called the station by.
    pub addressed: Vec<String>,
    pub handover: Option<Handover>,
    /// Characters the decoder could not read (`[.-..-]`).
    pub undecoded: u32,
    /// Tokens read, undecoded ones not counted.
    pub words: u32,
}

/// Keywords that end a field's value, and words that are never a name.
const KEYWORDS: &[&str] = &[
    "NAME", "OP", "NM", "QTH", "LOC", "RIG", "ANT", "PWR", "WX", "RST", "UR", "HW", "TNX", "TKS",
    "FB", "R", "RR", "ES", "DE", "73", "TU", "CQ", "K", "KN", "BK", "AR", "SK", "HR", "IS", "FER",
    "FOR", "PSE", "AGN", "RPT", "QRS", "QRQ", "QSL", "QSO", "OM", "YL", "DR", "GM", "GA", "GE",
    "GD", "GN", "HI", "CALL", "CUL", "GL", "DX", "BT", "SOLID", "CPY", "ALL", "OK", "INFO", "MY",
    "NR", "CL", "QRT", "GB", "VY", "GUD", "E", "EE",
];

const PROSIGNS: &[&str] = &["<BT>", "<AR>", "<KN>", "<SK>", "<AS>", "<HH>", "<SOS>"];

/// Field markers, and the field each starts.
const MARKERS: &[(&str, Field)] = &[
    ("NAME", Field::Name),
    ("OP", Field::Name),
    ("NM", Field::Name),
    ("QTH", Field::Qth),
    ("LOC", Field::Qth),
    ("RIG", Field::Rig),
    ("ANT", Field::Antenna),
    ("PWR", Field::Power),
    ("WX", Field::Weather),
];

/// Words that sit between a marker and its value (`NAME HR BOB`).
const FILLERS: &[&str] = &["HR", "IS", "HERE", "=", "<BT>"];

/// The longest value a field takes, in tokens (`EFHW AT 30 FT`).
const MAX_VALUE: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Qth,
    Rig,
    Antenna,
    Power,
    Weather,
}

fn is_keyword(t: &str) -> bool {
    KEYWORDS.contains(&t) || PROSIGNS.contains(&t)
}

fn topic_of(word: &str) -> Option<Topic> {
    Some(match word {
        "NAME" | "NM" | "OP" => Topic::Name,
        "QTH" | "LOC" => Topic::Qth,
        "RIG" => Topic::Rig,
        "ANT" => Topic::Antenna,
        "PWR" => Topic::Power,
        "WX" => Topic::Weather,
        "RST" | "RPRT" => Topic::Rst,
        "HW" | "CPY" | "HW CPY" => Topic::Copy,
        "CALL" | "UR CALL" => Topic::Call,
        _ => return None,
    })
}

/// A digit of a report, cut numbers included (`N` for 9, `T` for 0, ...).
fn cut_digit(c: char) -> Option<char> {
    Some(match c {
        '0'..='9' => c,
        'T' | 'O' => '0',
        'A' => '1',
        'U' => '2',
        'V' => '3',
        'E' => '5',
        'B' => '7',
        'D' => '8',
        'N' => '9',
        _ => return None,
    })
}

/// A readability-strength-tone report: `[1-5][1-9][1-9]`, cut numbers
/// allowed after the first digit, as digits.
pub fn report(token: &str) -> Option<String> {
    let cs: Vec<char> = token.chars().collect();
    if cs.len() != 3 || !('1'..='5').contains(&cs[0]) {
        return None;
    }
    let mut out = String::from(cs[0]);
    for &c in &cs[1..] {
        let d = cut_digit(c)?;
        if d == '0' {
            return None;
        }
        out.push(d);
    }
    Some(out)
}

/// A number with a unit (`100W`, `72F`, `20C`, `40M`): never a call.
fn is_quantity(token: &str) -> bool {
    let digits = token.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && {
        let unit = &token[digits..];
        matches!(unit, "W" | "F" | "C" | "M" | "FT" | "WPM" | "KW" | "MHZ")
    }
}

/// A token that reads as a call: letters and a digit, valid to cat-morse,
/// and neither a report nor a quantity.
pub fn call(token: &str) -> Option<String> {
    let has_letter = token.chars().any(|c| c.is_ascii_alphabetic());
    let has_digit = token.chars().any(|c| c.is_ascii_digit());
    if !has_letter || !has_digit || report(token).is_some() || is_quantity(token) {
        return None;
    }
    Callsign::parse(token).ok().map(|c| c.as_str().to_string())
}

/// Split decoded text into tokens: upper case, prosigns apart, `=` `+`
/// `(` as the prosigns they stand for, commas and stops dropped, and
/// undecoded `[...]` groups counted and removed.
fn tokenize(text: &str) -> (Vec<String>, u32) {
    let mut out = Vec::new();
    let mut undecoded = 0;
    let upper = text.to_ascii_uppercase();
    for piece in upper.split_whitespace() {
        let mut word = String::new();
        let mut chars = piece.chars().peekable();
        let flush = |word: &mut String, out: &mut Vec<String>| {
            let w = word.trim_matches(|c| c == ',' || c == '.').to_string();
            if !w.is_empty() {
                out.push(w);
            }
            word.clear();
        };
        while let Some(c) = chars.next() {
            match c {
                '[' => {
                    // An unread character: a word around it stays one
                    // (damaged) word, so `KF0[..-.]UWV` is never read as
                    // the call `KF0`.
                    undecoded += 1;
                    for d in chars.by_ref() {
                        if d == ']' {
                            break;
                        }
                    }
                    if !(word.is_empty() && chars.peek().is_none()) {
                        word.push('*');
                    }
                }
                '<' => {
                    flush(&mut word, &mut out);
                    let mut p = String::from('<');
                    for d in chars.by_ref() {
                        p.push(d);
                        if d == '>' {
                            break;
                        }
                    }
                    out.push(p);
                }
                '=' => {
                    flush(&mut word, &mut out);
                    out.push("<BT>".to_string());
                }
                '+' => {
                    flush(&mut word, &mut out);
                    out.push("<AR>".to_string());
                }
                '(' => {
                    flush(&mut word, &mut out);
                    out.push("<KN>".to_string());
                }
                _ => word.push(c),
            }
        }
        flush(&mut word, &mut out);
    }
    (out, undecoded)
}

/// `BOB BOB` is `BOB`, `DENVER CO DENVER CO` is `DENVER CO`: operators
/// send a field twice or three times to be sure.
fn collapse(tokens: &[String]) -> Vec<String> {
    let n = tokens.len();
    for p in 1..n {
        if n % p == 0 && (p..n).all(|i| tokens[i] == tokens[i % p]) {
            return tokens[..p].to_vec();
        }
    }
    tokens.to_vec()
}

fn push_once<T: PartialEq>(v: &mut Vec<T>, x: T) {
    if !v.contains(&x) {
        v.push(x);
    }
}

/// Read one over.
pub fn hear(text: &str) -> Heard {
    let (t, undecoded) = tokenize(text);
    let mut h = Heard {
        text: t.join(" "),
        undecoded,
        words: t.len() as u32,
        ..Heard::default()
    };
    // Tokens a field took, so a rig `IC7300` is never read as a call.
    let mut used = vec![false; t.len()];

    // Questions and requests first: `NAME?` and `NAME ?` ask, they do
    // not start a field.
    let mut i = 0;
    while i < t.len() {
        let w = t[i].as_str();
        let next = t.get(i + 1).map(String::as_str);
        if let Some(base) = w.strip_suffix('?').filter(|b| !b.is_empty()) {
            used[i] = true;
            match base {
                "AGN" | "RPT" => h.repeat = true,
                "QRS" => h.qrs = true,
                "QRQ" => h.qrq = true,
                _ => {
                    let two = i
                        .checked_sub(1)
                        .map(|j| format!("{} {base}", t[j]))
                        .and_then(|s| topic_of(&s));
                    if let Some(q) = two.or_else(|| topic_of(base)) {
                        push_once(&mut h.questions, q);
                    }
                }
            }
        } else if w == "?" {
            used[i] = true;
            let before = i.checked_sub(1).map(|j| t[j].as_str());
            match before.and_then(topic_of) {
                Some(q) => {
                    used[i - 1] = true;
                    push_once(&mut h.questions, q);
                }
                None => h.repeat = true,
            }
        } else if (w == "PSE" && matches!(next, Some("RPT" | "AGN"))) || w == "AGN" {
            // `PSE RPT NAME` asks for the name; `PSE AGN` alone, for it all.
            let at = if w == "PSE" { i + 1 } else { i };
            used[i] = true;
            used[at] = true;
            match t
                .get(at + 1)
                .and_then(|n| topic_of(n.trim_end_matches('?')))
            {
                Some(q) => {
                    used[at + 1] = true;
                    push_once(&mut h.questions, q);
                    i = at + 1;
                }
                None => {
                    h.repeat = true;
                    i = at;
                }
            }
        }
        i += 1;
    }

    // Fields: a marker, maybe a filler, then words up to the next keyword.
    let mut i = 0;
    while i < t.len() {
        let field = MARKERS
            .iter()
            .find(|(m, _)| *m == t[i])
            .map(|(_, f)| *f)
            .filter(|_| !used[i]);
        let Some(field) = field else {
            i += 1;
            continue;
        };
        used[i] = true;
        let mut j = i + 1;
        while j < t.len() && FILLERS.contains(&t[j].as_str()) {
            used[j] = true;
            j += 1;
        }
        let start = j;
        while j < t.len()
            && j - start < MAX_VALUE
            && !used[j]
            && !(is_keyword(&t[j]) || MARKERS.iter().any(|(m, _)| *m == t[j]))
        {
            used[j] = true;
            j += 1;
        }
        let value = collapse(&t[start..j]);
        if !value.is_empty() {
            let joined = value.join(" ");
            let slot = match field {
                Field::Name => &mut h.name,
                Field::Qth => &mut h.qth,
                Field::Rig => &mut h.rig,
                Field::Antenna => &mut h.antenna,
                Field::Power => &mut h.power,
                Field::Weather => &mut h.weather,
            };
            if slot.is_none() {
                *slot = Some(if field == Field::Name {
                    value[0].clone()
                } else {
                    joined
                });
            }
        }
        i = j.max(i + 1);
    }

    // A bare power (`100W`) is a power.
    if h.power.is_none() {
        if let Some(k) =
            (0..t.len()).find(|&k| !used[k] && t[k].ends_with('W') && is_quantity(&t[k]))
        {
            used[k] = true;
            h.power = Some(t[k].clone());
        }
    }

    // The report: after `RST`/`RPT`, else any report-shaped word.
    let after_rst = (0..t.len())
        .filter(|&k| matches!(t[k].as_str(), "RST" | "RPT" | "RPRT"))
        .flat_map(|k| (k + 1)..(k + 3).min(t.len()))
        .find(|&k| !used[k] && report(&t[k]).is_some());
    let anywhere = || (0..t.len()).find(|&k| !used[k] && report(&t[k]).is_some());
    if let Some(k) = after_rst.or_else(anywhere) {
        used[k] = true;
        h.rst = report(&t[k]);
    }

    // Calls: around `DE`, then any other call-shaped word left over.
    for k in 0..t.len() {
        if t[k] != "DE" {
            continue;
        }
        if h.to_call.is_none() {
            if let Some(c) = k
                .checked_sub(1)
                .filter(|&j| !used[j])
                .and_then(|j| call(&t[j]))
            {
                h.to_call = Some(c);
            }
        }
        if h.from_call.is_none() {
            if let Some(c) = t.get(k + 1).filter(|_| !used[k + 1]).and_then(|w| call(w)) {
                h.from_call = Some(c);
            }
        }
    }
    for k in 0..t.len() {
        if !used[k] {
            if let Some(c) = call(&t[k]) {
                push_once(&mut h.calls, c);
            }
        }
    }

    // Everything else is one word at a time.
    for k in 0..t.len() {
        let w = t[k].as_str();
        match w {
            "CQ" => h.cq = true,
            "QRS" => h.qrs = true,
            "QRQ" => h.qrq = true,
            "73" | "CUL" | "QRT" | "CL" | "GB" | "GN" | "<SK>" | "SK" => h.closing = true,
            "R" | "RR" | "FB" | "TNX" | "TKS" | "QSL" | "TU" => h.acknowledged = true,
            _ => {}
        }
        if w == "QSO" && k > 0 && matches!(t[k - 1].as_str(), "TNX" | "TKS" | "FER" | "FB") {
            h.closing = true;
        }
        if matches!(
            w,
            "TNX" | "TKS" | "FB" | "DR" | "GM" | "GA" | "GE" | "GD" | "HI"
        ) {
            if let Some(n) = t.get(k + 1) {
                if !used[k + 1]
                    && !is_keyword(n)
                    && (2..=10).contains(&n.len())
                    && n.chars().all(|c| c.is_ascii_alphabetic())
                {
                    push_once(&mut h.addressed, n.clone());
                }
            }
        }
    }

    // The hand-over is the last word, past any closing `E E`.
    h.handover = t
        .iter()
        .rev()
        .map(String::as_str)
        .find(|w| !matches!(*w, "E" | "EE"))
        .and_then(|w| match w {
            "K" => Some(Handover::K),
            "KN" | "<KN>" => Some(Handover::Kn),
            "BK" => Some(Handover::Bk),
            "AR" | "<AR>" => Some(Handover::Ar),
            "SK" | "<SK>" => Some(Handover::Sk),
            _ => None,
        })
        // `<SK> W1AW DE KF0UWV TU E E` ends the contact too.
        .or_else(|| {
            t.iter()
                .any(|w| w == "<SK>" || w == "SK")
                .then_some(Handover::Sk)
        });
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> Option<String> {
        Some(x.to_string())
    }

    #[test]
    fn an_answer_to_a_cq_names_both_calls() {
        let h = hear("w1aw de kf0uwv kf0uwv k");
        assert_eq!(h.to_call, s("W1AW"));
        assert_eq!(h.from_call, s("KF0UWV"));
        assert_eq!(h.calls, vec!["W1AW", "KF0UWV"]);
        assert_eq!(h.handover, Some(Handover::K));
        assert!(!h.cq);
        assert_eq!(h.text, "W1AW DE KF0UWV KF0UWV K");
    }

    #[test]
    fn a_cq_is_a_cq_with_its_call() {
        let h = hear("CQ CQ CQ DE KF0UWV KF0UWV K");
        assert!(h.cq);
        assert_eq!(h.to_call, None);
        assert_eq!(h.from_call, s("KF0UWV"));
    }

    #[test]
    fn the_first_exchange_in_its_usual_shapes() {
        let h = hear(
            "W1AW DE KF0UWV R TNX BOB <BT> UR RST 5NN 5NN <BT> NAME MATT MATT <BT> \
             QTH DENVER, CO DENVER, CO <BT> HW? <AR> W1AW DE KF0UWV <KN>",
        );
        assert_eq!(h.rst, s("599"));
        assert_eq!(h.name, s("MATT"));
        assert_eq!(h.qth, s("DENVER CO"));
        assert_eq!(h.questions, vec![Topic::Copy]);
        assert_eq!(h.addressed, vec!["BOB"]);
        assert!(h.acknowledged);
        assert!(!h.closing);
        assert_eq!(h.handover, Some(Handover::Kn));

        for (text, name) in [
            ("OP HR BOB", "BOB"),
            ("NM ED ED", "ED"),
            ("NAME IS SUE", "SUE"),
            ("MY NAME = JIM JIM JIM", "JIM"),
        ] {
            assert_eq!(hear(text).name, s(name), "{text}");
        }
        for (text, rst) in [
            ("RST 579", "579"),
            ("UR 5NN", "599"),
            ("RPT 4N9", "499"),
            ("559", "559"),
        ] {
            assert_eq!(hear(text).rst, s(rst), "{text}");
        }
        assert_eq!(hear("UR RST 6NN").rst, None, "readability is 1 to 5");
    }

    #[test]
    fn the_chat_fields() {
        let h = hear("RIG IC7300 PWR 100W ANT DIPOLE AT 30 FT <BT> WX SUNNY 72F <BT>");
        assert_eq!(h.rig, s("IC7300"));
        assert_eq!(h.power, s("100W"));
        assert_eq!(h.antenna, s("DIPOLE AT 30 FT"));
        assert_eq!(h.weather, s("SUNNY 72F"));
        assert!(h.calls.is_empty(), "a rig is not a call: {:?}", h.calls);

        assert_eq!(hear("RIG HR KX3 ES 5W").power, s("5W"));
        assert_eq!(hear("RIG HR KX3 ES 5W").rig, s("KX3"));
    }

    #[test]
    fn questions_and_repeat_requests() {
        assert_eq!(hear("UR NAME?").questions, vec![Topic::Name]);
        assert_eq!(hear("QTH ?").questions, vec![Topic::Qth]);
        assert_eq!(hear("QTH ?").qth, None, "asking is not telling");
        assert_eq!(hear("PSE RPT NAME ES QTH").questions, vec![Topic::Name]);
        assert_eq!(hear("HW CPY?").questions, vec![Topic::Copy]);
        assert_eq!(
            hear("RIG? ANT?").questions,
            vec![Topic::Rig, Topic::Antenna]
        );
        for text in ["AGN", "PSE AGN", "?", "PSE RPT K", "AGN?"] {
            let h = hear(text);
            assert!(h.repeat, "{text}");
            assert!(h.questions.is_empty(), "{text}");
        }
        assert!(hear("PSE QRS").qrs);
        assert!(hear("QRQ?").qrq);
    }

    #[test]
    fn a_closing() {
        let h = hear("R TNX FER FB QSO BOB <BT> 73 ES CUL <SK> W1AW DE KF0UWV TU E E");
        assert!(h.closing);
        assert_eq!(h.handover, Some(Handover::Sk));
        assert!(hear("TNX QSO").closing);
        assert!(hear("73").closing);
        assert!(!hear("TNX FER CALL").closing);
        assert_eq!(hear("<SK> E E").handover, Some(Handover::Sk));
    }

    #[test]
    fn undecoded_characters_are_counted_and_left_out() {
        let h = hear("W1AW DE KF0[..-.]UWV [-.-.-] K");
        assert_eq!(h.undecoded, 2);
        assert_eq!(h.text, "W1AW DE KF0*UWV K");
        assert_eq!(h.from_call, None, "a broken call is not a call");
        assert_eq!(h.to_call, s("W1AW"));
    }

    #[test]
    fn prosign_spellings_and_glued_prosigns() {
        let h = hear("NAME BOB = QTH ERIE + W1AW DE KF0UWV (");
        assert_eq!(h.name, s("BOB"));
        assert_eq!(h.qth, s("ERIE"));
        assert_eq!(h.handover, Some(Handover::Kn));
        assert_eq!(hear("TU<SK>").handover, Some(Handover::Sk));
    }

    #[test]
    fn nothing_is_nothing() {
        let h = hear("   ");
        assert_eq!(h, Heard::default());
    }

    #[test]
    fn reports_and_calls_are_told_apart() {
        assert_eq!(report("5NN"), s("599"));
        assert_eq!(report("5TT"), None, "no zero in a report");
        assert_eq!(report("599"), s("599"));
        assert_eq!(call("5NN"), None);
        assert_eq!(call("100W"), None);
        assert_eq!(call("20C"), None);
        assert_eq!(call("K1ABC/P"), s("K1ABC/P"));
        assert_eq!(call("BOB"), None);
    }
}
