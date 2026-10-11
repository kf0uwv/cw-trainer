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

//! `cw-trainer` binary: parse, run, exit. Everything testable is in the lib.

use std::process::ExitCode;

use cw_trainer::cli::{self, Mode};
use cw_trainer::{app, version_line};

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .with_writer(std::io::stderr)
        .init();
    match cli::parse_args(std::env::args().skip(1)) {
        Ok(Mode::Help) => {
            print!("{}", cli::usage());
            ExitCode::SUCCESS
        }
        Ok(Mode::Version) => {
            println!("{}", version_line());
            ExitCode::SUCCESS
        }
        Ok(Mode::Send) => {
            eprintln!("`cw-trainer send` (sending practice) is not available yet.");
            ExitCode::from(2)
        }
        Ok(Mode::Devices(audio)) => {
            app::run_devices(&audio);
            ExitCode::SUCCESS
        }
        Ok(Mode::Copy(args)) => match app::run_copy(&args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("error: {e}\n\n{}", cli::usage());
            ExitCode::FAILURE
        }
    }
}
