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

//! cw-trainer: a radio-neutral Morse code (CW) trainer.
//!
//! A cat-native *client* of any radio-cat-rs radio's server. Bootstrap
//! only so far; the trainer itself moves here from `ts570d cw`.

pub mod time;

/// The program's name as it appears on the command line.
pub const PROGRAM: &str = "cw-trainer";

/// What `cw-trainer --version` prints: the program name and its version.
pub fn version_line() -> String {
    format!("{PROGRAM} {}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_line_names_program_and_package_version() {
        assert_eq!(
            version_line(),
            format!("cw-trainer {}", env!("CARGO_PKG_VERSION"))
        );
    }
}
