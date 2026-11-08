//! The command line: run scripts headless, or open the window.

use std::io::BufRead;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::flash::Flash;
use crate::script::{self, Next, Output};
use crate::watch::Watch;

const USAGE: &str = "\
Usage: ur-sim [--window] [--flash FILE] [--out DIR] [--speed N] [--zoom N]
              [--sent-colours] [SCRIPT ...]
       ur-sim --devkit ELF [--out DIR] SCRIPT ...
       ur-sim --diff BEFORE AFTER [--out DIR]

Runs the ur firmware's UI on this computer. With scripts, runs them (each
line a command; `-` reads commands from stdin) and exits; without, or with
--window, opens a window to use the watch with the mouse and keyboard,
after running any scripts, and takes commands typed into the terminal.

  --window     open the window even with scripts
  --flash FILE keep the watch's flash (its settings) in FILE, from one run
               to the next; or start from a dump of a watch's (its 12 KiB
               at 0x7D000). Without, it starts erased, and is forgotten
  --out DIR    where `shot` writes screenshots (default: .)
  --speed N    in the window, run time N times as fast (default: 1)
  --zoom N     in the window (and `shot … --watch`), draw a canvas pixel N
               pixels wide (default: 4, a 320-pixel screen; 3 is the
               panel's own size)
  --sent-colours
               in the window, show the colours as sent to the panel,
               rather than as the real panel shows them (P toggles)
  --devkit ELF follow a dev kit running ELF (a `--features compare` build):
               run the scripts' inputs on it, replay what its UI did, and
               check each `compare NAME` frame against it, pixel for pixel
               (see README.md)
  --diff BEFORE AFTER
               compare the screenshots (every PNG) under two directories,
               pixel for pixel; write the differences under --out (as
               changed/, added/ and removed/), and fail if there are any
  --help       this, and the commands
";

pub struct Options {
    pub scripts: Vec<PathBuf>,
    pub window: bool,
    pub output: Output,
    pub speed: f64,
    pub zoom: usize,
    /// In the window, the real panel's colours (see `panel`).
    pub mimic: bool,
    pub devkit: Option<PathBuf>,
    /// `--diff BEFORE AFTER`.
    pub diff: Option<(PathBuf, PathBuf)>,
    pub flash: Option<PathBuf>,
}

pub fn main() -> ExitCode {
    let options = match parse(std::env::args().skip(1)) {
        Ok(Some(options)) => options,
        Ok(None) => {
            print!("{USAGE}\nCommands:\n{}", script::HELP);
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("ur-sim: {e}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    if let Some((before, after)) = &options.diff {
        return match crate::diff::run(before, after, &options.output.dir) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::FAILURE,
            Err(e) => {
                eprintln!("ur-sim: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if let Some(elf) = &options.devkit {
        return match crate::devkit::run(elf, &options.scripts, &options.output.dir) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("ur-sim: {e}");
                ExitCode::FAILURE
            }
        };
    }
    let flash = match &options.flash {
        Some(path) => match Flash::open(path) {
            Ok(flash) => flash,
            Err(e) => {
                eprintln!("ur-sim: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => Flash::erased(),
    };
    let mut watch = Watch::boot(flash);
    for path in &options.scripts {
        let result = if path.as_os_str() == "-" {
            run_stdin(&mut watch, &options.output)
        } else {
            script::run_file(&mut watch, path, &options.output)
        };
        match result {
            Ok(Next::Continue) => {}
            Ok(Next::Quit) => return finish_recording(ExitCode::SUCCESS),
            Err(e) => {
                eprintln!("ur-sim: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    let code = if options.window {
        open_window(watch, &options)
    } else {
        ExitCode::SUCCESS
    };
    finish_recording(code)
}

/// A recording left going (no `stop`, a `quit`, or the window closed with
/// one on) is written as it stands, rather than lost.
fn finish_recording(code: ExitCode) -> ExitCode {
    if !crate::record::active() {
        return code;
    }
    match crate::record::stop() {
        Ok(recorded) => {
            eprintln!("ur-sim: never stopped; {recorded}");
            code
        }
        Err(e) => {
            eprintln!("ur-sim: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(feature = "window")]
fn open_window(watch: Watch, options: &Options) -> ExitCode {
    match crate::window::run(watch, options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ur-sim: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(feature = "window"))]
fn open_window(_: Watch, _: &Options) -> ExitCode {
    eprintln!("ur-sim: built without the window (the `window` feature)");
    ExitCode::FAILURE
}

fn run_stdin(watch: &mut Watch, output: &Output) -> Result<Next, String> {
    for line in std::io::stdin().lock().lines() {
        let line = line.map_err(|e| format!("reading stdin: {e}"))?;
        if let Next::Quit = script::execute(watch, &line, output)? {
            return Ok(Next::Quit);
        }
    }
    Ok(Next::Continue)
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut options = Options {
        scripts: Vec::new(),
        window: false,
        output: Output {
            dir: ".".into(),
            zoom: 4,
        },
        speed: 1.0,
        zoom: 4,
        mimic: true,
        devkit: None,
        diff: None,
        flash: None,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--window" => options.window = true,
            "--sent-colours" => options.mimic = false,
            "--zoom" => {
                let zoom = args.next().ok_or("--zoom needs N")?;
                options.zoom = match zoom.parse::<usize>() {
                    Ok(n @ 1..=12) => n,
                    _ => return Err(format!("--zoom {zoom}? (1 to 12)")),
                };
                options.output.zoom = options.zoom;
            }
            "--devkit" => options.devkit = Some(args.next().ok_or("--devkit needs an ELF")?.into()),
            "--flash" => options.flash = Some(args.next().ok_or("--flash needs a FILE")?.into()),
            "--diff" => {
                let before = args.next().ok_or("--diff needs BEFORE and AFTER")?;
                let after = args.next().ok_or("--diff needs BEFORE and AFTER")?;
                options.diff = Some((before.into(), after.into()));
            }
            "--out" => options.output.dir = args.next().ok_or("--out needs a DIR")?.into(),
            "--speed" => {
                let speed = args.next().ok_or("--speed needs N")?;
                options.speed = match speed.parse::<f64>() {
                    Ok(n) if n > 0.0 => n,
                    _ => return Err(format!("--speed {speed}?")),
                };
            }
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            path => options.scripts.push(path.into()),
        }
    }
    if options.devkit.is_some() && (options.window || options.scripts.is_empty()) {
        return Err("--devkit runs scripts, without the window".into());
    }
    if options.diff.is_some()
        && (options.window || options.devkit.is_some() || !options.scripts.is_empty())
    {
        return Err("--diff compares screenshots; it runs no scripts".into());
    }
    options.window |= options.scripts.is_empty();
    Ok(Some(options))
}
