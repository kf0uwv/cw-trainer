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

//! Which sound card the trainer plays to — chosen, never assumed.
//!
//! # Why there is no default
//!
//! On a station the system's default output can be **the radio's own
//! audio interface**: a USB sound device wired to ACC2. Audio played there
//! goes into the transmitter's PKD input, and with VOX on it can key the
//! radio. That happened on this project's own bench (the default output
//! was the C-Media "USB PnP Sound Device" on ACC2). "Zero RF" has to cover
//! the speaker as well as CAT, so:
//!
//! - `cw-trainer copy` never plays to the system default. The device is named with
//!   `--audio-out audio:<name>` once, then remembered in `audio.json` in the
//!   trainer's data directory. `audio:` alone (the default) is refused.
//! - A deny list (`--deny-audio <text>`, remembered the same way) refuses
//!   any device whose name contains that text, whatever was asked for.
//!   Put the radio's interface on it once and it can never be picked by
//!   accident again.
//! - `cw-trainer devices` lists the outputs, marking the remembered and the
//!   denied ones.

use cat_signal::DeviceList;
use serde::{Deserialize, Serialize};

/// The warning printed wherever an output is chosen.
pub const ACC2_WARNING: &str = "Never choose the radio's own audio interface (the USB sound \
device wired to ACC2, often a C-Media \"USB PnP Sound Device\"): audio played there goes into \
the transmitter, and with VOX on it can key the radio. Deny it once with --deny-audio <part of \
its name>.";

const PREFIX: &str = "audio:";

/// What is remembered about audio output, in `audio.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioPrefs {
    /// The chosen output, as an `audio:<name>` spec.
    pub device: Option<String>,
    /// Name fragments (case-insensitive) that are never played to.
    pub deny: Vec<String>,
}

impl AudioPrefs {
    /// Add and remove deny entries; `true` if anything changed.
    pub fn edit_deny(&mut self, add: &[String], remove: &[String]) -> bool {
        let before = self.deny.clone();
        for a in add {
            let a = a.trim();
            if !a.is_empty() && !self.deny.iter().any(|d| d.eq_ignore_ascii_case(a)) {
                self.deny.push(a.to_string());
            }
        }
        self.deny
            .retain(|d| !remove.iter().any(|r| r.trim().eq_ignore_ascii_case(d)));
        self.deny != before
    }

    /// The deny entry `name` matches, if any.
    pub fn denied_by(&self, name: &str) -> Option<&str> {
        let name = name.to_lowercase();
        self.deny
            .iter()
            .map(String::as_str)
            .find(|d| !d.is_empty() && name.contains(&d.to_lowercase()))
    }

    /// Refuse a device by the name it actually opened as.
    pub fn check_opened(&self, label: &str) -> Result<(), String> {
        match self.denied_by(label) {
            Some(d) => Err(denied_message(label, d)),
            None => Ok(()),
        }
    }
}

fn denied_message(name: &str, entry: &str) -> String {
    format!(
        "refusing to play to {name:?}: it matches the deny list entry {entry:?}. \
         Choose another output with --audio-out (see `cw-trainer devices`), or remove the \
         entry with --undeny-audio {entry:?} if it is not the radio's interface."
    )
}

/// The output spec to open: `--audio-out`, else the remembered one.
/// Never the system default.
pub fn choose(flag: Option<&str>, prefs: &AudioPrefs) -> Result<String, String> {
    let spec = flag.or(prefs.device.as_deref()).ok_or_else(|| {
        format!(
            "no audio output chosen. Pick one with --audio-out audio:<name> (it is remembered); \
             `cw-trainer devices` lists them. The system default is never used.\n{ACC2_WARNING}"
        )
    })?;
    let name = spec.strip_prefix(PREFIX).ok_or_else(|| {
        format!("--audio-out wants audio:<name>, got {spec:?} (see `cw-trainer devices`)")
    })?;
    if name.trim().is_empty() {
        return Err(format!(
            "{spec:?} means the system default output, which cw-trainer copy never uses: name the \
             device (see `cw-trainer devices`).\n{ACC2_WARNING}"
        ));
    }
    if let Some(d) = prefs.denied_by(name) {
        return Err(denied_message(name, d));
    }
    Ok(spec.to_string())
}

/// `cw-trainer devices`: every output, marking the remembered and denied.
pub fn describe_devices(list: &DeviceList, prefs: &AudioPrefs) -> String {
    let mut out = String::from("Audio outputs (use with --audio-out):\n");
    if let Some(why) = &list.error {
        out.push_str(&format!("  cannot list outputs: {why}\n"));
    } else if list.devices.is_empty() {
        out.push_str("  none found\n");
    }
    for d in &list.devices {
        let mut notes = Vec::new();
        if prefs.device.as_deref() == Some(d.spec.as_str()) {
            notes.push("chosen".to_string());
        }
        if d.is_default {
            notes.push("system default, never used automatically".to_string());
        }
        if let Some(e) = prefs.denied_by(&d.label) {
            notes.push(format!("DENIED by {e:?}"));
        }
        let notes = if notes.is_empty() {
            String::new()
        } else {
            format!("  [{}]", notes.join("; "))
        };
        out.push_str(&format!("  {}{notes}\n", d.spec));
        if let Some(detail) = &d.detail {
            out.push_str(&format!("      {detail}\n"));
        }
    }
    if !prefs.deny.is_empty() {
        out.push_str(&format!("Deny list: {}\n", prefs.deny.join(", ")));
    }
    out.push_str(&format!("\n{ACC2_WARNING}\n"));
    out
}

#[cfg(test)]
mod tests {
    use cat_signal::{DeviceInfo, DeviceKind};

    use super::*;

    fn prefs(device: Option<&str>, deny: &[&str]) -> AudioPrefs {
        AudioPrefs {
            device: device.map(String::from),
            deny: deny.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn with_nothing_chosen_there_is_no_fallback_to_the_default() {
        let e = choose(None, &AudioPrefs::default()).unwrap_err();
        assert!(e.contains("never used"), "{e}");
        assert!(e.contains("ACC2"), "{e}");
    }

    #[test]
    fn the_bare_default_spec_is_refused() {
        for spec in ["audio:", "audio:  "] {
            let e = choose(Some(spec), &AudioPrefs::default()).unwrap_err();
            assert!(e.contains("system default"), "{e}");
        }
        assert!(choose(Some("127.0.0.1:4533"), &AudioPrefs::default()).is_err());
    }

    #[test]
    fn the_remembered_choice_is_used_and_a_flag_overrides_it() {
        let p = prefs(Some("audio:Headphones"), &[]);
        assert_eq!(choose(None, &p).unwrap(), "audio:Headphones");
        assert_eq!(
            choose(Some("audio:Speakers"), &p).unwrap(),
            "audio:Speakers"
        );
    }

    #[test]
    fn a_denied_device_is_refused_however_it_was_asked_for() {
        let p = prefs(Some("audio:USB PnP Sound Device"), &["usb pnp"]);
        let e = choose(None, &p).unwrap_err();
        assert!(e.contains("deny list"), "{e}");
        let e = choose(Some("audio:Front: USB PnP Sound Device"), &p).unwrap_err();
        assert!(e.contains("\"usb pnp\""), "{e}");
        assert!(p.check_opened("C-Media USB PnP Sound Device").is_err());
        assert!(p.check_opened("Built-in Audio").is_ok());
    }

    #[test]
    fn deny_edits_are_case_insensitive_and_idempotent() {
        let mut p = AudioPrefs::default();
        assert!(p.edit_deny(&["USB PnP".into()], &[]));
        assert!(!p.edit_deny(&["usb pnp".into()], &[]));
        assert_eq!(p.deny, vec!["USB PnP".to_string()]);
        assert!(p.edit_deny(&[], &["usb PNP".into()]));
        assert!(p.deny.is_empty());
        assert!(!p.edit_deny(&["  ".into()], &[]));
    }

    #[test]
    fn the_device_list_marks_chosen_default_and_denied() {
        let dev = |label: &str, default: bool| DeviceInfo {
            kind: DeviceKind::AudioOutput,
            spec: format!("audio:{label}"),
            label: label.to_string(),
            detail: None,
            is_default: default,
        };
        let list = DeviceList::found(
            DeviceKind::AudioOutput,
            vec![dev("USB PnP Sound Device", true), dev("Headphones", false)],
        );
        let p = prefs(Some("audio:Headphones"), &["PnP"]);
        let s = describe_devices(&list, &p);
        assert!(s.contains("audio:USB PnP Sound Device  [system default, never used automatically; DENIED by \"PnP\"]"), "{s}");
        assert!(s.contains("audio:Headphones  [chosen]"), "{s}");
        assert!(s.contains("ACC2"));
        let s = describe_devices(
            &DeviceList::unavailable(DeviceKind::AudioOutput, "no ALSA"),
            &p,
        );
        assert!(s.contains("cannot list outputs: no ALSA"));
    }

    #[test]
    fn choose_refuses_the_default_device_whatever_the_route() {
        // The cw-trainer guard for the station rule: the system default
        // output IS the radio's ACC2 input. Nothing chosen, a bare
        // `audio:` on the command line, or a bare `audio:` remembered
        // (e.g. in an audio.json carried over from ts570d) must all
        // refuse, and never return a spec that would open the default.
        for flag in [None, Some("audio:"), Some("audio: ")] {
            for remembered in [None, Some("audio:"), Some("audio:\t")] {
                if flag.is_none() && remembered.is_none() {
                    let e = choose(None, &AudioPrefs::default()).unwrap_err();
                    assert!(e.contains("never used"), "{e}");
                    continue;
                }
                let r = choose(flag, &prefs(remembered, &[]));
                assert!(r.is_err(), "{flag:?} / {remembered:?} gave {r:?}");
            }
        }
    }

    #[test]
    fn refusals_point_at_cw_trainer_not_ts570d() {
        let denied = prefs(Some("audio:USB PnP"), &["pnp"]);
        let messages = [
            choose(None, &AudioPrefs::default()).unwrap_err(),
            choose(Some("x"), &AudioPrefs::default()).unwrap_err(),
            choose(Some("audio:"), &AudioPrefs::default()).unwrap_err(),
            choose(None, &denied).unwrap_err(),
            denied.check_opened("USB PnP").unwrap_err(),
        ];
        for m in messages {
            assert!(m.contains("`cw-trainer devices`"), "{m}");
            assert!(!m.contains("ts570d"), "{m}");
        }
    }
}
