// Copyright (C) 2026 Kenneth Allen Flegal and DappleAI
// SPDX-License-Identifier: GPL-3.0-or-later

//! One request on the tokio stack, run to its end on the task that holds it.

use std::io::Write;

use whp::{Outcome, Refusal, Store};

use crate::store::FileStore;

/// Runs `page`, as the working directory names it, to its end, writing it to `sink`.
pub fn page(
    store: &'static FileStore,
    page: &str,
    sink: &mut dyn Write,
) -> Result<Outcome, Refusal> {
    let cwd = store.cwd().as_os_str().as_encoded_bytes().to_vec();
    let entry = match store.kept(page.as_bytes(), &cwd) {
        Some(p) => p,
        None => store.load(page.as_bytes(), &cwd)?,
    };
    let outcome = whp::run(store, entry, sink)?;
    sink.flush()
        .map_err(|e| Refusal::at(format!("writing the page out: {e}"), 0))?;
    Ok(outcome)
}
