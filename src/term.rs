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

//! The operator's keyboard and screen, as the session sees them.
//!
//! Raw mode, because copy is typed *while* the audio plays: line-buffered
//! input would show nothing until Enter and could not see `Ctrl-R`. It is
//! one input line, not a console, so this is crossterm alone rather than
//! ratatui. Output written in raw mode needs `\r\n`; every string the
//! session writes uses it.
//!
//! # Leaving raw mode, however the program ends
//!
//! A terminal left in raw mode is unusable until `reset`, so raw mode is
//! undone on every way out, not only the happy one:
//!
//! - **Normal exit, and `Esc`/`Ctrl-C`:** [`CrosstermTerminal`]'s `Drop`.
//! - **A panic:** the release profile is `panic = "abort"`, so `Drop` never
//!   runs. [`chain_panic_hook`] restores the terminal *before* the previous
//!   (default) hook prints the message, so the message is readable.
//! - **SIGTERM, SIGHUP, SIGINT** (Unix) and **console close, logoff,
//!   shutdown, Ctrl-Break** (Windows): a `ctrlc` handler (its `termination`
//!   feature) drops raw mode at once and raises a flag that
//!   [`CrosstermTerminal::poll_key`] turns into [`Key::Quit`] within one
//!   poll, so the session ends the ordinary way — summary, stats and
//!   history saved. On Windows the process may be killed as soon as the
//!   handler returns; restoring the console first is what makes that safe.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// What the operator pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// A character of copy (already upper-cased).
    Char(char),
    Backspace,
    /// Submit the copy (or continue past a *you* line).
    Enter,
    /// Ask for a repeat: replay the over.
    Agn,
    /// Give up on this over, unscored.
    Skip,
    /// End the session.
    Quit,
}

/// Keys, a clock and somewhere to write.
pub trait Terminal {
    /// The next key, waiting at most `timeout`.
    fn poll_key(&mut self, timeout: Duration) -> io::Result<Option<Key>>;
    /// Now, on this terminal's clock.
    fn now(&self) -> Instant;
    /// Where text goes.
    fn out(&mut self) -> &mut dyn Write;
}

/// Map a crossterm key event to a [`Key`]; `None` for anything else.
pub fn map_key(ev: KeyEvent) -> Option<Key> {
    // Windows reports releases as well as presses.
    if ev.kind == KeyEventKind::Release {
        return None;
    }
    // AltGr arrives as Ctrl+Alt on Windows: with a printable character
    // that is the character (`@`, `/` on many layouts), not a command.
    let ctrl =
        ev.modifiers.contains(KeyModifiers::CONTROL) && !ev.modifiers.contains(KeyModifiers::ALT);
    match ev.code {
        KeyCode::Char(c) if ctrl => match c.to_ascii_lowercase() {
            'r' => Some(Key::Agn),
            'n' => Some(Key::Skip),
            'c' | 'd' => Some(Key::Quit),
            _ => None,
        },
        KeyCode::Char(c) if !c.is_control() => Some(Key::Char(c.to_ascii_uppercase())),
        KeyCode::Backspace => Some(Key::Backspace),
        KeyCode::Enter => Some(Key::Enter),
        KeyCode::Esc => Some(Key::Quit),
        KeyCode::F(5) => Some(Key::Agn),
        KeyCode::F(6) => Some(Key::Skip),
        _ => None,
    }
}

/// Set by the termination handler; sticky, so every later poll quits too.
static TERMINATE: AtomicBool = AtomicBool::new(false);

/// Run `restore` on any panic, then whatever hook was there before.
pub fn chain_panic_hook(restore: impl Fn() + Send + Sync + 'static) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
}

/// A key the termination flag forces, if it is up.
fn forced_quit(flag: &AtomicBool) -> Option<Key> {
    flag.load(Ordering::SeqCst).then_some(Key::Quit)
}

fn restore_terminal() {
    let _ = crossterm::terminal::disable_raw_mode();
}

/// The real terminal. Raw mode is on for as long as this lives.
pub struct CrosstermTerminal {
    stdout: io::Stdout,
}

impl CrosstermTerminal {
    pub fn new() -> io::Result<Self> {
        static HOOKS: Once = Once::new();
        HOOKS.call_once(|| {
            chain_panic_hook(restore_terminal);
            // Fails only if a handler is already set in this process; the
            // panic hook and Drop still cover the terminal then.
            let _ = ctrlc::set_handler(|| {
                restore_terminal();
                TERMINATE.store(true, Ordering::SeqCst);
            });
        });
        crossterm::terminal::enable_raw_mode()?;
        Ok(CrosstermTerminal {
            stdout: io::stdout(),
        })
    }
}

impl Drop for CrosstermTerminal {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = self.stdout.flush();
    }
}

impl Terminal for CrosstermTerminal {
    fn poll_key(&mut self, timeout: Duration) -> io::Result<Option<Key>> {
        let _ = self.stdout.flush();
        if let Some(k) = forced_quit(&TERMINATE) {
            return Ok(Some(k));
        }
        if !event::poll(timeout)? {
            return Ok(forced_quit(&TERMINATE));
        }
        match event::read()? {
            Event::Key(k) => Ok(map_key(k)),
            _ => Ok(None),
        }
    }

    fn now(&self) -> Instant {
        Instant::now()
    }

    fn out(&mut self) -> &mut dyn Write {
        &mut self.stdout
    }
}

#[cfg(test)]
pub mod tests {
    use std::collections::VecDeque;

    use super::*;

    /// A terminal driven by a script of keys on a fake clock.
    ///
    /// The clock moves only when the session waits: a poll that finds no
    /// key due advances it by the timeout. A key is due at its scheduled
    /// time. When the script runs out the operator "quits", so a test can
    /// never hang.
    pub struct ScriptedTerminal {
        start: Instant,
        clock: Duration,
        keys: VecDeque<(Duration, Key)>,
        pub output: Vec<u8>,
        /// When the script runs out: fail like a vanished terminal
        /// instead of quitting.
        pub fail_when_done: bool,
    }

    impl ScriptedTerminal {
        pub fn new(keys: Vec<(Duration, Key)>) -> Self {
            ScriptedTerminal {
                start: Instant::now(),
                clock: Duration::ZERO,
                keys: keys.into(),
                output: Vec::new(),
                fail_when_done: false,
            }
        }

        /// Type `text` then press Enter, all due immediately.
        pub fn typed(lines: &[&str]) -> Vec<(Duration, Key)> {
            let mut v = Vec::new();
            for l in lines {
                v.extend(l.chars().map(|c| (Duration::ZERO, Key::Char(c))));
                v.push((Duration::ZERO, Key::Enter));
            }
            v
        }

        pub fn text(&self) -> String {
            String::from_utf8_lossy(&self.output).into_owned()
        }
    }

    impl Terminal for ScriptedTerminal {
        fn poll_key(&mut self, timeout: Duration) -> io::Result<Option<Key>> {
            match self.keys.front() {
                None if self.fail_when_done => Err(io::Error::other("terminal went away")),
                None => Ok(Some(Key::Quit)),
                Some(&(at, key)) if at <= self.clock + timeout => {
                    self.clock = self.clock.max(at);
                    self.keys.pop_front();
                    Ok(Some(key))
                }
                Some(_) => {
                    self.clock += timeout;
                    Ok(None)
                }
            }
        }

        fn now(&self) -> Instant {
            self.start + self.clock
        }

        fn out(&mut self) -> &mut dyn Write {
            &mut self.output
        }
    }

    fn press(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn copy_is_upper_cased_and_controls_map_to_actions() {
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::CONTROL;
        assert_eq!(
            map_key(press(KeyCode::Char('w'), none)),
            Some(Key::Char('W'))
        );
        assert_eq!(
            map_key(press(KeyCode::Char('/'), none)),
            Some(Key::Char('/'))
        );
        assert_eq!(map_key(press(KeyCode::Char('r'), ctrl)), Some(Key::Agn));
        assert_eq!(map_key(press(KeyCode::Char('n'), ctrl)), Some(Key::Skip));
        assert_eq!(map_key(press(KeyCode::Char('c'), ctrl)), Some(Key::Quit));
        assert_eq!(map_key(press(KeyCode::Esc, none)), Some(Key::Quit));
        assert_eq!(map_key(press(KeyCode::F(5), none)), Some(Key::Agn));
        assert_eq!(map_key(press(KeyCode::F(6), none)), Some(Key::Skip));
        assert_eq!(map_key(press(KeyCode::Enter, none)), Some(Key::Enter));
        assert_eq!(
            map_key(press(KeyCode::Backspace, none)),
            Some(Key::Backspace)
        );
        assert_eq!(map_key(press(KeyCode::Up, none)), None);
    }

    #[test]
    fn a_termination_signal_becomes_quit_and_stays_quit() {
        let flag = AtomicBool::new(false);
        assert_eq!(forced_quit(&flag), None);
        flag.store(true, Ordering::SeqCst);
        assert_eq!(forced_quit(&flag), Some(Key::Quit));
        assert_eq!(forced_quit(&flag), Some(Key::Quit));
    }

    #[test]
    fn a_panic_restores_the_terminal_before_the_previous_hook_runs() {
        use std::sync::atomic::AtomicUsize;
        use std::sync::Arc;

        static ORDER: AtomicUsize = AtomicUsize::new(0);
        let restored_at = Arc::new(AtomicUsize::new(0));
        let previous_at = Arc::new(AtomicUsize::new(0));
        let original = std::panic::take_hook();
        {
            let previous_at = previous_at.clone();
            std::panic::set_hook(Box::new(move |_| {
                previous_at.store(ORDER.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
            }));
        }
        {
            let restored_at = restored_at.clone();
            chain_panic_hook(move || {
                restored_at.store(ORDER.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
            });
        }
        let r = std::panic::catch_unwind(|| panic!("boom"));
        std::panic::set_hook(original);
        assert!(r.is_err());
        let (restored, previous) = (
            restored_at.load(Ordering::SeqCst),
            previous_at.load(Ordering::SeqCst),
        );
        assert!(restored > 0, "restore ran");
        assert!(previous > restored, "then the previous hook");
    }

    #[test]
    fn altgr_characters_are_copy_not_commands() {
        let altgr = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert_eq!(
            map_key(press(KeyCode::Char('/'), altgr)),
            Some(Key::Char('/'))
        );
        assert_eq!(
            map_key(press(KeyCode::Char('r'), altgr)),
            Some(Key::Char('R'))
        );
    }

    #[test]
    fn a_key_release_is_not_a_second_keystroke() {
        let mut ev = press(KeyCode::Char('a'), KeyModifiers::NONE);
        ev.kind = KeyEventKind::Release;
        assert_eq!(map_key(ev), None);
    }
}
