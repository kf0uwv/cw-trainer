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

//! The command line: parsed purely, so it can be tested.
//!
//! Unknown flags are errors, because a typo here would otherwise quietly
//! become a default.

use std::path::PathBuf;

use cat_morse::Callsign;

use crate::{Farnsworth, Kind, Level};

/// `cw-trainer copy`'s flags, parsed but not yet resolved against the radio.
#[derive(Debug, Clone, PartialEq)]
pub struct CopyArgs {
    /// A radio server's native (console) port, `host:port`.
    pub server: Option<String>,
    pub kind: Kind,
    pub level: Level,
    pub qsos: u32,
    pub call: Callsign,
    pub wpm: Option<u32>,
    pub farnsworth: Farnsworth,
    pub pitch_hz: Option<f32>,
    pub seed: Option<u64>,
    pub adapt: bool,
    pub audio: AudioArgs,
}

/// The audio-output flags `copy` and `devices` share.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AudioArgs {
    pub audio_out: Option<String>,
    pub deny: Vec<String>,
    pub undeny: Vec<String>,
    pub data_dir: Option<PathBuf>,
}

/// What `cw-trainer <...>` asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Copy(Box<CopyArgs>),
    Devices(AudioArgs),
    Send,
    Help,
    Version,
}

/// Parse everything after the program name.
pub fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Mode, String> {
    let copy = match args.next().as_deref() {
        Some("copy") => true,
        Some("devices") => false,
        Some("send") => return Ok(Mode::Send),
        Some("--help" | "-h" | "help") => return Ok(Mode::Help),
        Some("--version" | "-V") => return Ok(Mode::Version),
        Some("cw") => {
            return Err(
                "there is no `cw` mode: what was `ts570d cw copy` is now `cw-trainer copy`"
                    .to_string(),
            )
        }
        Some(other) => {
            return Err(format!(
                "unknown mode {other:?}; expected copy, devices or send"
            ))
        }
        None => return Err("a mode is required: copy, devices (or send)".to_string()),
    };

    fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
        args.next()
            .ok_or_else(|| format!("{flag} requires a value"))
    }
    fn number<T>(text: String, flag: &str, range: std::ops::RangeInclusive<T>) -> Result<T, String>
    where
        T: std::str::FromStr + PartialOrd + std::fmt::Display,
    {
        match text.parse::<T>() {
            Ok(n) if range.contains(&n) => Ok(n),
            _ => Err(format!(
                "{flag} wants a whole number {}-{}, got {text:?}",
                range.start(),
                range.end()
            )),
        }
    }

    let mut server = None;
    let mut kind = Kind::Mixed;
    let mut level = Level::Intermediate;
    let mut qsos = 5;
    let mut call = None;
    let mut wpm = None;
    let mut farnsworth = Farnsworth::Preset;
    let mut pitch_hz = None;
    let mut seed = None;
    let mut adapt = true;
    let mut audio = AudioArgs::default();

    while let Some(flag) = args.next() {
        // Flags both modes take.
        match flag.as_str() {
            "--deny-audio" => {
                audio.deny.push(value(&mut args, &flag)?);
                continue;
            }
            "--undeny-audio" => {
                audio.undeny.push(value(&mut args, &flag)?);
                continue;
            }
            "--data-dir" => {
                audio.data_dir = Some(PathBuf::from(value(&mut args, &flag)?));
                continue;
            }
            _ if !copy => return Err(format!("unknown flag {flag:?} for devices")),
            _ => {}
        }
        match flag.as_str() {
            "--server" => server = Some(server_addr(value(&mut args, &flag)?)?),
            "--server-raw" => {
                return Err(
                    "--server-raw (a ts570d raw CAT port) is gone: use --server host:port, \
                     the radio server's console (native protocol) port"
                        .to_string(),
                )
            }
            "--kind" => kind = value(&mut args, &flag)?.parse()?,
            "--level" => level = value(&mut args, &flag)?.parse()?,
            "--qsos" => qsos = number(value(&mut args, &flag)?, &flag, 1..=100)?,
            "--call" => {
                let v = value(&mut args, &flag)?;
                call = Some(
                    Callsign::parse(&v.to_ascii_uppercase())
                        .map_err(|e| format!("--call {v:?}: {e}"))?,
                );
            }
            "--wpm" => {
                wpm = Some(number(
                    value(&mut args, &flag)?,
                    &flag,
                    crate::MIN_WPM..=crate::MAX_WPM,
                )?)
            }
            "--farnsworth" => farnsworth = value(&mut args, &flag)?.parse()?,
            "--pitch" => {
                // Whole hertz: "6e2" and "600.5" are typos, not tones.
                let hz: u32 = number(
                    value(&mut args, &flag)?,
                    &flag,
                    crate::MIN_PITCH_HZ as u32..=crate::MAX_PITCH_HZ as u32,
                )?;
                pitch_hz = Some(hz as f32);
            }
            "--seed" => {
                let v = value(&mut args, &flag)?;
                seed = Some(
                    v.parse()
                        .map_err(|_| format!("--seed wants a whole number, got {v:?}"))?,
                );
            }
            "--no-adapt" => adapt = false,
            "--audio-out" => audio.audio_out = Some(value(&mut args, &flag)?),
            other => return Err(format!("unknown flag {other:?}")),
        }
    }

    if !copy {
        return Ok(Mode::Devices(audio));
    }
    let call = call.ok_or_else(|| {
        "--call is required: the stations you work send to your callsign".to_string()
    })?;
    Ok(Mode::Copy(Box::new(CopyArgs {
        server,
        kind,
        level,
        qsos,
        call,
        wpm,
        farnsworth,
        pitch_hz,
        seed,
        adapt,
        audio,
    })))
}

/// `host:port` with a non-empty host and a port 1-65535 (`[::1]:port` too).
fn server_addr(v: String) -> Result<String, String> {
    let ok = v
        .rsplit_once(':')
        .is_some_and(|(host, port)| !host.is_empty() && port.parse::<u16>().is_ok_and(|p| p > 0));
    if ok {
        Ok(v)
    } else {
        Err(format!(
            "--server wants host:port (the radio server's console port), got {v:?}"
        ))
    }
}

/// The full `--help` text.
pub fn usage() -> String {
    format!(
        "Usage: cw-trainer copy --call <callsign> --audio-out audio:<name> [options]\n\
         \x20      cw-trainer devices [--deny-audio <text>] [--undeny-audio <text>]\n\
         \x20      cw-trainer --help | --version\n\
         \n\
         Copy practice. The PC plays synthesized on-air contacts through band\n\
         noise, fading, static and QRM; you type what you copy while it plays,\n\
         and every over is scored character by character. Nothing is\n\
         transmitted: with --server the radio's server is asked once for the\n\
         radio's CW pitch and keyer speed, and nothing else.\n\
         \n\
         AUDIO OUTPUT -- READ THIS. The system default output is never used: it\n\
         may be the radio's own USB sound interface on ACC2 (often a C-Media\n\
         \"USB PnP Sound Device\"), and audio played there goes into the\n\
         transmitter -- with VOX on it can key the radio. Name your speakers or\n\
         headphones once with --audio-out (it is remembered), and put the\n\
         radio's interface on the deny list so it can never be chosen:\n\
         \x20   cw-trainer devices --deny-audio \"USB PnP\"\n\
         \n\
         Options:\n\
         \x20 --call <cs>             your callsign (required)\n\
         \x20 --audio-out <spec>      audio:<name> from `cw-trainer devices`;\n\
         \x20                         required the first time, then remembered\n\
         \x20 --deny-audio <text>     never play to a device whose name contains\n\
         \x20                         <text> (remembered; repeatable)\n\
         \x20 --undeny-audio <text>   remove a deny-list entry\n\
         \x20 --kind <k>              {kinds} (mixed)\n\
         \x20 --level <l>             {levels} (intermediate)\n\
         \x20 --qsos <n>              contacts in the session, 1-100 (5)\n\
         \x20 --wpm <n>               starting speed, {min_wpm}-{max_wpm} (the radio's, else {wpm})\n\
         \x20 --farnsworth <wpm|off>  effective speed (the level's preset)\n\
         \x20 --pitch <hz>            tone in whole Hz, {min_hz}-{max_hz} (the radio's, else {hz})\n\
         \x20 --seed <n>              replay a session (printed at the start)\n\
         \x20 --no-adapt              keep the speed fixed\n\
         \x20 --server <host:port>    a radio server's console port (its\n\
         \x20                         `server --console-port`), to follow the radio's\n\
         \x20                         pitch and keyer speed (read once, never set);\n\
         \x20                         unreachable means local defaults\n\
         \x20 --data-dir <path>       where stats, history and the audio choice live\n\
         \n\
         Keys: type copy, Enter submits, Ctrl-R/F5 AGN, Ctrl-N/F6 skip, Esc quits.\n\
         Anything typed while your own line is shown is discarded.\n\
         \n\
         Connecting to --server, handshake included, gives up after 5 seconds.\n\
         \n\
         Known limitation: two sessions at once means the last to save\n\
         stats.json wins (history.jsonl keeps both).\n\
         \n\
         `cw-trainer send` (sending practice) is not available yet.\n",
        kinds = Kind::NAMES,
        levels = Level::NAMES,
        min_wpm = crate::MIN_WPM,
        max_wpm = crate::MAX_WPM,
        wpm = crate::DEFAULT_WPM,
        min_hz = crate::MIN_PITCH_HZ,
        max_hz = crate::MAX_PITCH_HZ,
        hz = crate::DEFAULT_PITCH_HZ,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Result<Mode, String> {
        parse_args(line.split_whitespace().map(String::from))
    }

    fn copy(line: &str) -> CopyArgs {
        match parse(line) {
            Ok(Mode::Copy(a)) => *a,
            other => panic!("{line:?} -> {other:?}"),
        }
    }

    #[test]
    fn only_the_call_is_required() {
        let a = copy("copy --call kf0uwv");
        assert_eq!(a.call.as_str(), "KF0UWV");
        assert_eq!(a.kind, Kind::Mixed);
        assert_eq!(a.level, Level::Intermediate);
        assert_eq!(a.qsos, 5);
        assert_eq!(a.farnsworth, Farnsworth::Preset);
        assert!(a.adapt);
        assert_eq!(
            (a.server, a.wpm, a.pitch_hz, a.seed),
            (None, None, None, None)
        );
        assert_eq!(a.audio, AudioArgs::default());
    }

    #[test]
    fn every_flag_is_read() {
        let a = copy(
            "copy --call W1AW --kind pileup --level advanced --qsos 3 --wpm 28 \
             --farnsworth off --pitch 700 --seed 42 --no-adapt \
             --server 127.0.0.1:4540 --audio-out audio:USB --data-dir /tmp/cw",
        );
        assert_eq!(a.kind, Kind::Pileup);
        assert_eq!(a.level, Level::Advanced);
        assert_eq!(a.qsos, 3);
        assert_eq!(a.wpm, Some(28));
        assert_eq!(a.farnsworth, Farnsworth::Off);
        assert_eq!(a.pitch_hz, Some(700.0));
        assert_eq!(a.seed, Some(42));
        assert!(!a.adapt);
        assert_eq!(a.server.as_deref(), Some("127.0.0.1:4540"));
        assert_eq!(a.audio.audio_out.as_deref(), Some("audio:USB"));
        assert_eq!(a.audio.data_dir, Some(PathBuf::from("/tmp/cw")));
        assert_eq!(
            copy("copy --call W1AW --farnsworth 12").farnsworth,
            Farnsworth::Wpm(12)
        );
    }

    #[test]
    fn a_missing_call_says_why_it_is_needed() {
        let e = parse("copy --kind contest").unwrap_err();
        assert!(e.contains("--call is required"), "{e}");
    }

    #[test]
    fn out_of_range_and_malformed_values_are_refused() {
        for line in [
            "copy --call W1AW --wpm 4",
            "copy --call W1AW --wpm 61",
            "copy --call W1AW --pitch 250",
            "copy --call W1AW --pitch 1300",
            "copy --call W1AW --qsos 0",
            "copy --call W1AW --kind dx",
            "copy --call W1AW --level expert",
            "copy --call W1AW --seed abc",
            "copy --call W1AW --farnsworth fast",
            "copy --call 12345",
            "copy --call W1AW --wpm",
            "copy --call W1AW --pitch 6e2",
            "copy --call W1AW --pitch 600.5",
            "copy --call W1AW --server",
            "copy --call W1AW --server localhost",
            "copy --call W1AW --server :4540",
            "copy --call W1AW --server host:port",
        ] {
            assert!(parse(line).is_err(), "{line:?} should be refused");
        }
    }

    #[test]
    fn an_unknown_flag_is_an_error_not_a_default() {
        let e = parse("copy --call W1AW --speed 20").unwrap_err();
        assert!(e.contains("unknown flag"), "{e}");
    }

    #[test]
    fn server_raw_is_refused_with_a_pointer_to_server() {
        let e = parse("copy --call W1AW --server-raw 127.0.0.1:4532").unwrap_err();
        assert!(e.contains("--server-raw"), "{e}");
        assert!(e.contains("--server host:port"), "{e}");
        assert!(e.contains("console"), "{e}");
    }

    #[test]
    fn deny_edits_are_taken_by_both_modes() {
        let a = copy("copy --call W1AW --deny-audio PnP --deny-audio ACC2 --undeny-audio Old");
        assert_eq!(a.audio.deny, vec!["PnP".to_string(), "ACC2".to_string()]);
        assert_eq!(a.audio.undeny, vec!["Old".to_string()]);
        match parse("devices --deny-audio PnP --data-dir /tmp/x") {
            Ok(Mode::Devices(d)) => {
                assert_eq!(d.deny, vec!["PnP".to_string()]);
                assert_eq!(d.data_dir, Some(PathBuf::from("/tmp/x")));
            }
            other => panic!("{other:?}"),
        }
        assert!(parse("devices --call W1AW").is_err());
        assert!(parse("devices --server h:1").is_err());
    }

    #[test]
    fn modes_route() {
        assert_eq!(parse("send"), Ok(Mode::Send));
        assert_eq!(parse("--help"), Ok(Mode::Help));
        assert_eq!(parse("-h"), Ok(Mode::Help));
        assert_eq!(parse("help"), Ok(Mode::Help));
        assert_eq!(parse("--version"), Ok(Mode::Version));
        assert_eq!(parse("-V"), Ok(Mode::Version));
        assert!(parse("").is_err());
        assert!(parse("listen").is_err());
        // ts570d's `cw` prefix is not a mode here.
        let e = parse("cw copy --call W1AW").unwrap_err();
        assert!(e.contains("cw-trainer copy"), "{e}");
    }

    #[test]
    fn usage_names_this_program_and_the_audio_rule() {
        let u = usage();
        assert!(u.contains("cw-trainer copy --call"), "{u}");
        assert!(u.contains("cw-trainer devices"), "{u}");
        assert!(u.contains("--server <host:port>"), "{u}");
        assert!(u.contains("never used"), "{u}");
        assert!(u.contains("ACC2"), "{u}");
        assert!(!u.contains("ts570d"), "{u}");
        assert!(!u.contains("--server-raw"), "{u}");
    }
}
