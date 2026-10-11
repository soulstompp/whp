// Copyright (C) 2026 Kenneth Allen Flegal and DappleAI
// SPDX-License-Identifier: GPL-3.0-or-later

//! whp, the warren home page: PHP 8's templates, run as requests.
//!
//! A request comes in, a page runs top to bottom, the page goes out, and everything the request
//! made is thrown away. Each page gives PHP's own bytes, or is refused with a [`Refusal`] that
//! names what whp met and the line it met it on; whp never runs a page differently.
//!
//! A request is an iterator over its waits. In PHP a request runs to its end inside the engine,
//! which blocks at every sleep and every page it loads. Here a page is compiled once
//! ([`compile_page`]) and kept by its host's [`Store`], and a [`Request`] runs it, writing the
//! page to a sink the host gives it, until it must wait: `sleep()`, `usleep()`, or a page its
//! store has not kept. It hands the wait back as a [`Step`], the host does it with whatever Rust
//! library suits it, and resumes the request. whp does no I/O of its own: the store, the timer,
//! the loader and the runtime are the host's. [`run()`] drives one request to its end on the
//! calling thread.
//!
//! ```
//! use whp::{Page, Refusal, Request, Step, Store, Wait, compile_page};
//!
//! /// A store holding one page, kept from the start.
//! struct One(&'static Page);
//!
//! impl Store for One {
//!     fn kept(&self, _path: &[u8], _from_dir: &[u8]) -> Option<&'static Page> {
//!         Some(self.0)
//!     }
//!     fn load(&self, _path: &[u8], _from_dir: &[u8]) -> Result<&'static Page, Refusal> {
//!         Ok(self.0)
//!     }
//! }
//!
//! let source = b"<p><?= 'Hello, ' . 'world' ?></p>\n";
//! let page: &'static Page = Box::leak(Box::new(compile_page(b"/hello.php", source).unwrap()));
//! let store = One(page);
//! let mut out = Vec::new();
//! let mut request = Request::new(&store, page);
//! loop {
//!     match request.resume(&mut out).unwrap() {
//!         Step::Done(_) => break,
//!         Step::Wait(Wait::Sleep(time)) => std::thread::sleep(time),
//!         Step::Wait(Wait::Page { path, from_dir }) => {
//!             let loaded = store.load(&path, &from_dir).map_err(|r| request.failed(r));
//!             request.loaded(loaded.unwrap());
//!         }
//!     }
//! }
//! assert_eq!(out, b"<p>Hello, world</p>\n");
//! ```
//!
//! The `whp-tokio` crate is the host on the tokio stack, the `whp` command: `whp run` runs a page
//! from the command line, and `whp serve` serves a directory's pages over HTTP.

#![warn(missing_docs)]

mod builtins;
mod compile;
pub mod lex;
mod parse;
mod run;
mod store;
mod value;

pub use compile::{Page, compile_page};
pub use run::{Outcome, Request, Step, Wait, run};
pub use store::Store;

use std::fmt;

/// A page whp will not run, or will not run further: what it met, and where. whp refuses
/// rather than give a page PHP would not give.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// What whp met, in plain words: the construct, the value or the limit.
    pub what: String,
    /// The page it was met in, as its path.
    pub file: String,
    /// The line it was met on.
    pub line: u32,
}

impl Refusal {
    /// A refusal of `what`, met on `line` of a page not yet named.
    pub fn at(what: impl Into<String>, line: u32) -> Self {
        Refusal {
            what: what.into(),
            file: String::new(),
            line,
        }
    }

    /// Names the page the refusal was met in, unless it already names one.
    pub fn in_file(self, path: &[u8]) -> Self {
        if self.file.is_empty() {
            Refusal {
                file: String::from_utf8_lossy(path).into_owned(),
                ..self
            }
        } else {
            self
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "whp refuses {}, at {}:{}",
            self.what, self.file, self.line
        )
    }
}

impl std::error::Error for Refusal {}
