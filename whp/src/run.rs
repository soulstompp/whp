// Copyright (C) 2026 Kenneth Allen Flegal and DappleAI
// SPDX-License-Identifier: GPL-3.0-or-later

//! A request: one page run to its end. What the request makes lives in blocks: the request's own,
//! dropped whole when the page has gone out, and one for each call, dropped when the call returns.
//! What a call gives back is copied out of its block first.
//!
//! PHP's frames are kept in the request's block, never on Rust's stack: a call pushes a frame and
//! a return pops one, so a page may call as deep as its memory allows, and the request's state is
//! all in the machine at every instruction. So a request stops at any instruction that must wait,
//! hands its host what it waits on, and resumes there.

use std::io::Write;
use std::time::Duration;

use bumpalo::Bump;
use bumpalo::collections::Vec as BVec;
use hashbrown::{DefaultHashBuilder, HashMap, HashSet};

use crate::Refusal;
use crate::builtins;
use crate::compile::{Code, Function, Lit, Op, Page};
use crate::parse::{BinOp, IncludeKind, UnOp};
use crate::store::Store;
use crate::value::{
    Array, Key, Value, compare, identical, loose_equal, release, share, string_fits, to_bytes,
    to_int_operand, to_key, truthy,
};

/// What a request did, read when it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcome {
    /// The bytes of its page.
    pub out_bytes: u64,
    /// The bytes the request's own block took.
    pub block_bytes: usize,
    /// The calls of the page's own functions it made.
    pub calls: u64,
    /// The bytes the largest block of a call took.
    pub call_block_max: usize,
}

/// What a request waits on before it can go on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wait {
    /// `sleep()` or `usleep()`: the request waits this long.
    Sleep(Duration),
    /// A page the store has not kept: the host loads it ([`Store::load`]), hands it to the
    /// request ([`Request::loaded`]), and resumes the request, which includes it.
    Page {
        /// The page's path, as the page that includes it writes it.
        path: Vec<u8>,
        /// The directory of the page that includes it.
        from_dir: Vec<u8>,
    },
}

/// Where a request stands after it ran: waiting, or done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// It waits on this; its page so far has gone to the sink.
    Wait(Wait),
    /// It ended, its whole page gone to the sink.
    Done(Outcome),
}

/// One request, which its host drives: [`Request::resume`] runs it until it ends or must wait.
/// The request owns everything it has made, so it moves whole between threads while it waits.
pub struct Request<'s, S: Store + ?Sized> {
    // The machine borrows the block, so it is declared first, and dropped first.
    machine: Machine<'static, 's, S>,
    block: Box<Bump>,
}

// SAFETY: every reference inside a request points into its own blocks, which move with it, into a
// page kept for the life of the process, which is never written, or at the store, which is Sync.
// No reference into a request's blocks is ever handed out of it.
unsafe impl<S: Store + Sync + ?Sized> Send for Request<'_, S> {}

impl<'s, S: Store + ?Sized> Request<'s, S> {
    /// A request for `entry`, which its host has fetched.
    pub fn new(store: &'s S, entry: &'static Page) -> Self {
        let block = Box::new(Bump::new());
        // SAFETY: the block is boxed, so it stays put, and the request drops the machine before
        // the block; nothing made in it leaves the request.
        let request: &'static Bump = unsafe { &*(&*block as *const Bump) };
        let mut machine = Machine::new(request, store);
        machine.start(entry);
        Request { machine, block }
    }

    /// Runs the request on from where it stopped, writing its page to `sink`, until it ends or
    /// must wait.
    pub fn resume(&mut self, sink: &mut dyn Write) -> Result<Step, Refusal> {
        match self.machine.run(sink)? {
            Some(wait) => Ok(Step::Wait(wait)),
            None => Ok(Step::Done(Outcome {
                out_bytes: self.machine.out_bytes,
                block_bytes: self.block.allocated_bytes(),
                calls: self.machine.calls,
                call_block_max: self.machine.block_bytes_max,
            })),
        }
    }

    /// Hands the request the page it waited on, which its include runs when it resumes. The
    /// request never asks the store for it again, so a store need not keep what it loads.
    pub fn loaded(&mut self, page: &'static Page) {
        self.machine.loaded = Some(page);
    }

    /// A refusal met while the host did what the request waited on, such as a page it could not
    /// load, placed at the request's include.
    pub fn failed(&self, r: Refusal) -> Refusal {
        match self.machine.waiting_at {
            Some((path, line)) if r.line == 0 && r.file.is_empty() => Refusal {
                line,
                ..r.in_file(path)
            },
            _ => r,
        }
    }
}

/// Runs `entry` as one request to its end, writing its page to `sink`: each sleep blocks the
/// thread, and each page is loaded where the request waits for it.
pub fn run<S: Store + ?Sized>(
    store: &S,
    entry: &'static Page,
    sink: &mut dyn Write,
) -> Result<Outcome, Refusal> {
    let mut request = Request::new(store, entry);
    loop {
        match request.resume(sink)? {
            Step::Done(outcome) => return Ok(outcome),
            Step::Wait(Wait::Sleep(d)) => std::thread::sleep(d),
            Step::Wait(Wait::Page { path, from_dir }) => {
                let page = store
                    .load(&path, &from_dir)
                    .map_err(|r| request.failed(r))?;
                request.loaded(page);
            }
        }
    }
}

/// Whether `p` points into memory `block` has handed out.
fn holds(block: &Bump, p: *const u8) -> bool {
    let p = p as usize;
    // SAFETY: nothing is allocated in `block` while its chunks are walked, and they are only
    // compared with, never read.
    unsafe { block.iter_allocated_chunks_raw() }
        .any(|(start, len)| (start as usize..start as usize + len).contains(&p))
}

type Map<'r, K, V> = HashMap<K, V, DefaultHashBuilder, &'r Bump>;

/// The first number of calls in progress at which PHP 8.5 runs out of its default `memory_limit`,
/// on the cheapest call that can stop: a call that would make this many is refused.
const DEEPEST: usize = 915_135;

struct Scope<'r> {
    vars: std::cell::RefCell<Map<'r, &'r [u8], Value<'r>>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameKind {
    Main,
    Function,
    Include,
}

#[derive(Clone, Copy)]
struct Frame<'r> {
    page: &'static Page,
    code: &'static Code,
    pc: usize,
    scope: &'r Scope<'r>,
    base: usize,
    walks: usize,
    kind: FrameKind,
}

/// A `foreach` walking an array it holds.
struct Walk<'r> {
    array: &'r Array<'r>,
    pos: usize,
}

/// What one instruction leaves the machine to do.
enum Flow {
    Go,
    Done,
    Wait(Wait),
}

struct Machine<'r, 's, S: Store + ?Sized> {
    request: &'r Bump,
    /// One block for each call in progress, the innermost last. Each is boxed so it stays put
    /// while the list grows: what a call makes holds a reference to its block.
    #[allow(clippy::vec_box)]
    blocks: Vec<Box<Bump>>,
    /// Blocks overwritten and kept instead of dropped, so a read of one after its call is seen.
    #[cfg(feature = "poison")]
    #[allow(clippy::vec_box)]
    poisoned: Vec<Box<Bump>>,
    calls: u64,
    /// The calls of the page's own functions in progress.
    live_calls: usize,
    block_bytes_max: usize,
    store: &'s S,
    out_bytes: u64,
    stack: BVec<'r, Value<'r>>,
    frames: BVec<'r, Frame<'r>>,
    walks: BVec<'r, Walk<'r>>,
    functions: Map<'r, &'static [u8], (&'static Page, &'static Function)>,
    included: HashSet<&'static [u8], DefaultHashBuilder, &'r Bump>,
    /// Whether the entry page has been declared: its first run does it.
    started: bool,
    /// The page the host loaded while the request waited, for the include that waited on it.
    loaded: Option<&'static Page>,
    /// The page and line of the include the request waits at.
    waiting_at: Option<(&'static [u8], u32)>,
}

fn lit<'r>(l: &'static Lit) -> Value<'r> {
    match l {
        Lit::Null => Value::Null,
        Lit::Bool(b) => Value::Bool(*b),
        Lit::Int(i) => Value::Int(*i),
        Lit::Str(s) => Value::Str(s),
    }
}

impl<'r, 's, S: Store + ?Sized> Machine<'r, 's, S> {
    fn new(bump: &'r Bump, store: &'s S) -> Self {
        Machine {
            request: bump,
            blocks: Vec::new(),
            #[cfg(feature = "poison")]
            poisoned: Vec::new(),
            calls: 0,
            live_calls: 0,
            block_bytes_max: 0,
            store,
            out_bytes: 0,
            stack: BVec::new_in(bump),
            frames: BVec::new_in(bump),
            walks: BVec::new_in(bump),
            functions: HashMap::with_hasher_in(DefaultHashBuilder::default(), bump),
            included: HashSet::with_hasher_in(DefaultHashBuilder::default(), bump),
            started: false,
            loaded: None,
            waiting_at: None,
        }
    }

    /// The block what is made now goes into: the innermost call's, or the request's.
    fn cur(&self) -> &'r Bump {
        match self.blocks.last() {
            // SAFETY: a call's block is dropped only when its call returns, after the call's
            // frame, stack, walks and scope are gone and its return value is copied out of it
            // (`evacuate`); nothing else made in a call outlives it, so nothing read later points
            // into a dropped block.
            Some(b) => unsafe { &*(&**b as *const Bump) },
            None => self.request,
        }
    }

    /// `v`, with whatever of it lies in `from` copied into the current block. A caller's array
    /// the call wrote while it alone held it is rewritten in place.
    fn evacuate(&self, v: Value<'r>, from: &Bump) -> Value<'r> {
        match v {
            Value::Str(s) if holds(from, s.as_ptr()) => Value::Str(self.cur().alloc_slice_copy(s)),
            Value::Array(a) if holds(from, a as *const Array<'r> as *const u8) => {
                Value::Array(a.moved(self.cur(), |k, e| {
                    (self.evacuate_key(k, from), self.evacuate(e, from))
                }))
            }
            Value::Array(a) => {
                a.rewrite(|k, e| (self.evacuate_key(k, from), self.evacuate(e, from)));
                v
            }
            v => v,
        }
    }

    fn evacuate_key(&self, k: Key<'r>, from: &Bump) -> Key<'r> {
        match k {
            Key::Str(s) if holds(from, s.as_ptr()) => Key::Str(self.cur().alloc_slice_copy(s)),
            k => k,
        }
    }

    /// Lets a returned call's block go.
    fn drop_block(&mut self, block: Box<Bump>) {
        #[cfg(feature = "poison")]
        {
            // SAFETY: the chunks are the block's own memory, which nothing reads any more; they
            // are overwritten through the block's raw pointers and kept until the request ends.
            for (p, len) in unsafe { block.iter_allocated_chunks_raw() } {
                unsafe { std::ptr::write_bytes(p, 0xA5, len) };
            }
            self.poisoned.push(block);
        }
        #[cfg(not(feature = "poison"))]
        drop(block);
    }

    fn new_scope(&self) -> &'r Scope<'r> {
        self.cur().alloc(Scope {
            vars: std::cell::RefCell::new(HashMap::with_hasher_in(
                DefaultHashBuilder::default(),
                self.cur(),
            )),
        })
    }

    /// Makes a page's functions callable, as PHP declares them when the page is reached.
    fn declare(&mut self, page: &'static Page, line: u32) -> Result<(), Refusal> {
        for f in &page.functions {
            if self.functions.contains_key(&*f.key) {
                return Err(Refusal::at(
                    format!("declaring {}() again", String::from_utf8_lossy(&f.name)),
                    line,
                ));
            }
            self.functions.insert(&f.key, (page, f));
        }
        Ok(())
    }

    fn pop(&mut self) -> Value<'r> {
        self.stack.pop().expect("an operand")
    }

    fn top(&self) -> Value<'r> {
        *self.stack.last().expect("an operand")
    }

    fn frame(&mut self) -> &mut Frame<'r> {
        self.frames.last_mut().expect("a frame")
    }

    fn write(&mut self, sink: &mut dyn Write, bytes: &[u8], line: u32) -> Result<(), Refusal> {
        self.out_bytes += bytes.len() as u64;
        sink.write_all(bytes)
            .map_err(|e| Refusal::at(format!("writing the page out: {e}"), line))
    }

    /// Sets the entry page up to run. Its declaration waits for its first instruction, so that a
    /// page declaring a function twice is refused when it runs, as PHP does.
    fn start(&mut self, entry: &'static Page) {
        let scope = self.new_scope();
        self.frames.push(Frame {
            page: entry,
            code: &entry.main,
            pc: 0,
            scope,
            base: 0,
            walks: 0,
            kind: FrameKind::Main,
        });
    }

    /// Runs on until the request ends (none) or must wait.
    fn run(&mut self, sink: &mut dyn Write) -> Result<Option<Wait>, Refusal> {
        if !self.started {
            self.started = true;
            let entry = self.frames[0].page;
            self.declare(entry, 0).map_err(|r| r.in_file(&entry.path))?;
            self.included.insert(&entry.path);
        }
        loop {
            let Some(&f) = self.frames.last() else {
                return Ok(None);
            };
            let op = f.code.ops[f.pc];
            let line = f.code.lines[f.pc];
            self.frame().pc += 1;
            match self.step(op, f, line, sink) {
                Ok(Flow::Go) => {}
                Ok(Flow::Done) => return Ok(None),
                Ok(Flow::Wait(w)) => return Ok(Some(w)),
                Err(r) => return Err(r.in_file(&f.page.path)),
            }
        }
    }

    /// One instruction.
    fn step(
        &mut self,
        op: Op,
        f: Frame<'r>,
        line: u32,
        sink: &mut dyn Write,
    ) -> Result<Flow, Refusal> {
        let name = |i: u32| -> &'static [u8] { &f.page.names[i as usize] };
        match op {
            Op::Lit(i) => self.stack.push(lit(&f.page.lits[i as usize])),
            Op::Load(n) => {
                let v = f.scope.vars.borrow().get(name(n)).copied();
                let Some(v) = v else {
                    return Err(Refusal::at(
                        format!(
                            "reading ${}, which is not set",
                            String::from_utf8_lossy(name(n))
                        ),
                        line,
                    ));
                };
                share(v);
                self.stack.push(v);
            }
            Op::LoadQuiet(n) => {
                let v = f
                    .scope
                    .vars
                    .borrow()
                    .get(name(n))
                    .copied()
                    .unwrap_or(Value::Null);
                share(v);
                self.stack.push(v);
            }
            Op::Store(n) => {
                let v = self.top();
                share(v);
                let old = f.scope.vars.borrow_mut().insert(name(n), v);
                if let Some(old) = old {
                    release(old);
                }
            }
            Op::Dim | Op::DimQuiet => {
                let key = self.pop();
                let container = self.pop();
                let v = self.element(container, key, op == Op::DimQuiet, line)?;
                share(v);
                release(container);
                release(key);
                self.stack.push(v);
            }
            Op::StoreDim { var, depth, append } => {
                let v = self.top();
                let n_keys = if append { depth - 1 } else { depth } as usize;
                let at = self.stack.len() - 1 - n_keys;
                let keys: Vec<Value<'r>> = self.stack[at..self.stack.len() - 1].to_vec();
                self.stack.remove(at + n_keys);
                self.stack.truncate(at);
                share(v);
                self.store_dim(f.scope, name(var), &keys, (!append).then_some(()), v, line)?;
                for k in keys {
                    release(k);
                }
                self.stack.push(v);
            }
            Op::IncDec { var, pre, inc } => {
                let old = f.scope.vars.borrow().get(name(var)).copied();
                let Some(old) = old else {
                    return Err(Refusal::at(
                        format!(
                            "++ or -- on ${}, which is not set",
                            String::from_utf8_lossy(name(var))
                        ),
                        line,
                    ));
                };
                let new = match old {
                    Value::Null if inc => Value::Int(1),
                    Value::Int(i) => {
                        let n = if inc {
                            i.checked_add(1)
                        } else {
                            i.checked_sub(1)
                        };
                        Value::Int(n.ok_or_else(|| {
                            Refusal::at("an int stepped past its range into a float", line)
                        })?)
                    }
                    _ => return Err(Refusal::at("++ or -- on something other than an int", line)),
                };
                f.scope.vars.borrow_mut().insert(name(var), new);
                self.stack.push(if pre { new } else { old });
            }
            Op::NotNull => {
                let v = self.pop();
                release(v);
                self.stack.push(Value::Bool(!matches!(v, Value::Null)));
            }
            Op::Unset { var, depth } => {
                let at = self.stack.len() - depth as usize;
                let keys: Vec<Value<'r>> = self.stack[at..].to_vec();
                self.stack.truncate(at);
                self.unset(f.scope, name(var), &keys, line)?;
                for k in keys {
                    release(k);
                }
            }
            Op::Echo => {
                let v = self.pop();
                let bytes = to_bytes(self.cur(), v, line)?;
                self.write(sink, bytes, line)?;
                release(v);
            }
            Op::Print => {
                let v = self.pop();
                let bytes = to_bytes(self.cur(), v, line)?;
                self.write(sink, bytes, line)?;
                release(v);
                self.stack.push(Value::Int(1));
            }
            Op::Pop => {
                let v = self.pop();
                release(v);
            }
            Op::Dup => {
                let v = self.top();
                share(v);
                self.stack.push(v);
            }
            Op::Bin(op) => {
                let b = self.pop();
                let a = self.pop();
                let r = self.binary(op, a, b, line)?;
                release(a);
                release(b);
                self.stack.push(r);
            }
            Op::Unary(op) => {
                let a = self.pop();
                let r = match op {
                    UnOp::Not => Value::Bool(!truthy(a)),
                    UnOp::Neg => {
                        Value::Int(to_int_operand(a, line)?.checked_neg().ok_or_else(|| {
                            Refusal::at("an int negated past its range into a float", line)
                        })?)
                    }
                    UnOp::Plus => Value::Int(to_int_operand(a, line)?),
                };
                release(a);
                self.stack.push(r);
            }
            Op::ToBool => {
                let a = self.pop();
                release(a);
                self.stack.push(Value::Bool(truthy(a)));
            }
            Op::Concat(n) => {
                let at = self.stack.len() - n as usize;
                let first: &'r [u8] = match self.stack.get(at) {
                    Some(&v) => to_bytes(self.cur(), v, line)?,
                    None => b"",
                };
                let mut rest = Vec::new();
                for k in at + 1..self.stack.len() {
                    rest.extend_from_slice(to_bytes(self.cur(), self.stack[k], line)?);
                    string_fits(rest.len(), line)?;
                }
                let joined = self.concat(first, &rest, line)?;
                for k in at..self.stack.len() {
                    release(self.stack[k]);
                }
                self.stack.truncate(at);
                self.stack.push(Value::Str(joined));
            }
            Op::Compound { var, op, keep } => {
                let b = self.pop();
                let held = f.scope.vars.borrow().get(name(var)).copied();
                let Some(a) = held else {
                    return Err(Refusal::at(
                        format!(
                            "reading ${}, which is not set",
                            String::from_utf8_lossy(name(var))
                        ),
                        line,
                    ));
                };
                let r = match op {
                    BinOp::Concat => {
                        let (x, y) = (
                            to_bytes(self.cur(), a, line)?,
                            to_bytes(self.cur(), b, line)?,
                        );
                        Value::Str(self.concat(x, y, line)?)
                    }
                    op => self.binary(op, a, b, line)?,
                };
                release(b);
                if let Some(old) = f.scope.vars.borrow_mut().insert(name(var), r) {
                    release(old);
                }
                if keep {
                    share(r);
                    self.stack.push(r);
                }
            }
            Op::Jump(t) => self.frame().pc = t as usize,
            Op::JumpIfFalse(t) => {
                let v = self.pop();
                release(v);
                if !truthy(v) {
                    self.frame().pc = t as usize;
                }
            }
            Op::JumpIfTrue(t) => {
                let v = self.pop();
                release(v);
                if truthy(v) {
                    self.frame().pc = t as usize;
                }
            }
            Op::JumpIfTruthyKeep(t) => {
                if truthy(self.top()) {
                    self.frame().pc = t as usize;
                } else {
                    let v = self.pop();
                    release(v);
                }
            }
            Op::JumpIfNotNullKeep(t) => {
                if !matches!(self.top(), Value::Null) {
                    self.frame().pc = t as usize;
                } else {
                    self.pop();
                }
            }
            Op::JumpIfBound(p, t) => {
                if f.scope.vars.borrow().contains_key(name(p)) {
                    self.frame().pc = t as usize;
                }
            }
            Op::Call { name: n, argc } => {
                if let Some(w) = self.call(name(n), argc as usize, line)? {
                    return Ok(Flow::Wait(w));
                }
            }
            Op::Return => return self.ret(),
            Op::NewArray => self.stack.push(Value::Array(Array::new(self.cur()))),
            Op::ArrayPush => {
                let v = self.pop();
                let Value::Array(a) = self.top() else {
                    unreachable!("an array literal")
                };
                a.push(v, line)?;
            }
            Op::ArraySet => {
                let v = self.pop();
                let k = self.pop();
                let key = to_key(k, line)?;
                let Value::Array(a) = self.top() else {
                    unreachable!("an array literal")
                };
                if let Some(old) = a.set(key, v, line)? {
                    release(old);
                }
            }
            Op::IterInit => {
                let v = self.pop();
                let Value::Array(array) = v else {
                    return Err(Refusal::at(
                        "foreach over something other than an array",
                        line,
                    ));
                };
                self.walks.push(Walk { array, pos: 0 });
            }
            Op::IterNext { key, value, end } => {
                let walk = self.walks.last_mut().expect("a walk");
                let next = loop {
                    let (entry, n) = walk.array.at(walk.pos);
                    if walk.pos >= n {
                        break None;
                    }
                    walk.pos += 1;
                    if entry.is_some() {
                        break entry;
                    }
                };
                match next {
                    None => self.frame().pc = end as usize,
                    Some((k, v)) => {
                        share(v);
                        let mut vars = f.scope.vars.borrow_mut();
                        if let Some(old) = vars.insert(name(value), v) {
                            release(old);
                        }
                        if let Some(key) = key
                            && let Some(old) = vars.insert(name(key), k.value())
                        {
                            release(old);
                        }
                    }
                }
            }
            Op::PopIter => {
                let walk = self.walks.pop().expect("a walk");
                release(Value::Array(walk.array));
            }
            Op::Include(kind) => {
                let path = to_bytes(self.cur(), self.top(), line)?;
                let Some(page) = self
                    .loaded
                    .take()
                    .or_else(|| self.store.kept(path, &f.page.dir))
                else {
                    // The page is loaded while the request waits; then this include runs again.
                    self.frame().pc -= 1;
                    self.waiting_at = Some((&f.page.path, line));
                    return Ok(Flow::Wait(Wait::Page {
                        path: path.to_vec(),
                        from_dir: f.page.dir.to_vec(),
                    }));
                };
                let p = self.pop();
                release(p);
                let once = matches!(kind, IncludeKind::IncludeOnce | IncludeKind::RequireOnce);
                if once && self.included.contains(&*page.path) {
                    self.stack.push(Value::Bool(true));
                } else {
                    self.included.insert(&page.path);
                    self.declare(page, line)?;
                    let (base, walks) = (self.stack.len(), self.walks.len());
                    self.frames.push(Frame {
                        page,
                        code: &page.main,
                        pc: 0,
                        scope: f.scope,
                        base,
                        walks,
                        kind: FrameKind::Include,
                    });
                }
            }
            Op::Unmatched => return Err(Refusal::at("a match with no arm for its value", line)),
        }
        Ok(Flow::Go)
    }

    /// Reads `container[key]`. Quietly, a missing element reads null.
    fn element(
        &self,
        container: Value<'r>,
        key: Value<'r>,
        quiet: bool,
        line: u32,
    ) -> Result<Value<'r>, Refusal> {
        match container {
            Value::Array(a) => {
                let k = to_key(key, line)?;
                match a.get(k) {
                    Some(v) => Ok(v),
                    None if quiet => Ok(Value::Null),
                    None => Err(Refusal::at("reading an array key that is not set", line)),
                }
            }
            Value::Str(s) => {
                let offset = match key {
                    Value::Int(i) => Some(i),
                    Value::Str(k) => match crate::value::numeric(k) {
                        crate::value::Numeric::Int(i)
                            if !k.iter().any(|b| b.is_ascii_whitespace()) =>
                        {
                            Some(i)
                        }
                        _ => None,
                    },
                    _ => None,
                };
                let at = offset.and_then(|i| {
                    if i < 0 {
                        (s.len() as i64).checked_add(i)
                    } else {
                        Some(i)
                    }
                });
                match at.filter(|&i| i >= 0 && (i as usize) < s.len()) {
                    Some(i) => Ok(Value::Str(
                        self.cur().alloc_slice_copy(&s[i as usize..i as usize + 1]),
                    )),
                    None if quiet => Ok(Value::Null),
                    None => Err(Refusal::at("a string offset that is not there", line)),
                }
            }
            _ if quiet => Ok(Value::Null),
            _ => Err(Refusal::at(
                "reading an offset of something other than an array or a string",
                line,
            )),
        }
    }

    /// The array a holder may write: its own, copied first if another holder shares it, or a new
    /// one where it held null.
    fn writable(&self, v: Option<Value<'r>>, line: u32) -> Result<(&'r Array<'r>, bool), Refusal> {
        match v {
            None | Some(Value::Null) => Ok((Array::new(self.cur()), true)),
            Some(Value::Array(a)) if a.refs() <= 1 => Ok((a, false)),
            Some(Value::Array(a)) => {
                let copy = a.copy(self.cur());
                release(Value::Array(a));
                Ok((copy, true))
            }
            Some(Value::Str(_)) => Err(Refusal::at("writing to a string offset", line)),
            Some(_) => Err(Refusal::at(
                "writing an element of a value that is not an array",
                line,
            )),
        }
    }

    /// `$var[k1]…[kn] = v`, or `$var[k1]…[] = v` when `last` is none.
    fn store_dim(
        &mut self,
        scope: &'r Scope<'r>,
        var: &'static [u8],
        keys: &[Value<'r>],
        last: Option<()>,
        v: Value<'r>,
        line: u32,
    ) -> Result<(), Refusal> {
        let held = scope.vars.borrow().get(var).copied();
        let (mut arr, fresh) = self.writable(held, line)?;
        if fresh {
            scope.vars.borrow_mut().insert(var, Value::Array(arr));
        }
        let (walk, final_key) = match last {
            Some(()) => (&keys[..keys.len() - 1], Some(keys[keys.len() - 1])),
            None => (keys, None),
        };
        for k in walk {
            let key = to_key(*k, line)?;
            let (child, fresh) = self.writable(arr.get(key), line)?;
            if fresh {
                arr.set(key, Value::Array(child), line)?;
            }
            arr = child;
        }
        match final_key {
            Some(k) => {
                let key = to_key(k, line)?;
                if let Some(old) = arr.set(key, v, line)? {
                    release(old);
                }
            }
            None => arr.push(v, line)?,
        }
        Ok(())
    }

    fn unset(
        &mut self,
        scope: &'r Scope<'r>,
        var: &'static [u8],
        keys: &[Value<'r>],
        line: u32,
    ) -> Result<(), Refusal> {
        if keys.is_empty() {
            if let Some(old) = scope.vars.borrow_mut().remove(var) {
                release(old);
            }
            return Ok(());
        }
        // PHP is silent where the variable or an element on the way holds null or is missing,
        // warns where the variable is not set, and throws or deprecates for anything else.
        let not_an_array = |v: Value<'r>| {
            Refusal::at(
                format!("unset on an offset of {}", crate::value::type_name(v)),
                line,
            )
        };
        let held = scope.vars.borrow().get(var).copied();
        match held {
            None => {
                return Err(Refusal::at(
                    format!(
                        "unset on an offset of ${}, which is not set",
                        String::from_utf8_lossy(var)
                    ),
                    line,
                ));
            }
            Some(Value::Null) => return Ok(()),
            Some(Value::Array(_)) => {}
            Some(v) => return Err(not_an_array(v)),
        }
        let (mut arr, fresh) = self.writable(held, line)?;
        if fresh {
            scope.vars.borrow_mut().insert(var, Value::Array(arr));
        }
        for k in &keys[..keys.len() - 1] {
            let key = to_key(*k, line)?;
            let child = arr.get(key);
            match child {
                None | Some(Value::Null) => return Ok(()),
                Some(Value::Array(_)) => {}
                Some(v) => return Err(not_an_array(v)),
            }
            let (child, fresh) = self.writable(child, line)?;
            if fresh {
                arr.set(key, Value::Array(child), line)?;
            }
            arr = child;
        }
        let key = to_key(keys[keys.len() - 1], line)?;
        if let Some(old) = arr.remove(key) {
            release(old);
        }
        Ok(())
    }

    /// `left` and then `right`, as one string of its own length.
    fn concat(&self, left: &[u8], right: &[u8], line: u32) -> Result<&'r [u8], Refusal> {
        let len = left.len() + right.len();
        string_fits(len, line)?;
        let mut out = BVec::with_capacity_in(len, self.cur());
        out.extend_from_slice(left);
        out.extend_from_slice(right);
        Ok(out.into_bump_slice())
    }

    fn binary(
        &self,
        op: BinOp,
        a: Value<'r>,
        b: Value<'r>,
        line: u32,
    ) -> Result<Value<'r>, Refusal> {
        let int = |r: Option<i64>| {
            r.map(Value::Int)
                .ok_or_else(|| Refusal::at("an int result past its range, a float", line))
        };
        Ok(match op {
            BinOp::Add => int(to_int_operand(a, line)?.checked_add(to_int_operand(b, line)?))?,
            BinOp::Sub => int(to_int_operand(a, line)?.checked_sub(to_int_operand(b, line)?))?,
            BinOp::Mul => int(to_int_operand(a, line)?.checked_mul(to_int_operand(b, line)?))?,
            BinOp::Div => {
                let (x, y) = (to_int_operand(a, line)?, to_int_operand(b, line)?);
                if y == 0 {
                    return Err(Refusal::at("division by zero", line));
                }
                if x.checked_rem(y) != Some(0) {
                    return Err(Refusal::at("a division with a remainder, a float", line));
                }
                int(x.checked_div(y))?
            }
            BinOp::Mod => {
                let (x, y) = (to_int_operand(a, line)?, to_int_operand(b, line)?);
                if y == 0 {
                    return Err(Refusal::at("modulo by zero", line));
                }
                int(x.checked_rem(y))?
            }
            BinOp::Pow => {
                let (x, y) = (to_int_operand(a, line)?, to_int_operand(b, line)?);
                let y =
                    u32::try_from(y).map_err(|_| Refusal::at("a negative power, a float", line))?;
                int(x.checked_pow(y))?
            }
            BinOp::Concat => {
                let (x, y) = (
                    to_bytes(self.cur(), a, line)?,
                    to_bytes(self.cur(), b, line)?,
                );
                Value::Str(self.concat(x, y, line)?)
            }
            BinOp::Equal => Value::Bool(loose_equal(a, b, line)?),
            BinOp::NotEqual => Value::Bool(!loose_equal(a, b, line)?),
            BinOp::Identical => Value::Bool(identical(a, b)),
            BinOp::NotIdentical => Value::Bool(!identical(a, b)),
            BinOp::Less => Value::Bool(compare(a, b, line)? == Some(std::cmp::Ordering::Less)),
            BinOp::LessOrEqual => Value::Bool(matches!(
                compare(a, b, line)?,
                Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
            )),
            BinOp::Greater => Value::Bool(compare(b, a, line)? == Some(std::cmp::Ordering::Less)),
            BinOp::GreaterOrEqual => Value::Bool(matches!(
                compare(b, a, line)?,
                Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
            )),
            BinOp::Spaceship => Value::Int(match compare(a, b, line)? {
                Some(std::cmp::Ordering::Less) => -1,
                Some(std::cmp::Ordering::Equal) => 0,
                _ => 1,
            }),
            BinOp::Xor => Value::Bool(truthy(a) ^ truthy(b)),
            BinOp::And | BinOp::Or => unreachable!("compiled to jumps"),
        })
    }

    /// `sleep(seconds)` and `usleep(microseconds)`: PHP's own functions that wait. Each pushes
    /// what it gives back (0, and null) and hands the wait to the host; a sleep of no time is
    /// no wait.
    fn wait(&mut self, key: &[u8], argc: usize, line: u32) -> Result<Option<Wait>, Refusal> {
        let micros_per = if key == b"sleep" { 1_000_000 } else { 1 };
        let name = String::from_utf8_lossy(key);
        if argc != 1 {
            return Err(Refusal::at(format!("{name}() with {argc} arguments"), line));
        }
        let v = self.pop();
        release(v);
        let Value::Int(n) = v else {
            return Err(Refusal::at(
                format!("{name}() given something other than an int"),
                line,
            ));
        };
        let micros = u64::try_from(n)
            .ok()
            .and_then(|n| n.checked_mul(micros_per))
            .ok_or_else(|| Refusal::at(format!("{name}() given a negative time"), line))?;
        self.stack.push(if key == b"sleep" {
            Value::Int(0)
        } else {
            Value::Null
        });
        Ok((micros > 0).then(|| Wait::Sleep(Duration::from_micros(micros))))
    }

    fn call(
        &mut self,
        key: &'static [u8],
        argc: usize,
        line: u32,
    ) -> Result<Option<Wait>, Refusal> {
        let at = self.stack.len() - argc;
        if let Some(&(page, func)) = self.functions.get(key) {
            if argc < func.required {
                return Err(Refusal::at(
                    format!(
                        "too few arguments to {}()",
                        String::from_utf8_lossy(&func.name)
                    ),
                    line,
                ));
            }
            if self.live_calls + 1 >= DEEPEST {
                return Err(Refusal::at(
                    "a call deeper than PHP's memory_limit allows",
                    line,
                ));
            }
            self.blocks.push(Box::new(Bump::new()));
            self.calls += 1;
            self.live_calls += 1;
            let scope = self.new_scope();
            {
                let mut vars = scope.vars.borrow_mut();
                for (n, arg) in self.stack[at..].iter().enumerate() {
                    match func.params.get(n) {
                        Some(&p) => {
                            vars.insert(&page.names[p as usize], *arg);
                        }
                        None => release(*arg),
                    }
                }
            }
            self.stack.truncate(at);
            let (base, walks) = (self.stack.len(), self.walks.len());
            self.frames.push(Frame {
                page,
                code: &func.code,
                pc: 0,
                scope,
                base,
                walks,
                kind: FrameKind::Function,
            });
            return Ok(None);
        }
        if builtins::waits(key) {
            return self.wait(key, argc, line);
        }
        let Some(builtin) = builtins::find(key) else {
            return Err(Refusal::at(
                format!(
                    "a call to {}(), which whp does not have",
                    String::from_utf8_lossy(key)
                ),
                line,
            ));
        };
        let args: Vec<Value<'r>> = self.stack[at..].to_vec();
        self.stack.truncate(at);
        let r = builtin(self.cur(), &args, line)?;
        for a in args {
            release(a);
        }
        self.stack.push(r);
        Ok(None)
    }

    /// Leaves the current frame with the value on top of its stack. Gives back whether the
    /// request goes on.
    fn ret(&mut self) -> Result<Flow, Refusal> {
        let v = self.pop();
        let f = self.frames.pop().expect("a frame");
        while self.stack.len() > f.base {
            let left = self.pop();
            release(left);
        }
        while self.walks.len() > f.walks {
            let w = self.walks.pop().expect("a walk");
            release(Value::Array(w.array));
        }
        let v = if f.kind == FrameKind::Function {
            for (_, held) in f.scope.vars.borrow_mut().drain() {
                release(held);
            }
            self.live_calls -= 1;
            let block = self.blocks.pop().expect("the call's block");
            let v = self.evacuate(v, &block);
            self.block_bytes_max = self.block_bytes_max.max(block.allocated_bytes());
            self.drop_block(block);
            v
        } else {
            v
        };
        if f.kind == FrameKind::Main || self.frames.is_empty() {
            release(v);
            return Ok(Flow::Done);
        }
        self.stack.push(v);
        Ok(Flow::Go)
    }
}
