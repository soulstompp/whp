// Copyright (C) 2026 Kenneth Allen Flegal and DappleAI
// SPDX-License-Identifier: GPL-3.0-or-later

//! Where a request finds its pages. The host keeps them: a page is compiled the first time a
//! request asks for it, and every later request reads the kept page.

use crate::Refusal;
use crate::compile::Page;

/// The pages a request may run, kept by the host.
pub trait Store {
    /// The compiled page at `path`, as a page in `from_dir` names it, if the store has kept it. A
    /// kept page lives as long as the process.
    fn kept(&self, path: &[u8], from_dir: &[u8]) -> Option<&'static Page>;

    /// Reads and compiles the page, and keeps it if the store keeps pages.
    fn load(&self, path: &[u8], from_dir: &[u8]) -> Result<&'static Page, Refusal>;
}
