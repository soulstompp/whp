// Copyright (C) 2026 Kenneth Allen Flegal and DappleAI
// SPDX-License-Identifier: GPL-3.0-or-later

//! Every page of the corpus gives PHP's own bytes, as `tests/pages/<page>.out` holds them.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use whp::{Page, Refusal, Store, compile_page};

/// Pages read from disk once and kept.
#[derive(Default)]
struct Files {
    kept: Mutex<HashMap<PathBuf, &'static Page>>,
}

fn resolve(path: &[u8], from_dir: &[u8]) -> Option<PathBuf> {
    let p = Path::new(OsStr::from_bytes(path));
    let p = if p.is_absolute() {
        p.to_path_buf()
    } else {
        Path::new(OsStr::from_bytes(from_dir)).join(p)
    };
    p.canonicalize().ok()
}

impl Store for Files {
    fn kept(&self, path: &[u8], from_dir: &[u8]) -> Option<&'static Page> {
        let real = resolve(path, from_dir)?;
        self.kept.lock().unwrap().get(&real).copied()
    }

    fn load(&self, path: &[u8], from_dir: &[u8]) -> Result<&'static Page, Refusal> {
        let real =
            resolve(path, from_dir).ok_or_else(|| Refusal::at("a page that cannot be found", 0))?;
        let mut kept = self.kept.lock().unwrap();
        if let Some(page) = kept.get(&real) {
            return Ok(page);
        }
        let source = std::fs::read(&real)
            .map_err(|e| Refusal::at(format!("a page that cannot be read: {e}"), 0))?;
        let page: &'static Page = Box::leak(Box::new(compile_page(
            real.as_os_str().as_bytes(),
            &source,
        )?));
        kept.insert(real, page);
        Ok(page)
    }
}

/// A store that keeps nothing: every include waits for its page, which is read again. It refuses
/// once it has loaded more pages than the corpus could ask for, so a request that asks for the
/// same page forever fails instead of running on.
#[derive(Default)]
struct Forgetful {
    files: Files,
    loads: std::sync::atomic::AtomicU64,
}

impl Store for Forgetful {
    fn kept(&self, _path: &[u8], _from_dir: &[u8]) -> Option<&'static Page> {
        None
    }

    fn load(&self, path: &[u8], from_dir: &[u8]) -> Result<&'static Page, Refusal> {
        let n = self
            .loads
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if n >= 10_000 {
            return Err(Refusal::at("the same page asked for over and over", 0));
        }
        self.files.load(path, from_dir)
    }
}

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/pages")
        .canonicalize()
        .unwrap()
}

#[test]
fn every_page_gives_phps_bytes() {
    every_page_on(&Files::default());
}

#[test]
fn every_page_runs_on_a_store_that_keeps_nothing() {
    every_page_on(&Forgetful::default());
}

fn every_page_on(store: &impl Store) {
    let dir = corpus();
    let mut pages: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension() == Some(OsStr::new("php")))
        .collect();
    pages.sort();
    assert!(
        pages.len() >= 13,
        "the corpus is missing pages: {}",
        pages.len()
    );
    let mut wrong = Vec::new();
    for page in &pages {
        let want = std::fs::read(page.with_extension("out")).unwrap();
        let entry = store.load(page.as_os_str().as_bytes(), dir.as_os_str().as_bytes());
        let mut got = Vec::new();
        let result = entry.and_then(|p| whp::run(store, p, &mut got));
        match result {
            Ok(_) if got == want => {}
            Ok(_) => wrong.push(format!(
                "{}: differs at byte {}",
                page.display(),
                got.iter()
                    .zip(&want)
                    .position(|(a, b)| a != b)
                    .unwrap_or(got.len().min(want.len()))
            )),
            Err(r) => wrong.push(format!("{}: {r}", page.display())),
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} pages:\n{}",
        wrong.len(),
        pages.len(),
        wrong.join("\n")
    );
}

#[test]
fn a_float_is_refused_and_never_printed() {
    let store = Files::default();
    let page: &'static Page = Box::leak(Box::new(
        compile_page(b"/t.php", b"<?php echo 'a'; echo 7 / 2;").unwrap(),
    ));
    let mut got = Vec::new();
    let err = whp::run(&store, page, &mut got).unwrap_err();
    assert_eq!(got, b"a");
    assert!(err.what.contains("float"), "{err}");
}

#[test]
fn a_class_is_refused_before_anything_runs() {
    let err = compile_page(b"/t.php", b"<?php echo 'a'; class A {}").unwrap_err();
    assert!(err.what.contains("classes"), "{err}");
}

/// The line a refusal page says whp refuses it at, if it says one: the line holding
/// `// refused here`, or N where a line says `// refused at line N`.
fn refused_at(src: &str) -> Option<u32> {
    src.lines()
        .position(|l| l.contains("// refused here"))
        .map(|n| n as u32 + 1)
        .or_else(|| {
            src.lines()
                .find_map(|l| l.split("// refused at line ").nth(1))
                .and_then(|n| n.trim().parse().ok())
        })
}

/// Every page of the refusal corpus gives PHP's bytes, as `tests/refuse/<page>.out` holds them
/// with an exit status of 0, or is refused at the line it marks.
#[test]
fn every_refusal_page_gives_phps_bytes_or_is_refused_at_its_line() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/refuse")
        .canonicalize()
        .unwrap();
    let mut pages: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension() == Some(OsStr::new("php")))
        .collect();
    pages.sort();
    assert!(pages.len() >= 100, "the refusal corpus is missing pages");
    let mut wrong = Vec::new();
    for page in &pages {
        let src = std::fs::read_to_string(page).unwrap();
        let want = std::fs::read(page.with_extension("out")).unwrap();
        let exit = std::fs::read_to_string(page.with_extension("exit")).unwrap();
        let store = Files::default();
        let mut got = Vec::new();
        let result = store
            .load(page.as_os_str().as_bytes(), dir.as_os_str().as_bytes())
            .and_then(|p| whp::run(&store, p, &mut got));
        let name = page.file_stem().unwrap().to_string_lossy();
        match (refused_at(&src), result) {
            (Some(line), Err(r)) if r.line == line => {}
            (Some(line), Err(r)) => {
                wrong.push(format!("{name}: refused at {}, not {line}: {r}", r.line))
            }
            (Some(line), Ok(_)) => {
                wrong.push(format!("{name}: ran, where it is refused at {line}"))
            }
            (None, Ok(_)) if got == want && exit.trim() == "0" => {}
            (None, Ok(_)) => wrong.push(format!(
                "{name}: gave {:?}, where PHP gave {:?}",
                String::from_utf8_lossy(&got),
                String::from_utf8_lossy(&want)
            )),
            (None, Err(r)) => wrong.push(format!(
                "{name}: refused, where PHP's bytes are wanted: {r}"
            )),
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} pages:\n{}",
        wrong.len(),
        pages.len(),
        wrong.join("\n")
    );
}
