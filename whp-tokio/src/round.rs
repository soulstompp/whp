// Copyright (C) 2026 Kenneth Allen Flegal and DappleAI
// SPDX-License-Identifier: GPL-3.0-or-later

//! The round's counters, read around each request: the polls the stack gave its task, the files
//! the store read and the bytes the process read, and the heap the request's task held and left
//! behind.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::task::{Context, Poll};

use crate::Args;
use crate::host;
use crate::store::FileStore;

/// Counts the heap's live bytes, for the process and for the task being polled.
pub struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

/// A task's live bytes and their peak.
#[derive(Default)]
struct TaskCount {
    live: AtomicIsize,
    peak: AtomicIsize,
}

impl TaskCount {
    fn add(&self, n: isize) {
        let v = self.live.fetch_add(n, Ordering::Relaxed) + n;
        self.peak.fetch_max(v, Ordering::Relaxed);
    }
}

thread_local! {
    /// The task this thread is polling, if it is a counted one.
    static TASK: Cell<*const TaskCount> = const { Cell::new(std::ptr::null()) };
}

fn add(n: isize) {
    LIVE.fetch_add(n, Ordering::Relaxed);
    let _ = TASK.try_with(|t| {
        let p = t.get();
        if !p.is_null() {
            // SAFETY: the pointer is set only while its task is polled, and the task holds it.
            unsafe { (*p).add(n) }
        }
    });
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            add(layout.size() as isize);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            add(layout.size() as isize);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        add(-(layout.size() as isize));
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            add(new_size as isize - layout.size() as isize);
        }
        p
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// A future that counts how often the stack polls it, and whose allocations count to its task.
struct Counted<F> {
    inner: Pin<Box<F>>,
    polls: Arc<AtomicU32>,
    count: Arc<TaskCount>,
}

impl<F: Future> Future for Counted<F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        self.polls.fetch_add(1, Ordering::Relaxed);
        let count: *const TaskCount = &*self.count;
        let before = TASK.with(|t| t.replace(count));
        let r = self.inner.as_mut().poll(cx);
        TASK.with(|t| t.set(before));
        r
    }
}

/// One request's row of the round.
#[derive(Debug, Default, Clone)]
pub struct Row {
    pub request: u64,
    pub polls: u32,
    pub page_reads: u64,
    pub page_bytes: u64,
    pub rchar: i64,
    pub kept_bytes: i64,
    pub peak_bytes: i64,
    pub block_bytes: u64,
    pub out_bytes: u64,
    pub calls: u64,
    pub call_block_max: u64,
    pub process_live_after: i64,
    pub workers: u64,
    pub teardown_waits: u64,
    pub refused: bool,
}

/// The process's `rchar`, and the bytes reading it took.
fn rchar() -> (i64, i64) {
    let text = std::fs::read_to_string("/proc/self/io").unwrap_or_default();
    let n = text
        .lines()
        .find_map(|l| l.strip_prefix("rchar: "))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(-1);
    (n, text.len() as i64)
}

/// One request on its own task, writing its page to stdout, its row read around it.
async fn request(
    store: &'static FileStore,
    page: &'static str,
    n: u64,
) -> (Result<(), whp::Refusal>, Row) {
    let polls = Arc::new(AtomicU32::new(0));
    let count = Arc::new(TaskCount::default());
    let task_count = count.clone();
    let task = Counted {
        inner: Box::pin(async move {
            let (reads0, bytes0) = store.reads();
            let (rchar0, read_len) = rchar();
            let live0 = task_count.live.load(Ordering::Relaxed);
            task_count.peak.store(live0, Ordering::Relaxed);
            let mut out = std::io::BufWriter::new(std::io::stdout());
            let result = host::page(store, page, &mut out);
            drop(out);
            let live1 = task_count.live.load(Ordering::Relaxed);
            let peak = task_count.peak.load(Ordering::Relaxed);
            let (rchar1, _) = rchar();
            let (reads1, bytes1) = store.reads();
            let row = Row {
                request: n,
                page_reads: reads1 - reads0,
                page_bytes: bytes1 - bytes0,
                rchar: rchar1 - rchar0 - read_len,
                kept_bytes: (live1 - live0) as i64,
                peak_bytes: (peak - live0) as i64,
                block_bytes: result.as_ref().map_or(0, |o| o.block_bytes as u64),
                out_bytes: result.as_ref().map_or(0, |o| o.out_bytes),
                calls: result.as_ref().map_or(0, |o| o.calls),
                call_block_max: result.as_ref().map_or(0, |o| o.call_block_max as u64),
                refused: result.is_err(),
                ..Row::default()
            };
            (result.map(|_| ()), row)
        }),
        polls: polls.clone(),
        count,
    };
    let (result, mut row) = tokio::spawn(task).await.expect("the request's task");
    row.polls = polls.load(Ordering::Relaxed);
    (result, row)
}

/// Reads the process's heap once the stack has let every task go: it finishes letting a task go
/// after its handle has the output.
async fn settle(row: &mut Row) {
    let m = tokio::runtime::Handle::current().metrics();
    let mut waits = 0u64;
    while m.num_alive_tasks() > 0 {
        waits += 1;
        tokio::task::yield_now().await;
    }
    row.process_live_after = LIVE.load(Ordering::Relaxed) as i64;
    row.workers = m.num_workers() as u64;
    row.teardown_waits = waits;
}

/// Runs the requests the arguments ask for, and writes their round.
pub async fn drive(
    store: &'static FileStore,
    page: &'static str,
    args: &Args,
) -> Result<(), whp::Refusal> {
    let mut rows = Vec::with_capacity(args.repeat as usize);
    let mut failed = None;
    for n in 0..args.repeat {
        let (result, mut row) = request(store, page, n).await;
        settle(&mut row).await;
        rows.push(row);
        if let Err(r) = result {
            failed = Some(r);
            break;
        }
    }
    if let Some(file) = &args.round {
        write(file, &rows).map_err(|e| whp::Refusal::at(format!("writing the round: {e}"), 0))?;
    }
    match failed {
        Some(r) => Err(r),
        None => Ok(()),
    }
}

/// The round as JSON lines.
fn write(file: &str, rows: &[Row]) -> std::io::Result<()> {
    let mut s = String::new();
    for r in rows {
        let _ = writeln!(
            s,
            "{{\"request\":{},\"polls\":{},\"page_reads\":{},\"page_bytes\":{},\"rchar\":{},\"kept_bytes\":{},\"peak_bytes\":{},\"block_bytes\":{},\"out_bytes\":{},\"calls\":{},\"call_block_max\":{},\"process_live_after\":{},\"workers\":{},\"teardown_waits\":{},\"refused\":{}}}",
            r.request,
            r.polls,
            r.page_reads,
            r.page_bytes,
            r.rchar,
            r.kept_bytes,
            r.peak_bytes,
            r.block_bytes,
            r.out_bytes,
            r.calls,
            r.call_block_max,
            r.process_live_after,
            r.workers,
            r.teardown_waits,
            r.refused
        );
    }
    std::fs::write(file, s)
}
