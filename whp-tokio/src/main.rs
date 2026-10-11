// Copyright (C) 2026 Kenneth Allen Flegal and DappleAI
// SPDX-License-Identifier: GPL-3.0-or-later

//! `whp run <page>`: runs a page as one request on the tokio stack and writes it to stdout.
//!
//! `whp serve <dir> [--addr <addr>]`: serves the pages under `<dir>` over HTTP/1.1, each HTTP
//! request one request, until the process is stopped.
//!
//! `whp` exits 0 when the page went out whole, 3 when whp refused it (the refusal, its page and
//! its line, on stderr), and 2 when it could not start: a wrong argument, or a directory it
//! cannot serve.
//!
//! Built with the `round` feature, `run` also takes `--repeat N`, N requests one after another;
//! `--concurrent N`, one request and then N at once, each of which must give the same bytes; and
//! `serve` and `run` take `--workers W`, the stack's worker threads, and `--round <file>`, one
//! JSON line a request saying what the stack and the request did.

mod host;
mod serve;
mod store;

#[cfg(feature = "round")]
mod round;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;

use store::FileStore;

const REFUSED: u8 = 3;

pub struct Args {
    pub page: String,
    pub repeat: u64,
    pub concurrent: u64,
    pub workers: Option<usize>,
    pub round: Option<String>,
}

pub struct ServeArgs {
    pub dir: PathBuf,
    pub addr: SocketAddr,
    pub workers: Option<usize>,
    pub round: Option<String>,
}

enum Command {
    Run(Args),
    Serve(ServeArgs),
}

fn usage() -> ExitCode {
    eprintln!("usage: whp run <page.php>");
    eprintln!("       whp serve <dir> [--addr <addr>]");
    #[cfg(feature = "round")]
    {
        eprintln!(
            "       whp run <page.php> [--repeat N] [--concurrent N] [--workers W] [--round <file.jsonl>]"
        );
        eprintln!("       whp serve <dir> [--addr <addr>] [--workers W] [--round <file.jsonl>]");
    }
    eprintln!(
        "exits 0 when the page went out whole, 3 when whp refused it, 2 when it cannot start"
    );
    ExitCode::from(2)
}

// The flags take a value from the same iterator, so the loops cannot be `for`s.
#[allow(clippy::while_let_on_iterator)]
fn args() -> Option<Command> {
    let mut it = std::env::args().skip(1);
    match it.next().as_deref() {
        Some("run") => {
            let mut a = Args {
                page: String::new(),
                repeat: 1,
                concurrent: 0,
                workers: None,
                round: None,
            };
            while let Some(arg) = it.next() {
                match arg.as_str() {
                    #[cfg(feature = "round")]
                    "--repeat" => a.repeat = it.next()?.parse().ok().filter(|&n| n > 0)?,
                    #[cfg(feature = "round")]
                    "--concurrent" => a.concurrent = it.next()?.parse().ok().filter(|&n| n > 0)?,
                    #[cfg(feature = "round")]
                    "--workers" => a.workers = Some(it.next()?.parse().ok().filter(|&n| n > 0)?),
                    #[cfg(feature = "round")]
                    "--round" => a.round = Some(it.next()?),
                    _ if a.page.is_empty() && !arg.starts_with("--") => a.page = arg,
                    _ => return None,
                }
            }
            (!a.page.is_empty()).then_some(Command::Run(a))
        }
        Some("serve") => {
            let mut dir = None;
            let mut a = ServeArgs {
                dir: PathBuf::new(),
                addr: SocketAddr::from(([127, 0, 0, 1], 8080)),
                workers: None,
                round: None,
            };
            while let Some(arg) = it.next() {
                match arg.as_str() {
                    "--addr" => a.addr = it.next()?.parse().ok()?,
                    #[cfg(feature = "round")]
                    "--workers" => a.workers = Some(it.next()?.parse().ok().filter(|&n| n > 0)?),
                    #[cfg(feature = "round")]
                    "--round" => a.round = Some(it.next()?),
                    _ if dir.is_none() && !arg.starts_with("--") => dir = Some(arg),
                    _ => return None,
                }
            }
            a.dir = PathBuf::from(dir?);
            Some(Command::Serve(a))
        }
        _ => None,
    }
}

fn runtime(workers: Option<usize>, io: bool) -> Result<tokio::runtime::Runtime, ExitCode> {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.enable_time();
    if io {
        builder.enable_io();
    }
    if let Some(w) = workers {
        builder.worker_threads(w);
    }
    builder.build().map_err(|e| {
        eprintln!("whp: the tokio runtime did not start: {e}");
        ExitCode::from(2)
    })
}

fn main() -> ExitCode {
    match args() {
        Some(Command::Run(args)) => run(args),
        Some(Command::Serve(args)) => serve(args),
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
    let runtime = match runtime(args.workers, false) {
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
                host::page(store, page, &mut out).await.map(|_| ())
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

fn serve(args: ServeArgs) -> ExitCode {
    let dir = match args.dir.canonicalize() {
        Ok(d) if d.is_dir() => d,
        Ok(d) => {
            eprintln!("whp: not a directory: {}", d.display());
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("whp: {}: {e}", args.dir.display());
            return ExitCode::from(2);
        }
    };
    let store: &'static FileStore = Box::leak(Box::new(FileStore::new(dir)));
    #[cfg(feature = "round")]
    let log = match args.round.as_deref().map(round::Log::create).transpose() {
        Ok(log) => log,
        Err(e) => {
            eprintln!("whp: the round's file: {e}");
            return ExitCode::from(2);
        }
    };
    #[cfg(not(feature = "round"))]
    let _ = &args.round;
    let server = serve::Server {
        store,
        #[cfg(feature = "round")]
        log,
    };
    let runtime = match runtime(args.workers, true) {
        Ok(r) => r,
        Err(code) => return code,
    };
    match runtime.block_on(serve::listen(server, args.addr)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("whp: serving at {}: {e}", args.addr);
            ExitCode::from(2)
        }
    }
}
