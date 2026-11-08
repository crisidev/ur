//! The command line: run scripts headless, or open the window.

use std::io::BufRead;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::script::{self, Next, Output};
use crate::watch::Watch;

const USAGE: &str = "\
Usage: ur-sim [--window] [--out DIR] [--speed N] [--zoom N] [SCRIPT ...]
       ur-sim --devkit ELF [--out DIR] SCRIPT ...

Runs the ur firmware's UI on this computer. With scripts, runs them (each
line a command; `-` reads commands from stdin) and exits; without, or with
--window, opens a window to use the watch with the mouse and keyboard,
after running any scripts, and takes commands typed into the terminal.

  --window     open the window even with scripts
  --out DIR    where `shot` writes screenshots (default: .)
  --speed N    in the window, run time N times as fast (default: 1)
  --zoom N     in the window (and `shot … --watch`), draw a canvas pixel N
               pixels wide (default: 4, a 320-pixel screen; 3 is the
               panel's own size)
  --devkit ELF follow a dev kit running ELF (a `--features compare` build):
               run the scripts' inputs on it, replay what its UI did, and
               check each `compare NAME` frame against it, pixel for pixel
               (see README.md)
  --help       this, and the commands
";

pub struct Options {
    pub scripts: Vec<PathBuf>,
    pub window: bool,
    pub output: Output,
    pub speed: f64,
    pub zoom: usize,
    pub devkit: Option<PathBuf>,
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
    if let Some(elf) = &options.devkit {
        return match crate::devkit::run(elf, &options.scripts, &options.output.dir) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("ur-sim: {e}");
                ExitCode::FAILURE
            }
        };
    }
    let mut watch = Watch::boot();
    for path in &options.scripts {
        let result = if path.as_os_str() == "-" {
            run_stdin(&mut watch, &options.output)
        } else {
            script::run_file(&mut watch, path, &options.output)
        };
        match result {
            Ok(Next::Continue) => {}
            Ok(Next::Quit) => return ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("ur-sim: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    if options.window {
        return open_window(watch, &options);
    }
    ExitCode::SUCCESS
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
        devkit: None,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(None),
            "--window" => options.window = true,
            "--zoom" => {
                let zoom = args.next().ok_or("--zoom needs N")?;
                options.zoom = match zoom.parse::<usize>() {
                    Ok(n @ 1..=12) => n,
                    _ => return Err(format!("--zoom {zoom}? (1 to 12)")),
                };
                options.output.zoom = options.zoom;
            }
            "--devkit" => options.devkit = Some(args.next().ok_or("--devkit needs an ELF")?.into()),
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
    options.window |= options.scripts.is_empty();
    Ok(Some(options))
}
