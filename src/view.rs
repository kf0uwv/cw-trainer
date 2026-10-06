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

//! What the operator reads: pure text, so it can be tested.
//!
//! Every line ends `\r\n` because the terminal is in raw mode.

use cat_morse::score::AlignOp;
use cat_morse::{Alignment, CharStats, Score, Token, UnorderedAlignment};

use super::session::SessionRecord;

pub const NL: &str = "\r\n";

fn unit(t: Token) -> String {
    match t {
        Token::WordSpace => " ".to_string(),
        other => other.to_string(),
    }
}

/// Three rows — sent, copied, markers — with each step in its own column.
///
/// A miss shows `_` in the copied row, an extra `_` in the sent row, and
/// every error a `^` beneath it. Prosigns (`<AR>`) take their own width.
pub fn alignment_rows(a: &Alignment) -> [String; 3] {
    let mut rows = [String::new(), String::new(), String::new()];
    for op in &a.ops {
        let (sent, copied, mark) = match *op {
            AlignOp::Match { expected } => (unit(expected), unit(expected), false),
            AlignOp::Substitution { expected, copied } => (unit(expected), unit(copied), true),
            AlignOp::Deletion { expected } => (unit(expected), "_".to_string(), true),
            AlignOp::Insertion { copied, .. } => ("_".to_string(), unit(copied), true),
        };
        let w = sent.chars().count().max(copied.chars().count());
        rows[0].push_str(&format!("{sent:<w$}"));
        rows[1].push_str(&format!("{copied:<w$}"));
        rows[2].push_str(&if mark { "^".repeat(w) } else { " ".repeat(w) });
    }
    for r in &mut rows {
        let trimmed = r.trim_end().len();
        r.truncate(trimmed);
    }
    rows
}

fn percent(x: f64) -> String {
    format!("{:.0}%", x * 100.0)
}

fn score_line(s: &Score) -> String {
    format!(
        "CER {} ({} sent: {} ok, {} wrong, {} missed, {} extra)",
        percent(s.cer()),
        s.expected,
        s.matches,
        s.substitutions,
        s.deletions,
        s.insertions
    )
}

fn push_rows(out: &mut String, label: &str, a: &Alignment) {
    let [sent, copied, marks] = alignment_rows(a);
    let pad = " ".repeat(label.len());
    out.push_str(&format!("  {label}sent   {sent}{NL}"));
    out.push_str(&format!("  {pad}copied {copied}{NL}"));
    if !marks.is_empty() {
        out.push_str(&format!("  {pad}       {marks}{NL}"));
    }
}

/// The result of an ordinary over.
pub fn over_result(a: &Alignment, agn: u32) -> String {
    let mut out = String::new();
    push_rows(&mut out, "", a);
    out.push_str(&format!(
        "  {}{}{NL}",
        score_line(&a.score()),
        agn_note(agn)
    ));
    out
}

/// The result of an over several stations sent at once.
pub fn unordered_result(u: &UnorderedAlignment, agn: u32) -> String {
    let mut out = String::new();
    for (i, a) in u.items.iter().enumerate() {
        push_rows(&mut out, &format!("#{} ", i + 1), a);
    }
    for a in &u.extras {
        push_rows(&mut out, "+  ", a);
    }
    out.push_str(&format!(
        "  {}{}{NL}",
        score_line(&u.score()),
        agn_note(agn)
    ));
    out
}

fn agn_note(agn: u32) -> String {
    match agn {
        0 => String::new(),
        1 => ", 1 AGN".to_string(),
        n => format!(", {n} AGN"),
    }
}

fn weakest_line(stats: &CharStats) -> String {
    let w = stats.weakest(5);
    if w.is_empty() {
        return "none yet".to_string();
    }
    w.iter()
        .map(|(u, acc)| format!("{} {}", unit(*u), percent(*acc)))
        .collect::<Vec<_>>()
        .join("  ")
}

/// One line recalling the previous session.
pub fn last_session(r: &SessionRecord) -> String {
    format!(
        "Last session {}: {} overs, CER {}, ended at {}{NL}",
        r.started,
        r.overs.iter().filter(|o| !o.skipped).count(),
        percent(r.total.cer()),
        r.end_speed.describe()
    )
}

/// The end-of-session summary.
pub fn summary(r: &SessionRecord, session: &CharStats, all_time: &CharStats) -> String {
    let scored = r.overs.iter().filter(|o| !o.skipped).count();
    let skipped = r.overs.len() - scored;
    let agn: u32 = r.overs.iter().map(|o| o.agn).sum();
    let mut out = String::new();
    out.push_str(&format!(
        "{NL}=== Session summary (seed {}) ==={NL}",
        r.seed
    ));
    out.push_str(&format!(
        "  overs copied {scored}, skipped {skipped}, AGN {agn}{}{NL}",
        if r.quit_early { ", quit early" } else { "" }
    ));
    out.push_str(&format!("  {}{NL}", score_line(&r.total)));
    out.push_str(&format!(
        "  speed {} -> {}{NL}",
        r.start_speed.describe(),
        r.end_speed.describe()
    ));
    out.push_str(&format!(
        "  weakest this session: {}{NL}",
        weakest_line(session)
    ));
    out.push_str(&format!(
        "  weakest all time:     {}{NL}",
        weakest_line(all_time)
    ));
    out
}

#[cfg(test)]
mod tests {
    use cat_morse::{align, ScoreOptions};

    use super::*;

    #[test]
    fn a_perfect_copy_has_no_markers() {
        let a = align("CQ DE W1AW", "CQ DE W1AW", &ScoreOptions::default()).unwrap();
        let [sent, copied, marks] = alignment_rows(&a);
        assert_eq!(sent, "CQ DE W1AW");
        assert_eq!(copied, "CQ DE W1AW");
        assert_eq!(marks, "");
    }

    #[test]
    fn substitutions_misses_and_extras_are_marked_in_their_column() {
        let a = align("W1AW", "W1QW", &ScoreOptions::default()).unwrap();
        assert_eq!(
            alignment_rows(&a),
            ["W1AW".to_string(), "W1QW".to_string(), "  ^".to_string()]
        );

        let a = align("W1AW", "W1W", &ScoreOptions::default()).unwrap();
        let [sent, copied, marks] = alignment_rows(&a);
        assert_eq!(sent, "W1AW");
        assert_eq!(copied, "W1_W");
        assert_eq!(marks, "  ^");

        let a = align("W1AW", "W1AWX", &ScoreOptions::default()).unwrap();
        let [sent, copied, marks] = alignment_rows(&a);
        assert_eq!(sent, "W1AW_");
        assert_eq!(copied, "W1AWX");
        assert_eq!(marks, "    ^");
    }

    #[test]
    fn a_prosign_keeps_its_columns_aligned() {
        let a = align("73 <SK>", "73 E", &ScoreOptions::default()).unwrap();
        let [sent, copied, marks] = alignment_rows(&a);
        assert_eq!(sent, "73 <SK>");
        assert_eq!(copied, "73 E");
        assert_eq!(marks, "   ^^^^");
    }

    #[test]
    fn an_over_result_states_the_cer_and_any_agn() {
        let a = align("TU 5NN", "TU 5N", &ScoreOptions::default()).unwrap();
        let s = over_result(&a, 2);
        assert!(s.contains("CER 17%"), "{s}");
        assert!(s.contains("2 AGN"), "{s}");
        assert!(s.ends_with(NL));
    }
}
