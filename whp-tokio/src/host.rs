// Copyright (C) 2026 Kenneth Allen Flegal and DappleAI
// SPDX-License-Identifier: GPL-3.0-or-later

//! One request on the tokio stack: each wait the request hands back is an await, so its thread
//! runs other requests meanwhile, and the page's bytes so far go out at every wait.

use std::io::Write;

use whp::{Outcome, Page, Refusal, Request, Step, Store, Wait};

use crate::store::FileStore;

/// A request run to its end, and how many times it waited.
#[cfg_attr(not(feature = "round"), allow(dead_code))]
pub struct Served {
    pub outcome: Outcome,
    pub waits: u64,
}

/// Loads a page on the stack's blocking threads, where waiting on the disk holds up no request.
pub async fn load(
    store: &'static FileStore,
    path: Vec<u8>,
    from_dir: Vec<u8>,
) -> Result<&'static Page, Refusal> {
    tokio::task::spawn_blocking(move || store.load(&path, &from_dir))
        .await
        .map_err(|e| Refusal::at(format!("loading a page: {e}"), 0))?
}

fn flush(sink: &mut (dyn Write + Send)) -> Result<(), Refusal> {
    sink.flush()
        .map_err(|e| Refusal::at(format!("writing the page out: {e}"), 0))
}

/// Runs `page`, as the working directory names it, to its end, writing it to `sink`. Loading the
/// page itself is one of its waits.
pub async fn page(
    store: &'static FileStore,
    page: &str,
    sink: &mut (dyn Write + Send),
) -> Result<Served, Refusal> {
    let cwd = store.cwd().as_os_str().as_encoded_bytes().to_vec();
    let (entry, waited) = match store.kept(page.as_bytes(), &cwd) {
        Some(p) => (p, 0),
        None => (load(store, page.as_bytes().to_vec(), cwd).await?, 1),
    };
    let mut served = run(store, entry, sink).await?;
    served.waits += waited;
    Ok(served)
}

/// Runs `entry` to its end, writing it to `sink`.
pub async fn run(
    store: &'static FileStore,
    entry: &'static Page,
    sink: &mut (dyn Write + Send),
) -> Result<Served, Refusal> {
    let mut waits = 0;
    let mut request = Request::new(store, entry);
    loop {
        match request.resume(sink)? {
            Step::Done(outcome) => {
                flush(sink)?;
                return Ok(Served { outcome, waits });
            }
            Step::Wait(wait) => {
                flush(sink)?;
                waits += 1;
                match wait {
                    Wait::Sleep(d) => tokio::time::sleep(d).await,
                    Wait::Page { path, from_dir } => {
                        let page = load(store, path, from_dir)
                            .await
                            .map_err(|r| request.failed(r))?;
                        request.loaded(page);
                    }
                }
            }
        }
    }
}
