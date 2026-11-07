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

//! cw-trainer-core — Morse copy-practice sessions, with no I/O.
//!
//! The session logic of the `cw-trainer` CLI, as a library an app
//! consumer can share: synthesized on-air contacts, grouped into overs,
//! scored per character, with adaptive speed and a serializable record.
//! It has no terminal, sound card, network, file or clock code, and builds
//! for desktop and mobile targets alike. Everything about Morse comes from
//! [`cat_morse`], which is re-exported so a consumer links the very same
//! crate.
//!
//! # Audio safety: every consumer's duty
//!
//! This library produces audio to render; it never chooses or opens an
//! output device. **A consumer must never play it to a device it did not
//! explicitly choose.** On a radio station the system's default output may
//! be wired to a transmitter's audio input, so a sound played there can go
//! out over the air. The `cw-trainer` CLI plays only to an output the
//! operator named (or remembered), checks it against a deny list before
//! and after opening it, and refuses otherwise; a consumer must enforce an
//! equivalent rule on its own platform.

#![forbid(unsafe_code)]

pub use cat_morse;

pub mod options;
pub mod speed;
