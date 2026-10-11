// Copyright (C) 2026 Kenneth Allen Flegal and DappleAI
// SPDX-License-Identifier: GPL-3.0-or-later

//! The compiler: a page's statements into whp's own instructions, once per page. A compiled page
//! is kept by the store and only read by the requests that run it.

use std::collections::HashMap;

use crate::Refusal;
use crate::builtins;
use crate::lex::lex;
use crate::parse::{BinOp, Expr, IncludeKind, Part, Stmt, StmtKind, UnOp, parse_page, significant};

/// The variables PHP itself defines, which whp does not have yet: a page that names one is
/// refused, since it would read them as unset.
const PREDEFINED: &[&[u8]] = &[
    b"GLOBALS",
    b"_SERVER",
    b"_GET",
    b"_POST",
    b"_FILES",
    b"_COOKIE",
    b"_SESSION",
    b"_REQUEST",
    b"_ENV",
    b"argv",
    b"argc",
    b"http_response_header",
];

/// A compiled page: what [`compile_page`] makes of a page's source, kept by a
/// [`Store`](crate::Store) and only read by the requests that run it.
#[derive(Debug)]
pub struct Page {
    /// The page's path, resolved, as `__FILE__` gives it and `include_once` knows it.
    pub(crate) path: Box<[u8]>,
    pub(crate) dir: Box<[u8]>,
    pub(crate) main: Code,
    pub(crate) functions: Vec<Function>,
    pub(crate) names: Vec<Box<[u8]>>,
    pub(crate) lits: Vec<Lit>,
}

#[derive(Debug)]
pub struct Function {
    pub name: Box<[u8]>,
    /// The name, lowercased: functions are found without regard to case.
    pub key: Box<[u8]>,
    pub params: Vec<u32>,
    pub required: usize,
    pub code: Code,
}

#[derive(Debug, Default)]
pub struct Code {
    pub ops: Vec<Op>,
    pub lines: Vec<u32>,
}

/// A literal a page holds.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Lit {
    Null,
    Bool(bool),
    Int(i64),
    Str(Box<[u8]>),
}

/// whp's instructions. Values move through an operand stack; names and literals are the page's,
/// by index.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Op {
    Lit(u32),
    Load(u32),
    /// A variable that may be unset, for `??`, `isset` and `empty`: null when it is.
    LoadQuiet(u32),
    /// Pops a value into a variable and pushes it back: an assignment is an expression.
    Store(u32),
    /// Pops a key and a container, pushes the element.
    Dim,
    DimQuiet,
    /// Pops `depth` keys (one fewer when appending) and a value; writes the value into the
    /// variable's element at that path, and pushes it back.
    StoreDim {
        var: u32,
        depth: u32,
        append: bool,
    },
    IncDec {
        var: u32,
        pre: bool,
        inc: bool,
    },
    /// Pops a value and pushes whether it is not null: `isset`.
    NotNull,
    Unset {
        var: u32,
        depth: u32,
    },
    Echo,
    Print,
    Pop,
    Dup,
    Bin(BinOp),
    /// `$var op= value`, the value on the stack: the variable is read after the value, as PHP
    /// reads it, and the result stored, and pushed when `keep`.
    Compound {
        var: u32,
        op: BinOp,
        keep: bool,
    },
    Unary(UnOp),
    ToBool,
    Concat(u32),
    Jump(u32),
    JumpIfFalse(u32),
    JumpIfTrue(u32),
    /// Jumps keeping the value when it is truthy; else pops it.
    JumpIfTruthyKeep(u32),
    /// Jumps keeping the value when it is not null; else pops it.
    JumpIfNotNullKeep(u32),
    /// Jumps when the parameter was given; its default is computed otherwise.
    JumpIfBound(u32, u32),
    Call {
        name: u32,
        argc: u32,
    },
    Return,
    NewArray,
    ArrayPush,
    ArraySet,
    IterInit,
    IterNext {
        key: Option<u32>,
        value: u32,
        end: u32,
    },
    PopIter,
    Include(IncludeKind),
    Unmatched,
}

/// Compiles one page from its source. `path` is the page's resolved path.
pub fn compile_page(path: &[u8], source: &[u8]) -> Result<Page, Refusal> {
    let refuse = |what: String, line: u32| Refusal {
        what,
        file: String::from_utf8_lossy(path).into_owned(),
        line,
    };
    let tokens = lex(source).map_err(|e| refuse(e.what.to_string(), e.line))?;
    let sig = significant(source, &tokens);
    let stmts = parse_page(&sig).map_err(|e| refuse(e.what, e.line))?;
    let dir = match path.iter().rposition(|&b| b == b'/') {
        Some(0) => b"/".to_vec(),
        Some(p) => path[..p].to_vec(),
        None => b".".to_vec(),
    };
    let mut pc = PageCompiler {
        path: path.to_vec(),
        dir: dir.clone(),
        names: Vec::new(),
        name_index: HashMap::new(),
        lits: Vec::new(),
        lit_index: HashMap::new(),
        functions: Vec::new(),
    };
    let result = (|| {
        let mut main = Emitter::default();
        for s in &stmts {
            if let StmtKind::Function(f) = &s.kind {
                pc.function(f)?;
            }
        }
        for s in &stmts {
            if !matches!(s.kind, StmtKind::Function(_)) {
                pc.stmt(&mut main, s)?;
            }
        }
        let one = pc.lit(Lit::Int(1));
        main.emit(Op::Lit(one));
        main.emit(Op::Return);
        Ok(main.code)
    })();
    let main = result.map_err(|r: Refusal| Refusal {
        file: String::from_utf8_lossy(path).into_owned(),
        ..r
    })?;
    Ok(Page {
        path: path.into(),
        dir: dir.into(),
        main,
        functions: pc.functions,
        names: pc.names,
        lits: pc.lits,
    })
}

struct PageCompiler {
    path: Vec<u8>,
    dir: Vec<u8>,
    names: Vec<Box<[u8]>>,
    name_index: HashMap<Vec<u8>, u32>,
    lits: Vec<Lit>,
    lit_index: HashMap<Lit, u32>,
    functions: Vec<Function>,
}

/// One body's instructions as they are written, and the loops open around the next one.
#[derive(Default)]
struct Emitter {
    code: Code,
    line: u32,
    loops: Vec<Loop>,
}

struct Loop {
    foreach: bool,
    breaks: Vec<usize>,
    continues: Vec<usize>,
}

impl Emitter {
    fn emit(&mut self, op: Op) -> usize {
        self.code.ops.push(op);
        self.code.lines.push(self.line);
        self.code.ops.len() - 1
    }

    fn here(&self) -> u32 {
        self.code.ops.len() as u32
    }

    /// Points the jump at `at` to `target`.
    fn patch(&mut self, at: usize, target: u32) {
        self.code.ops[at] = match self.code.ops[at] {
            Op::Jump(_) => Op::Jump(target),
            Op::JumpIfFalse(_) => Op::JumpIfFalse(target),
            Op::JumpIfTrue(_) => Op::JumpIfTrue(target),
            Op::JumpIfTruthyKeep(_) => Op::JumpIfTruthyKeep(target),
            Op::JumpIfNotNullKeep(_) => Op::JumpIfNotNullKeep(target),
            Op::JumpIfBound(p, _) => Op::JumpIfBound(p, target),
            Op::IterNext { key, value, .. } => Op::IterNext {
                key,
                value,
                end: target,
            },
            op => unreachable!("patching {op:?}"),
        };
    }

    fn at_line(&mut self, line: u32) {
        if line > 0 {
            self.line = line;
        }
    }
}

impl PageCompiler {
    fn name(&mut self, n: &[u8]) -> u32 {
        if let Some(&i) = self.name_index.get(n) {
            return i;
        }
        let i = self.names.len() as u32;
        self.names.push(n.into());
        self.name_index.insert(n.to_vec(), i);
        i
    }

    fn lit(&mut self, l: Lit) -> u32 {
        if let Some(&i) = self.lit_index.get(&l) {
            return i;
        }
        let i = self.lits.len() as u32;
        self.lits.push(l.clone());
        self.lit_index.insert(l, i);
        i
    }

    fn refuse<T>(&self, e: &Emitter, what: impl Into<String>) -> Result<T, Refusal> {
        Err(Refusal::at(what, e.line))
    }

    /// A variable a page names, refused where it is `$this` or one PHP itself defines.
    fn var(&mut self, e: &Emitter, name: &[u8]) -> Result<u32, Refusal> {
        if name == b"this" {
            return self.refuse(e, "$this");
        }
        if PREDEFINED.contains(&name) {
            return self.refuse(
                e,
                format!(
                    "${}, which it does not have yet",
                    String::from_utf8_lossy(name)
                ),
            );
        }
        Ok(self.name(name))
    }

    fn function(&mut self, f: &crate::parse::Function<'_>) -> Result<(), Refusal> {
        let mut e = Emitter {
            line: f.line,
            ..Default::default()
        };
        if let Some((what, line)) = f.refused {
            e.at_line(line);
            return self.refuse(&e, what);
        }
        let key: Box<[u8]> = f.name.to_ascii_lowercase().into();
        if builtins::php_has(&key) {
            return self.refuse(
                &e,
                format!(
                    "declaring {}(), which PHP already has",
                    String::from_utf8_lossy(f.name)
                ),
            );
        }
        let mut params = Vec::new();
        let mut required = 0;
        let mut seen_default = false;
        for (n, p) in f.params.iter().enumerate() {
            let idx = self.var(&e, p.name)?;
            if params.contains(&idx) {
                return self.refuse(&e, "a parameter named twice");
            }
            params.push(idx);
            match &p.default {
                Some(d) if !constant(d) => {
                    return self
                        .refuse(&e, "a parameter default that is not a constant expression");
                }
                Some(d) => {
                    seen_default = true;
                    let jump = e.emit(Op::JumpIfBound(idx, 0));
                    self.expr(&mut e, d)?;
                    e.emit(Op::Store(idx));
                    e.emit(Op::Pop);
                    let here = e.here();
                    e.patch(jump, here);
                }
                None if seen_default => {
                    return self.refuse(&e, "a required parameter after an optional one");
                }
                None => required = n + 1,
            }
        }
        for s in &f.body {
            self.stmt(&mut e, s)?;
        }
        let null = self.lit(Lit::Null);
        e.emit(Op::Lit(null));
        e.emit(Op::Return);
        if self.functions.iter().any(|g| g.key == key) {
            return self.refuse(&e, "a function declared twice");
        }
        self.functions.push(Function {
            name: f.name.into(),
            key,
            params,
            required,
            code: e.code,
        });
        Ok(())
    }

    fn body(&mut self, e: &mut Emitter, stmts: &[Stmt<'_>]) -> Result<(), Refusal> {
        for s in stmts {
            self.stmt(e, s)?;
        }
        Ok(())
    }

    fn stmt(&mut self, e: &mut Emitter, s: &Stmt<'_>) -> Result<(), Refusal> {
        e.at_line(s.line);
        match &s.kind {
            StmtKind::Html(text) => {
                let l = self.lit(Lit::Str((*text).into()));
                e.emit(Op::Lit(l));
                e.emit(Op::Echo);
            }
            StmtKind::Echo(args) => {
                for a in args {
                    self.expr(e, a)?;
                    e.emit(Op::Echo);
                }
            }
            // A compound assignment whose value is not used keeps nothing but its variable.
            StmtKind::Expr(Expr::AssignOp(op, place, value, line))
                if matches!(**place, Expr::Var(..)) =>
            {
                let Expr::Var(name, _) = &**place else {
                    unreachable!("a variable")
                };
                e.at_line(*line);
                let var = self.var(e, name)?;
                self.expr(e, value)?;
                e.emit(Op::Compound {
                    var,
                    op: *op,
                    keep: false,
                });
            }
            StmtKind::Expr(x) => {
                self.expr(e, x)?;
                e.emit(Op::Pop);
            }
            StmtKind::If { arms, otherwise } => {
                let mut ends = Vec::new();
                for (cond, body) in arms {
                    self.expr(e, cond)?;
                    let skip = e.emit(Op::JumpIfFalse(0));
                    self.body(e, body)?;
                    ends.push(e.emit(Op::Jump(0)));
                    let here = e.here();
                    e.patch(skip, here);
                }
                if let Some(body) = otherwise {
                    self.body(e, body)?;
                }
                let here = e.here();
                for j in ends {
                    e.patch(j, here);
                }
            }
            StmtKind::While { cond, body } => {
                let top = e.here();
                self.expr(e, cond)?;
                let exit = e.emit(Op::JumpIfFalse(0));
                e.loops.push(Loop {
                    foreach: false,
                    breaks: vec![exit],
                    continues: Vec::new(),
                });
                self.body(e, body)?;
                e.emit(Op::Jump(top));
                self.close_loop(e, top);
            }
            StmtKind::DoWhile { body, cond } => {
                let top = e.here();
                e.loops.push(Loop {
                    foreach: false,
                    breaks: Vec::new(),
                    continues: Vec::new(),
                });
                self.body(e, body)?;
                let cont = e.here();
                self.expr(e, cond)?;
                e.emit(Op::JumpIfTrue(top));
                self.close_loop(e, cont);
            }
            StmtKind::For {
                init,
                cond,
                step,
                body,
            } => {
                for x in init {
                    self.expr(e, x)?;
                    e.emit(Op::Pop);
                }
                let top = e.here();
                let mut exit = None;
                for (n, x) in cond.iter().enumerate() {
                    self.expr(e, x)?;
                    if n + 1 < cond.len() {
                        e.emit(Op::Pop);
                    } else {
                        exit = Some(e.emit(Op::JumpIfFalse(0)));
                    }
                }
                e.loops.push(Loop {
                    foreach: false,
                    breaks: exit.into_iter().collect(),
                    continues: Vec::new(),
                });
                self.body(e, body)?;
                let cont = e.here();
                for x in step {
                    self.expr(e, x)?;
                    e.emit(Op::Pop);
                }
                e.emit(Op::Jump(top));
                self.close_loop(e, cont);
            }
            StmtKind::Foreach {
                subject,
                key,
                value,
                body,
            } => {
                self.expr(e, subject)?;
                e.emit(Op::IterInit);
                let key = key.map(|k| self.var(e, k)).transpose()?;
                let value = self.var(e, value)?;
                let next = e.here();
                let iter = e.emit(Op::IterNext { key, value, end: 0 });
                e.loops.push(Loop {
                    foreach: true,
                    breaks: vec![iter],
                    continues: Vec::new(),
                });
                self.body(e, body)?;
                e.emit(Op::Jump(next));
                // The loop's end pops its walk; a break lands there too.
                let lp = e.loops.pop().expect("the loop");
                let end = e.here();
                e.emit(Op::PopIter);
                for b in lp.breaks {
                    e.patch(b, end);
                }
                for c in lp.continues {
                    e.patch(c, next);
                }
            }
            StmtKind::Break(levels) => self.jump_out(e, *levels, true)?,
            StmtKind::Continue(levels) => self.jump_out(e, *levels, false)?,
            StmtKind::Return(x) => {
                match x {
                    Some(x) => self.expr(e, x)?,
                    None => {
                        let null = self.lit(Lit::Null);
                        e.emit(Op::Lit(null));
                    }
                }
                e.emit(Op::Return);
            }
            StmtKind::Function(_) => return self.refuse(e, "a function declared inside a block"),
            StmtKind::Unset(places) => {
                for p in places {
                    let (var, depth) = self.path(e, p, false)?;
                    e.emit(Op::Unset { var, depth });
                }
            }
            StmtKind::Block(b) => self.body(e, b)?,
        }
        Ok(())
    }

    /// Closes the innermost loop: breaks land after it, continues at `cont`.
    fn close_loop(&mut self, e: &mut Emitter, cont: u32) {
        let lp = e.loops.pop().expect("the loop");
        let end = e.here();
        for b in lp.breaks {
            e.patch(b, end);
        }
        for c in lp.continues {
            e.patch(c, cont);
        }
    }

    /// `break n` or `continue n`: the walks of the loops left are popped, then the jump.
    fn jump_out(&mut self, e: &mut Emitter, levels: u32, is_break: bool) -> Result<(), Refusal> {
        let n = levels as usize;
        if n > e.loops.len() {
            return self.refuse(e, "break or continue outside as many loops");
        }
        let target = e.loops.len() - n;
        let inner_walks = e.loops[target + 1..].iter().filter(|l| l.foreach).count();
        for _ in 0..inner_walks {
            e.emit(Op::PopIter);
        }
        let j = e.emit(Op::Jump(0));
        let lp = &mut e.loops[target];
        if is_break {
            lp.breaks.push(j)
        } else {
            lp.continues.push(j)
        };
        Ok(())
    }

    /// The keys of a place, pushed in order; gives back its variable and depth. A place ending in
    /// `[]` appends, which only an assignment may do.
    fn path(
        &mut self,
        e: &mut Emitter,
        place: &Expr<'_>,
        append_ok: bool,
    ) -> Result<(u32, u32), Refusal> {
        let mut keys: Vec<Option<&Expr<'_>>> = Vec::new();
        let mut at = place;
        loop {
            match at {
                Expr::Var(name, line) => {
                    e.at_line(*line);
                    let var = self.var(e, name)?;
                    let depth = keys.len() as u32;
                    for (n, k) in keys.iter().rev().enumerate() {
                        match k {
                            Some(k) => self.expr(e, k)?,
                            None if n + 1 == keys.len() && append_ok => {}
                            None => return self.refuse(e, "[] outside an assignment"),
                        }
                    }
                    return Ok((var, depth));
                }
                Expr::Dim(base, key, line) => {
                    e.at_line(*line);
                    keys.push(key.as_deref());
                    at = base;
                }
                _ => return self.refuse(e, "writing to this expression"),
            }
        }
    }

    /// An expression read where an unset variable or a missing key is no error: it reads null.
    fn quiet(&mut self, e: &mut Emitter, x: &Expr<'_>) -> Result<(), Refusal> {
        match x {
            Expr::Var(name, line) => {
                e.at_line(*line);
                let n = self.var(e, name)?;
                e.emit(Op::LoadQuiet(n));
            }
            Expr::Dim(base, Some(key), line) => {
                self.quiet(e, base)?;
                self.expr(e, key)?;
                e.at_line(*line);
                e.emit(Op::DimQuiet);
            }
            _ => self.expr(e, x)?,
        }
        Ok(())
    }

    fn expr(&mut self, e: &mut Emitter, x: &Expr<'_>) -> Result<(), Refusal> {
        match x {
            Expr::Null => {
                let l = self.lit(Lit::Null);
                e.emit(Op::Lit(l));
            }
            Expr::Bool(b) => {
                let l = self.lit(Lit::Bool(*b));
                e.emit(Op::Lit(l));
            }
            Expr::Int(i) => {
                let l = self.lit(Lit::Int(*i));
                e.emit(Op::Lit(l));
            }
            Expr::Str(s) => {
                let l = self.lit(Lit::Str(s.as_slice().into()));
                e.emit(Op::Lit(l));
            }
            Expr::Interp(parts, line) => {
                e.at_line(*line);
                for p in parts {
                    match p {
                        Part::Text(t) => {
                            let l = self.lit(Lit::Str(t.as_slice().into()));
                            e.emit(Op::Lit(l));
                        }
                        Part::Expr(x) => self.expr(e, x)?,
                    }
                }
                e.emit(Op::Concat(parts.len() as u32));
            }
            Expr::Var(name, line) => {
                e.at_line(*line);
                let n = self.var(e, name)?;
                e.emit(Op::Load(n));
            }
            Expr::Dim(base, key, line) => {
                let Some(key) = key else {
                    e.at_line(*line);
                    return self.refuse(e, "[] outside an assignment");
                };
                self.expr(e, base)?;
                self.expr(e, key)?;
                e.at_line(*line);
                e.emit(Op::Dim);
            }
            Expr::Call { name, args, line } => {
                for a in args {
                    self.expr(e, a)?;
                }
                e.at_line(*line);
                let n = self.name(&name.to_ascii_lowercase());
                e.emit(Op::Call {
                    name: n,
                    argc: args.len() as u32,
                });
            }
            Expr::Unary(op, a, line) => {
                self.expr(e, a)?;
                e.at_line(*line);
                e.emit(Op::Unary(*op));
            }
            Expr::Bin(BinOp::And, a, b, _) => {
                self.expr(e, a)?;
                let short = e.emit(Op::JumpIfFalse(0));
                self.expr(e, b)?;
                e.emit(Op::ToBool);
                let done = e.emit(Op::Jump(0));
                let here = e.here();
                e.patch(short, here);
                let f = self.lit(Lit::Bool(false));
                e.emit(Op::Lit(f));
                let here = e.here();
                e.patch(done, here);
            }
            Expr::Bin(BinOp::Or, a, b, _) => {
                self.expr(e, a)?;
                let short = e.emit(Op::JumpIfTrue(0));
                self.expr(e, b)?;
                e.emit(Op::ToBool);
                let done = e.emit(Op::Jump(0));
                let here = e.here();
                e.patch(short, here);
                let t = self.lit(Lit::Bool(true));
                e.emit(Op::Lit(t));
                let here = e.here();
                e.patch(done, here);
            }
            Expr::Bin(op, a, b, line) => {
                self.expr(e, a)?;
                self.expr(e, b)?;
                e.at_line(*line);
                e.emit(Op::Bin(*op));
            }
            Expr::Coalesce(a, b) => {
                self.quiet(e, a)?;
                let done = e.emit(Op::JumpIfNotNullKeep(0));
                self.expr(e, b)?;
                let here = e.here();
                e.patch(done, here);
            }
            Expr::Ternary(c, then, otherwise) => match then {
                Some(then) => {
                    self.expr(e, c)?;
                    let skip = e.emit(Op::JumpIfFalse(0));
                    self.expr(e, then)?;
                    let done = e.emit(Op::Jump(0));
                    let here = e.here();
                    e.patch(skip, here);
                    self.expr(e, otherwise)?;
                    let here = e.here();
                    e.patch(done, here);
                }
                None => {
                    self.expr(e, c)?;
                    let done = e.emit(Op::JumpIfTruthyKeep(0));
                    self.expr(e, otherwise)?;
                    let here = e.here();
                    e.patch(done, here);
                }
            },
            Expr::Assign(place, value, line) => {
                e.at_line(*line);
                match &**place {
                    Expr::Var(name, l) => {
                        e.at_line(*l);
                        let n = self.var(e, name)?;
                        self.expr(e, value)?;
                        e.emit(Op::Store(n));
                    }
                    _ => {
                        let append = matches!(&**place, Expr::Dim(_, None, _));
                        let (var, depth) = self.path(e, place, true)?;
                        self.expr(e, value)?;
                        e.emit(Op::StoreDim { var, depth, append });
                    }
                }
            }
            Expr::AssignOp(op, place, value, line) => {
                e.at_line(*line);
                let Expr::Var(name, _) = &**place else {
                    return self.refuse(e, "a compound assignment to an element");
                };
                let var = self.var(e, name)?;
                self.expr(e, value)?;
                e.emit(Op::Compound {
                    var,
                    op: *op,
                    keep: true,
                });
            }
            Expr::IncDec {
                pre,
                inc,
                place,
                line,
            } => {
                e.at_line(*line);
                let Expr::Var(name, _) = &**place else {
                    return self.refuse(e, "++ or -- on an element");
                };
                let var = self.var(e, name)?;
                e.emit(Op::IncDec {
                    var,
                    pre: *pre,
                    inc: *inc,
                });
            }
            Expr::Array(items, line) => {
                e.at_line(*line);
                e.emit(Op::NewArray);
                for (k, v) in items {
                    match k {
                        Some(k) => {
                            self.expr(e, k)?;
                            self.expr(e, v)?;
                            e.emit(Op::ArraySet);
                        }
                        None => {
                            self.expr(e, v)?;
                            e.emit(Op::ArrayPush);
                        }
                    }
                }
            }
            Expr::Match {
                subject,
                arms,
                line,
            } => {
                e.at_line(*line);
                self.expr(e, subject)?;
                let mut to_arm = Vec::new();
                let mut default = None;
                for (n, (conds, _)) in arms.iter().enumerate() {
                    match conds {
                        Some(conds) => {
                            for c in conds {
                                e.emit(Op::Dup);
                                self.expr(e, c)?;
                                e.emit(Op::Bin(BinOp::Identical));
                                to_arm.push((n, e.emit(Op::JumpIfTrue(0))));
                            }
                        }
                        None => default = Some(n),
                    }
                }
                let to_default = match default {
                    Some(_) => Some(e.emit(Op::Jump(0))),
                    None => {
                        e.emit(Op::Unmatched);
                        None
                    }
                };
                let mut ends = Vec::new();
                for (n, (_, value)) in arms.iter().enumerate() {
                    let start = e.here();
                    for (arm, j) in &to_arm {
                        if *arm == n {
                            e.patch(*j, start);
                        }
                    }
                    if default == Some(n)
                        && let Some(j) = to_default
                    {
                        e.patch(j, start);
                    }
                    e.emit(Op::Pop);
                    self.expr(e, value)?;
                    ends.push(e.emit(Op::Jump(0)));
                }
                let here = e.here();
                for j in ends {
                    e.patch(j, here);
                }
            }
            Expr::Isset(places) => {
                let mut shorts = Vec::new();
                for (n, p) in places.iter().enumerate() {
                    if !matches!(p, Expr::Var(..) | Expr::Dim(_, Some(_), _)) {
                        return self.refuse(
                            e,
                            "isset() on something other than a variable or an element",
                        );
                    }
                    self.quiet(e, p)?;
                    e.emit(Op::NotNull);
                    if n + 1 < places.len() {
                        shorts.push(e.emit(Op::JumpIfFalse(0)));
                    }
                }
                if !shorts.is_empty() {
                    let done = e.emit(Op::Jump(0));
                    let here = e.here();
                    for s in shorts {
                        e.patch(s, here);
                    }
                    let f = self.lit(Lit::Bool(false));
                    e.emit(Op::Lit(f));
                    let here = e.here();
                    e.patch(done, here);
                }
            }
            Expr::Empty(x) => {
                self.quiet(e, x)?;
                e.emit(Op::Unary(UnOp::Not));
            }
            Expr::Include(kind, path, line) => {
                self.expr(e, path)?;
                e.at_line(*line);
                e.emit(Op::Include(*kind));
            }
            Expr::Print(x, line) => {
                self.expr(e, x)?;
                e.at_line(*line);
                e.emit(Op::Print);
            }
            Expr::Dir => {
                let l = self.lit(Lit::Str(self.dir.as_slice().into()));
                e.emit(Op::Lit(l));
            }
            Expr::File => {
                let l = self.lit(Lit::Str(self.path.as_slice().into()));
                e.emit(Op::Lit(l));
            }
            Expr::Line(n) => {
                let l = self.lit(Lit::Int(*n as i64));
                e.emit(Op::Lit(l));
            }
            Expr::Refused { what, line } => {
                e.at_line(*line);
                return self.refuse(e, *what);
            }
        }
        Ok(())
    }
}

/// Whether an expression is one PHP takes as a constant expression, as a parameter's default
/// must be: literals, and operators, arrays and their elements made of them.
fn constant(x: &Expr<'_>) -> bool {
    match x {
        Expr::Null
        | Expr::Bool(_)
        | Expr::Int(_)
        | Expr::Str(_)
        | Expr::Dir
        | Expr::File
        | Expr::Line(_)
        | Expr::Refused { .. } => true,
        Expr::Interp(parts, _) => parts.iter().all(|p| matches!(p, Part::Text(_))),
        Expr::Array(items, _) => items
            .iter()
            .all(|(k, v)| k.as_ref().is_none_or(constant) && constant(v)),
        Expr::Unary(_, a, _) => constant(a),
        Expr::Bin(_, a, b, _) | Expr::Coalesce(a, b) => constant(a) && constant(b),
        Expr::Dim(a, b, _) => constant(a) && b.as_deref().is_some_and(constant),
        Expr::Ternary(c, t, o) => constant(c) && t.as_deref().is_none_or(constant) && constant(o),
        _ => false,
    }
}
