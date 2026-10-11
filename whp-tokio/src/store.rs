// Copyright (C) 2026 Kenneth Allen Flegal and DappleAI
// SPDX-License-Identifier: GPL-3.0-or-later

//! Pages read from the filesystem, compiled once, and kept for the life of the process.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use whp::{Page, Refusal, Store, compile_page};

/// A page as a request names it: its path as written, and the directory of the page naming it.
type Named = (Vec<u8>, Vec<u8>);

/// Every page a request names, kept by the path it was named by and the directory it was named
/// from, so a later request finds it without touching the filesystem.
pub struct FileStore {
    cwd: PathBuf,
    kept: Mutex<HashMap<Named, &'static Page>>,
    by_path: Mutex<HashMap<PathBuf, &'static Page>>,
    reads: AtomicU64,
    bytes: AtomicU64,
}

impl FileStore {
    pub fn new(cwd: PathBuf) -> Self {
        FileStore {
            cwd,
            kept: Mutex::default(),
            by_path: Mutex::default(),
            reads: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
        }
    }

    /// How many files the store has read, and their bytes.
    #[cfg(feature = "round")]
    pub fn reads(&self) -> (u64, u64) {
        (
            self.reads.load(Ordering::Relaxed),
            self.bytes.load(Ordering::Relaxed),
        )
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Where PHP looks for a page: an absolute path as it is; `./` and `../` from the working
    /// directory; any other relative path in the working directory, then beside the page that
    /// names it.
    fn resolve(&self, path: &[u8], from_dir: &[u8]) -> PathBuf {
        let p = Path::new(OsStr::from_bytes(path));
        if p.is_absolute() {
            return p.to_path_buf();
        }
        if path.starts_with(b"./") || path.starts_with(b"../") {
            return self.cwd.join(p);
        }
        let here = self.cwd.join(p);
        if here.exists() {
            return here;
        }
        Path::new(OsStr::from_bytes(from_dir)).join(p)
    }
}

impl Store for FileStore {
    fn kept(&self, path: &[u8], from_dir: &[u8]) -> Option<&'static Page> {
        let named = (path.to_vec(), from_dir.to_vec());
        self.kept.lock().expect("the store").get(&named).copied()
    }

    fn load(&self, path: &[u8], from_dir: &[u8]) -> Result<&'static Page, Refusal> {
        let named = (path.to_vec(), from_dir.to_vec());
        if let Some(page) = self.kept.lock().expect("the store").get(&named) {
            return Ok(page);
        }
        let unreadable = |e: std::io::Error| {
            Refusal::at(
                format!(
                    "a page that cannot be read, {}: {e}",
                    String::from_utf8_lossy(path)
                ),
                0,
            )
        };
        let real = self
            .resolve(path, from_dir)
            .canonicalize()
            .map_err(unreadable)?;
        let page = {
            let mut by_path = self.by_path.lock().expect("the store");
            match by_path.get(&real) {
                Some(page) => *page,
                None => {
                    let source = std::fs::read(&real).map_err(unreadable)?;
                    self.reads.fetch_add(1, Ordering::Relaxed);
                    self.bytes.fetch_add(source.len() as u64, Ordering::Relaxed);
                    let page: &'static Page = Box::leak(Box::new(compile_page(
                        real.as_os_str().as_bytes(),
                        &source,
                    )?));
                    by_path.insert(real, page);
                    page
                }
            }
        };
        self.kept.lock().expect("the store").insert(named, page);
        Ok(page)
    }
}
