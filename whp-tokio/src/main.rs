// Copyright (C) 2026 Kenneth Allen Flegal and DappleAI
// SPDX-License-Identifier: GPL-3.0-or-later

//! `whp run <page>`: runs a page as one request on the tokio stack and writes it to stdout.
//!
//! `whp` exits 0 when the page went out whole, 3 when whp refused it (the refusal, its page and
//! its line, on stderr), and 2 when it could not start: a wrong argument.
//!
//! Built with the `round` feature, `run` also takes `--repeat N`, N requests one after another,
//! and `--round <file>`, one JSON line a request saying what the stack and the request did.

mod host;
mod store;

#[cfg(feature = "round")]
mod round;

use std::process::ExitCode;

use store::FileStore;

const REFUSED: u8 = 3;

pub struct Args {
    pub page: String,
    pub repeat: u64,
    pub round: Option<String>,
}

fn usage() -> ExitCode {
    eprintln!("usage: whp run <page.php>");
    #[cfg(feature = "round")]
    {
        eprintln!("       whp run <page.php> [--repeat N] [--round <file.jsonl>]");
    }
    eprintln!(
        "exits 0 when the page went out whole, 3 when whp refused it, 2 when it cannot start"
    );
    ExitCode::from(2)
}

// The flags take a value from the same iterator, so the loops cannot be `for`s.
#[allow(clippy::while_let_on_iterator)]
fn args() -> Option<Args> {
    let mut it = std::env::args().skip(1);
    match it.next().as_deref() {
        Some("run") => {
            let mut a = Args {
                page: String::new(),
                repeat: 1,
                round: None,
            };
            while let Some(arg) = it.next() {
                match arg.as_str() {
                    #[cfg(feature = "round")]
                    "--repeat" => a.repeat = it.next()?.parse().ok().filter(|&n| n > 0)?,
                    #[cfg(feature = "round")]
                    "--round" => a.round = Some(it.next()?),
                    _ if a.page.is_empty() && !arg.starts_with("--") => a.page = arg,
                    _ => return None,
                }
            }
            (!a.page.is_empty()).then_some(a)
        }
        _ => None,
    }
}

fn runtime() -> Result<tokio::runtime::Runtime, ExitCode> {
    tokio::runtime::Builder::new_multi_thread()
        .build()
        .map_err(|e| {
            eprintln!("whp: the tokio runtime did not start: {e}");
            ExitCode::from(2)
        })
}

fn main() -> ExitCode {
    match args() {
        Some(args) => run(args),
        None => usage(),
    }
}

fn run(args: Args) -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("whp: no working directory: {e}");
            return ExitCode::from(2);
        }
    };
    let store: &'static FileStore = Box::leak(Box::new(FileStore::new(cwd)));
    let runtime = match runtime() {
        Ok(r) => r,
        Err(code) => return code,
    };
    let page: &'static str = Box::leak(args.page.clone().into_boxed_str());
    let result = runtime.block_on(async move {
        #[cfg(feature = "round")]
        {
            round::drive(store, page, &args).await
        }
        #[cfg(not(feature = "round"))]
        {
            let _ = &args;
            tokio::spawn(async move {
                let mut out = std::io::BufWriter::new(std::io::stdout());
                host::page(store, page, &mut out).map(|_| ())
            })
            .await
            .map_err(|e| whp::Refusal::at(format!("the request's task: {e}"), 0))?
        }
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(r) => {
            eprintln!("{r}");
            ExitCode::from(REFUSED)
        }
    }
}
