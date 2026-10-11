// Copyright (C) 2026 Kenneth Allen Flegal and DappleAI
// SPDX-License-Identifier: GPL-3.0-or-later

//! The parser: a page's tokens into statements, one function per production of the grammar the
//! PHP manual describes. A construct whp does not run yet parses to a `Refused` node, or fails
//! with the construct named, so a page is refused whole and never run wrongly.

use winnow::combinator::{alt, delimited, fail, opt, preceded, repeat, separated, terminated};
use winnow::error::ContextError;
use winnow::prelude::*;
use winnow::token::any;

use crate::lex::{Kind, Kw, Magic, Op, Token};

/// A token as the parser sees it: whitespace, comments and open tags are gone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tok<'s> {
    pub kind: Kind,
    pub text: &'s [u8],
    pub line: u32,
}

type In<'a, 's> = &'a [Tok<'s>];
type PResult<T> = winnow::Result<T, ContextError>;

/// A statement, and the line it begins on.
#[derive(Debug, Clone, PartialEq)]
pub struct Stmt<'s> {
    pub line: u32,
    pub kind: StmtKind<'s>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind<'s> {
    Html(&'s [u8]),
    Echo(Vec<Expr<'s>>),
    Expr(Expr<'s>),
    If {
        arms: Vec<(Expr<'s>, Vec<Stmt<'s>>)>,
        otherwise: Option<Vec<Stmt<'s>>>,
    },
    While {
        cond: Expr<'s>,
        body: Vec<Stmt<'s>>,
    },
    DoWhile {
        body: Vec<Stmt<'s>>,
        cond: Expr<'s>,
    },
    For {
        init: Vec<Expr<'s>>,
        cond: Vec<Expr<'s>>,
        step: Vec<Expr<'s>>,
        body: Vec<Stmt<'s>>,
    },
    Foreach {
        subject: Expr<'s>,
        key: Option<&'s [u8]>,
        value: &'s [u8],
        body: Vec<Stmt<'s>>,
    },
    /// `break`, with how many loops it leaves.
    Break(u32),
    /// `continue`, with how many loops it leaves.
    Continue(u32),
    Return(Option<Expr<'s>>),
    Function(Function<'s>),
    Unset(Vec<Expr<'s>>),
    Block(Vec<Stmt<'s>>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function<'s> {
    pub name: &'s [u8],
    pub params: Vec<Param<'s>>,
    pub body: Vec<Stmt<'s>>,
    pub line: u32,
    /// A part of the declaration whp does not run yet, and its line.
    pub refused: Option<(&'static str, u32)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param<'s> {
    pub name: &'s [u8],
    pub default: Option<Expr<'s>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Concat,
    Equal,
    NotEqual,
    Identical,
    NotIdentical,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Spaceship,
    And,
    Or,
    Xor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
    Plus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncludeKind {
    Include,
    IncludeOnce,
    Require,
    RequireOnce,
}

/// A piece of an interpolated string.
#[derive(Debug, Clone, PartialEq)]
pub enum Part<'s> {
    Text(Vec<u8>),
    Expr(Expr<'s>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr<'s> {
    Null,
    Bool(bool),
    Int(i64),
    Str(Vec<u8>),
    Interp(Vec<Part<'s>>, u32),
    Var(&'s [u8], u32),
    /// `base[key]`; a missing key is `base[]`, which only an assignment may name.
    Dim(Box<Expr<'s>>, Option<Box<Expr<'s>>>, u32),
    Call {
        name: &'s [u8],
        args: Vec<Expr<'s>>,
        line: u32,
    },
    Unary(UnOp, Box<Expr<'s>>, u32),
    Bin(BinOp, Box<Expr<'s>>, Box<Expr<'s>>, u32),
    Coalesce(Box<Expr<'s>>, Box<Expr<'s>>),
    Ternary(Box<Expr<'s>>, Option<Box<Expr<'s>>>, Box<Expr<'s>>),
    Assign(Box<Expr<'s>>, Box<Expr<'s>>, u32),
    AssignOp(BinOp, Box<Expr<'s>>, Box<Expr<'s>>, u32),
    IncDec {
        pre: bool,
        inc: bool,
        place: Box<Expr<'s>>,
        line: u32,
    },
    Array(Vec<(Option<Expr<'s>>, Expr<'s>)>, u32),
    Match {
        subject: Box<Expr<'s>>,
        arms: Vec<(Option<Vec<Expr<'s>>>, Expr<'s>)>,
        line: u32,
    },
    Isset(Vec<Expr<'s>>),
    Empty(Box<Expr<'s>>),
    Include(IncludeKind, Box<Expr<'s>>, u32),
    Print(Box<Expr<'s>>, u32),
    Dir,
    File,
    Line(u32),
    Refused {
        what: &'static str,
        line: u32,
    },
}

impl Expr<'_> {
    /// A place an assignment may write: a variable, or a variable's element at any depth.
    pub fn is_place(&self) -> bool {
        match self {
            Expr::Var(..) => true,
            Expr::Dim(base, _, _) => base.is_place(),
            _ => false,
        }
    }
}

/// Why a page could not be parsed: the line, and what was found there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: u32,
    pub what: String,
}

/// The parser's view of a page's tokens.
pub fn significant<'s>(src: &'s [u8], tokens: &[Token]) -> Vec<Tok<'s>> {
    tokens
        .iter()
        .filter(|t| {
            !matches!(
                t.kind,
                Kind::Whitespace | Kind::Comment | Kind::DocComment | Kind::OpenTag
            )
        })
        .map(|t| Tok {
            kind: t.kind,
            text: t.text(src),
            line: t.line,
        })
        .collect()
}

/// Parses a page's significant tokens into its statements.
pub fn parse_page<'s>(toks: &[Tok<'s>]) -> Result<Vec<Stmt<'s>>, ParseError> {
    let mut input: In<'_, 's> = toks;
    let stmts: Vec<Stmt<'s>> = repeat(0.., statement)
        .parse_next(&mut input)
        .unwrap_or_default();
    if input.is_empty() {
        return Ok(stmts);
    }
    let at = toks.len() - input.len();
    Err(explain(toks, at))
}

/// Names what stopped the parser: a construct whp does not run yet, or the token itself.
fn explain(toks: &[Tok<'_>], at: usize) -> ParseError {
    let line = toks[at].line;
    let end = toks[at..]
        .iter()
        .position(|t| matches!(t.kind, Kind::Char(b';') | Kind::CloseTag))
        .map_or(toks.len(), |p| at + p + 1);
    for t in &toks[at..end] {
        if let Some(what) = construct(t) {
            return ParseError {
                line: t.line,
                what: format!("whp does not run {what} yet"),
            };
        }
    }
    ParseError {
        line,
        what: format!(
            "cannot parse at `{}`",
            String::from_utf8_lossy(toks[at].text)
        ),
    }
}

/// The constructs outside whp's subset, by the token that starts them.
fn construct(t: &Tok<'_>) -> Option<&'static str> {
    Some(match t.kind {
        Kind::Keyword(k) => match k {
            Kw::Class | Kw::Interface | Kw::Trait | Kw::Abstract | Kw::Final | Kw::Readonly => {
                "classes"
            }
            Kw::New | Kw::Clone | Kw::InstanceOf => "objects",
            Kw::Fn => "arrow functions",
            Kw::Static => "static variables",
            Kw::Global => "global",
            Kw::Switch | Kw::Case => "switch",
            Kw::Try | Kw::Catch | Kw::Finally | Kw::Throw => "exceptions",
            Kw::Yield => "generators",
            Kw::Namespace | Kw::Use => "namespaces",
            Kw::Goto => "goto",
            Kw::List => "list()",
            Kw::Declare | Kw::EndDeclare => "declare",
            Kw::Eval => "eval",
            Kw::Exit | Kw::Die => "exit",
            Kw::Const => "constants",
            _ => return None,
        },
        Kind::DNumber => "floats",
        Kind::Op(Op::Arrow | Op::NullsafeArrow) => "objects",
        Kind::Op(Op::DoubleColon) => "classes",
        Kind::Op(Op::Pipe) => "the pipe operator",
        Kind::Op(Op::Ellipsis) => "variadics and spreads",
        Kind::AmpVar | Kind::AmpNotVar => "references",
        Kind::Attribute => "attributes",
        Kind::Cast(_) => "casts",
        Kind::NameQualified | Kind::NameRelative => "namespaces",
        Kind::DollarOpenCurlyBraces => "${…} in strings",
        Kind::Char(b'@') => "@",
        Kind::Char(b'`') => "backticks",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------------------------
// Tokens

fn kind<'a, 's: 'a>(k: Kind) -> impl Parser<In<'a, 's>, Tok<'s>, ContextError> {
    any.verify(move |t: &Tok<'s>| t.kind == k)
}

fn ch<'a, 's: 'a>(c: u8) -> impl Parser<In<'a, 's>, Tok<'s>, ContextError> {
    kind(Kind::Char(c))
}

fn kw<'a, 's: 'a>(k: Kw) -> impl Parser<In<'a, 's>, Tok<'s>, ContextError> {
    kind(Kind::Keyword(k))
}

/// The end of a statement: `;`, or `?>`, which implies one.
fn end(i: &mut In<'_, '_>) -> PResult<()> {
    alt((ch(b';'), kind(Kind::CloseTag))).void().parse_next(i)
}

fn variable_name<'s>(i: &mut In<'_, 's>) -> PResult<&'s [u8]> {
    kind(Kind::Variable).map(|t| &t.text[1..]).parse_next(i)
}

// ---------------------------------------------------------------------------------------------
// Statements

/// statement: one statement of a page, a block, or a function's body.
fn statement<'s>(i: &mut In<'_, 's>) -> PResult<Stmt<'s>> {
    let Some(first) = i.first().copied() else {
        return fail.parse_next(i);
    };
    let kind = match first.kind {
        Kind::InlineHtml => any.map(|t: Tok<'s>| StmtKind::Html(t.text)).parse_next(i),
        Kind::OpenTagWithEcho | Kind::Keyword(Kw::Echo) => {
            preceded(any, echo_args).map(StmtKind::Echo).parse_next(i)
        }
        Kind::Keyword(Kw::If) => if_statement(i),
        Kind::Keyword(Kw::While) => while_statement(i),
        Kind::Keyword(Kw::Do) => do_statement(i),
        Kind::Keyword(Kw::For) => for_statement(i),
        Kind::Keyword(Kw::Foreach) => foreach_statement(i),
        Kind::Keyword(Kw::Break) => jump(i).map(StmtKind::Break),
        Kind::Keyword(Kw::Continue) => jump(i).map(StmtKind::Continue),
        Kind::Keyword(Kw::Return) => delimited(any, opt(expr), end)
            .map(StmtKind::Return)
            .parse_next(i),
        Kind::Keyword(Kw::Unset) => delimited(
            (any, ch(b'(')),
            terminated(separated(1.., expr, ch(b',')), opt(ch(b','))),
            (ch(b')'), end),
        )
        .map(StmtKind::Unset)
        .parse_next(i),
        Kind::Keyword(Kw::Function) if i.get(1).is_some_and(|t| t.kind == Kind::String) => {
            function_declaration.map(StmtKind::Function).parse_next(i)
        }
        Kind::Char(b'{') => block.map(StmtKind::Block).parse_next(i),
        Kind::Char(b';') | Kind::CloseTag => end.map(|_| StmtKind::Block(Vec::new())).parse_next(i),
        _ => terminated(expr, end).map(StmtKind::Expr).parse_next(i),
    }?;
    Ok(Stmt {
        line: first.line,
        kind,
    })
}

/// echo_args: `expr, expr, …` and the statement's end.
fn echo_args<'s>(i: &mut In<'_, 's>) -> PResult<Vec<Expr<'s>>> {
    terminated(separated(1.., expr, ch(b',')), end).parse_next(i)
}

fn block<'s>(i: &mut In<'_, 's>) -> PResult<Vec<Stmt<'s>>> {
    delimited(ch(b'{'), repeat(0.., statement), ch(b'}')).parse_next(i)
}

/// The statements of an alternative-syntax body, up to its closing keyword.
fn inner<'s>(i: &mut In<'_, 's>) -> PResult<Vec<Stmt<'s>>> {
    repeat(0.., statement).parse_next(i)
}

fn paren<'s>(i: &mut In<'_, 's>) -> PResult<Expr<'s>> {
    delimited(ch(b'('), expr, ch(b')')).parse_next(i)
}

/// A body: one statement, which a block may be.
fn body<'s>(i: &mut In<'_, 's>) -> PResult<Vec<Stmt<'s>>> {
    statement
        .map(|s| match s.kind {
            StmtKind::Block(b) => b,
            _ => vec![s],
        })
        .parse_next(i)
}

/// if_statement: `if (c) s elseif (c) s else s`, or the same with `:` and `endif;`.
fn if_statement<'s>(i: &mut In<'_, 's>) -> PResult<StmtKind<'s>> {
    let cond = preceded(kw(Kw::If), paren).parse_next(i)?;
    if opt(ch(b':')).parse_next(i)?.is_some() {
        let mut arms = vec![(cond, inner(i)?)];
        while opt(kw(Kw::ElseIf)).parse_next(i)?.is_some() {
            let c = terminated(paren, ch(b':')).parse_next(i)?;
            arms.push((c, inner(i)?));
        }
        let otherwise = opt(preceded((kw(Kw::Else), ch(b':')), inner)).parse_next(i)?;
        (kw(Kw::EndIf), end).parse_next(i)?;
        return Ok(StmtKind::If { arms, otherwise });
    }
    let mut arms = vec![(cond, body(i)?)];
    while opt(kw(Kw::ElseIf)).parse_next(i)?.is_some() {
        let c = paren(i)?;
        arms.push((c, body(i)?));
    }
    let otherwise = opt(preceded(kw(Kw::Else), body)).parse_next(i)?;
    Ok(StmtKind::If { arms, otherwise })
}

fn while_statement<'s>(i: &mut In<'_, 's>) -> PResult<StmtKind<'s>> {
    let cond = preceded(kw(Kw::While), paren).parse_next(i)?;
    let body = alt((delimited(ch(b':'), inner, (kw(Kw::EndWhile), end)), body)).parse_next(i)?;
    Ok(StmtKind::While { cond, body })
}

fn do_statement<'s>(i: &mut In<'_, 's>) -> PResult<StmtKind<'s>> {
    let body = preceded(kw(Kw::Do), body).parse_next(i)?;
    let cond = delimited(kw(Kw::While), paren, end).parse_next(i)?;
    Ok(StmtKind::DoWhile { body, cond })
}

fn expr_list<'s>(i: &mut In<'_, 's>) -> PResult<Vec<Expr<'s>>> {
    separated(0.., expr, ch(b',')).parse_next(i)
}

fn for_statement<'s>(i: &mut In<'_, 's>) -> PResult<StmtKind<'s>> {
    let (_, _, init, _, cond, _, step, _) = (
        kw(Kw::For),
        ch(b'('),
        expr_list,
        ch(b';'),
        expr_list,
        ch(b';'),
        expr_list,
        ch(b')'),
    )
        .parse_next(i)?;
    let body = alt((delimited(ch(b':'), inner, (kw(Kw::EndFor), end)), body)).parse_next(i)?;
    Ok(StmtKind::For {
        init,
        cond,
        step,
        body,
    })
}

fn foreach_statement<'s>(i: &mut In<'_, 's>) -> PResult<StmtKind<'s>> {
    let (_, _, subject, _, first) =
        (kw(Kw::Foreach), ch(b'('), expr, kw(Kw::As), variable_name).parse_next(i)?;
    let second = opt(preceded(kind(Kind::Op(Op::DoubleArrow)), variable_name)).parse_next(i)?;
    ch(b')').parse_next(i)?;
    let (key, value) = match second {
        Some(v) => (Some(first), v),
        None => (None, first),
    };
    let body = alt((delimited(ch(b':'), inner, (kw(Kw::EndForeach), end)), body)).parse_next(i)?;
    Ok(StmtKind::Foreach {
        subject,
        key,
        value,
        body,
    })
}

/// `break` or `continue`: how many loops it leaves.
fn jump(i: &mut In<'_, '_>) -> PResult<u32> {
    any.parse_next(i)?;
    let levels = opt(kind(Kind::LNumber)).parse_next(i)?;
    end.parse_next(i)?;
    let levels = match levels {
        Some(n) => std::str::from_utf8(n.text)
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|&n| n > 0),
        None => Some(1),
    };
    match levels {
        Some(n) => Ok(n),
        None => fail.parse_next(i),
    }
}

/// function_declaration: `function name($a, $b = default) { … }`. Types on the parameters or the
/// return are parsed so that the function can be refused by name.
fn function_declaration<'s>(i: &mut In<'_, 's>) -> PResult<Function<'s>> {
    let (f, name) = (kw(Kw::Function), kind(Kind::String)).parse_next(i)?;
    let params: Vec<(Param<'s>, Option<u32>)> = delimited(
        ch(b'('),
        terminated(separated(0.., param, ch(b',')), opt(ch(b','))),
        ch(b')'),
    )
    .parse_next(i)?;
    let returns = opt(preceded(ch(b':'), type_declaration)).parse_next(i)?;
    let body = block(i)?;
    let typed = params.iter().find_map(|(_, t)| *t).or(returns);
    Ok(Function {
        name: name.text,
        params: params.into_iter().map(|(p, _)| p).collect(),
        body,
        line: f.line,
        refused: typed.map(|line| ("type declarations", line)),
    })
}

/// A parameter, and the line of its type if it has one.
fn param<'s>(i: &mut In<'_, 's>) -> PResult<(Param<'s>, Option<u32>)> {
    let typed = opt(type_declaration).parse_next(i)?;
    let name = variable_name(i)?;
    let default = opt(preceded(ch(b'='), expr)).parse_next(i)?;
    Ok((Param { name, default }, typed))
}

/// type_declaration: a type's names with `?`, `|` and `&` among them. Gives back its line.
fn type_declaration(i: &mut In<'_, '_>) -> PResult<u32> {
    let toks: Vec<Tok<'_>> = repeat(
        1..,
        any.verify(|t: &Tok<'_>| {
            matches!(
                t.kind,
                Kind::String
                    | Kind::NameQualified
                    | Kind::NameFullyQualified
                    | Kind::Keyword(Kw::Array | Kw::Callable | Kw::Static)
                    | Kind::Char(b'?' | b'|')
                    | Kind::AmpNotVar
            )
        }),
    )
    .parse_next(i)?;
    Ok(toks[0].line)
}

// ---------------------------------------------------------------------------------------------
// Expressions

/// Binding powers, from the manual's operator precedence table: higher binds tighter.
mod power {
    pub const OR: i64 = 10;
    pub const XOR: i64 = 20;
    pub const AND: i64 = 30;
    pub const PRINT: i64 = 40;
    pub const ASSIGN: i64 = 50;
    pub const TERNARY: i64 = 60;
    pub const COALESCE: i64 = 70;
    pub const BOOLEAN_OR: i64 = 80;
    pub const BOOLEAN_AND: i64 = 90;
    pub const BIT_OR: i64 = 100;
    pub const BIT_XOR: i64 = 110;
    pub const BIT_AND: i64 = 120;
    pub const EQUALITY: i64 = 130;
    pub const COMPARISON: i64 = 140;
    pub const CONCAT: i64 = 150;
    pub const SHIFT: i64 = 160;
    pub const ADD: i64 = 170;
    pub const MUL: i64 = 180;
    pub const NOT: i64 = 190;
    pub const INSTANCEOF: i64 = 200;
    pub const UNARY: i64 = 210;
    pub const POW: i64 = 220;
    pub const POSTFIX: i64 = 230;
}

#[derive(Clone, Copy)]
enum Assoc {
    Left,
    Right,
    /// Two in a row are a parse error: `a == b == c`.
    Neither,
}

#[derive(Clone, Copy)]
enum Infix {
    Bin(BinOp),
    Assign(Option<BinOp>),
    Coalesce,
    Refused(&'static str),
}

#[derive(Clone, Copy)]
enum Prefix {
    Unary(UnOp),
    IncDec(bool),
    Print,
    Include(IncludeKind),
    Refused(&'static str),
}

#[derive(Clone, Copy)]
enum Postfix {
    IncDec(bool),
    Ternary,
}

/// The infix operator a token is: its binding power, how it groups, and what it makes.
fn infix(k: Kind) -> Option<(i64, Assoc, Infix)> {
    use Assoc::{Left, Neither, Right};
    use Infix::{Assign, Bin, Coalesce, Refused};
    Some(match k {
        Kind::Keyword(Kw::Or) => (power::OR, Left, Bin(BinOp::Or)),
        Kind::Keyword(Kw::Xor) => (power::XOR, Left, Bin(BinOp::Xor)),
        Kind::Keyword(Kw::And) => (power::AND, Left, Bin(BinOp::And)),
        Kind::Char(b'=') => (power::ASSIGN, Right, Assign(None)),
        Kind::Op(Op::PlusEqual) => (power::ASSIGN, Right, Assign(Some(BinOp::Add))),
        Kind::Op(Op::MinusEqual) => (power::ASSIGN, Right, Assign(Some(BinOp::Sub))),
        Kind::Op(Op::MulEqual) => (power::ASSIGN, Right, Assign(Some(BinOp::Mul))),
        Kind::Op(Op::DivEqual) => (power::ASSIGN, Right, Assign(Some(BinOp::Div))),
        Kind::Op(Op::ModEqual) => (power::ASSIGN, Right, Assign(Some(BinOp::Mod))),
        Kind::Op(Op::PowEqual) => (power::ASSIGN, Right, Assign(Some(BinOp::Pow))),
        Kind::Op(Op::ConcatEqual) => (power::ASSIGN, Right, Assign(Some(BinOp::Concat))),
        Kind::Op(
            Op::CoalesceEqual
            | Op::AndEqual
            | Op::OrEqual
            | Op::XorEqual
            | Op::ShiftLeftEqual
            | Op::ShiftRightEqual,
        ) => (power::ASSIGN, Right, Refused("this assignment operator")),
        Kind::Op(Op::Coalesce) => (power::COALESCE, Right, Coalesce),
        Kind::Op(Op::BooleanOr) => (power::BOOLEAN_OR, Left, Bin(BinOp::Or)),
        Kind::Op(Op::BooleanAnd) => (power::BOOLEAN_AND, Left, Bin(BinOp::And)),
        Kind::Char(b'|') => (power::BIT_OR, Left, Refused("bitwise operators")),
        Kind::Char(b'^') => (power::BIT_XOR, Left, Refused("bitwise operators")),
        Kind::AmpNotVar | Kind::AmpVar => (power::BIT_AND, Left, Refused("bitwise operators")),
        Kind::Op(Op::Equal) => (power::EQUALITY, Neither, Bin(BinOp::Equal)),
        Kind::Op(Op::NotEqual | Op::NotEqualAngle) => {
            (power::EQUALITY, Neither, Bin(BinOp::NotEqual))
        }
        Kind::Op(Op::Identical) => (power::EQUALITY, Neither, Bin(BinOp::Identical)),
        Kind::Op(Op::NotIdentical) => (power::EQUALITY, Neither, Bin(BinOp::NotIdentical)),
        Kind::Op(Op::Spaceship) => (power::EQUALITY, Neither, Bin(BinOp::Spaceship)),
        Kind::Char(b'<') => (power::COMPARISON, Neither, Bin(BinOp::Less)),
        Kind::Op(Op::SmallerOrEqual) => (power::COMPARISON, Neither, Bin(BinOp::LessOrEqual)),
        Kind::Char(b'>') => (power::COMPARISON, Neither, Bin(BinOp::Greater)),
        Kind::Op(Op::GreaterOrEqual) => (power::COMPARISON, Neither, Bin(BinOp::GreaterOrEqual)),
        Kind::Char(b'.') => (power::CONCAT, Left, Bin(BinOp::Concat)),
        Kind::Op(Op::ShiftLeft | Op::ShiftRight) => {
            (power::SHIFT, Left, Refused("bitwise operators"))
        }
        Kind::Char(b'+') => (power::ADD, Left, Bin(BinOp::Add)),
        Kind::Char(b'-') => (power::ADD, Left, Bin(BinOp::Sub)),
        Kind::Char(b'*') => (power::MUL, Left, Bin(BinOp::Mul)),
        Kind::Char(b'/') => (power::MUL, Left, Bin(BinOp::Div)),
        Kind::Char(b'%') => (power::MUL, Left, Bin(BinOp::Mod)),
        Kind::Keyword(Kw::InstanceOf) => (power::INSTANCEOF, Left, Refused("objects")),
        Kind::Op(Op::Pow) => (power::POW, Right, Bin(BinOp::Pow)),
        _ => return None,
    })
}

/// The prefix operator a token is, and the binding power of its operand.
fn prefix(k: Kind) -> Option<(i64, Prefix)> {
    Some(match k {
        Kind::Char(b'!') => (power::NOT, Prefix::Unary(UnOp::Not)),
        Kind::Char(b'-') => (power::UNARY, Prefix::Unary(UnOp::Neg)),
        Kind::Char(b'+') => (power::UNARY, Prefix::Unary(UnOp::Plus)),
        Kind::Op(Op::Inc) => (power::UNARY, Prefix::IncDec(true)),
        Kind::Op(Op::Dec) => (power::UNARY, Prefix::IncDec(false)),
        Kind::Char(b'~') => (power::UNARY, Prefix::Refused("bitwise operators")),
        Kind::Char(b'@') => (power::UNARY, Prefix::Refused("@")),
        Kind::Cast(_) => (power::UNARY, Prefix::Refused("casts")),
        Kind::Keyword(Kw::Print) => (power::PRINT, Prefix::Print),
        Kind::Keyword(Kw::Include) => (power::PRINT, Prefix::Include(IncludeKind::Include)),
        Kind::Keyword(Kw::IncludeOnce) => (power::PRINT, Prefix::Include(IncludeKind::IncludeOnce)),
        Kind::Keyword(Kw::Require) => (power::PRINT, Prefix::Include(IncludeKind::Require)),
        Kind::Keyword(Kw::RequireOnce) => (power::PRINT, Prefix::Include(IncludeKind::RequireOnce)),
        _ => return None,
    })
}

/// The postfix operator a token is, and its binding power.
fn postfix(k: Kind) -> Option<(i64, Postfix)> {
    Some(match k {
        Kind::Op(Op::Inc) => (power::POSTFIX, Postfix::IncDec(true)),
        Kind::Op(Op::Dec) => (power::POSTFIX, Postfix::IncDec(false)),
        Kind::Char(b'?') => (power::TERNARY, Postfix::Ternary),
        _ => return None,
    })
}

pub fn expr<'s>(i: &mut In<'_, 's>) -> PResult<Expr<'s>> {
    expr_from(i, 0)
}

fn assign<'s>(op: Option<BinOp>, place: Expr<'s>, value: Expr<'s>, line: u32) -> PResult<Expr<'s>> {
    if !place.is_place() {
        return Err(ContextError::new());
    }
    Ok(match op {
        None => Expr::Assign(Box::new(place), Box::new(value), line),
        Some(op) => Expr::AssignOp(op, Box::new(place), Box::new(value), line),
    })
}

fn inc_dec<'s>(pre: bool, inc: bool, place: Expr<'s>, line: u32) -> PResult<Expr<'s>> {
    if !place.is_place() {
        return Err(ContextError::new());
    }
    Ok(Expr::IncDec {
        pre,
        inc,
        place: Box::new(place),
        line,
    })
}

fn fold_prefix<'s>(op: Prefix, a: Expr<'s>, line: u32) -> PResult<Expr<'s>> {
    Ok(match op {
        Prefix::Unary(op) => Expr::Unary(op, Box::new(a), line),
        Prefix::IncDec(inc) => return inc_dec(true, inc, a, line),
        Prefix::Print => Expr::Print(Box::new(a), line),
        Prefix::Include(kind) => Expr::Include(kind, Box::new(a), line),
        Prefix::Refused(what) => Expr::Refused { what, line },
    })
}

fn fold_infix<'s>(op: Infix, a: Expr<'s>, b: Expr<'s>, line: u32) -> PResult<Expr<'s>> {
    Ok(match op {
        Infix::Bin(op) => Expr::Bin(op, Box::new(a), Box::new(b), line),
        Infix::Assign(op) => return assign(op, a, b, line),
        Infix::Coalesce => Expr::Coalesce(Box::new(a), Box::new(b)),
        Infix::Refused(what) => Expr::Refused { what, line },
    })
}

/// An expression whose operators bind at least as tightly as `min`: an operand, or a prefix
/// operator and its operand, then each postfix and infix operator that binds tightly enough.
/// Each operator's node carries its token's line.
fn expr_from<'s>(i: &mut In<'_, 's>, min: i64) -> PResult<Expr<'s>> {
    let mut lhs = match opt(operand).parse_next(i)? {
        Some(e) => e,
        None => {
            let Some((t, (power, op))) = i.first().and_then(|t| prefix(t.kind).map(|p| (*t, p)))
            else {
                return fail.parse_next(i);
            };
            any.parse_next(i)?;
            let a = expr_from(i, power)?;
            fold_prefix(op, a, t.line)?
        }
    };
    // The power of the last infix operator when it groups neither way.
    let mut last_neither = None;
    // Whether `lhs` is a ternary just made here, without parentheses, and whether it is short.
    let mut bare_ternary: Option<bool> = None;
    while let Some(t) = i.first().copied() {
        if let Some((power, op)) = postfix(t.kind) {
            if power < min {
                break;
            }
            any.parse_next(i)?;
            match op {
                Postfix::IncDec(inc) => {
                    lhs = inc_dec(false, inc, lhs, t.line)?;
                    bare_ternary = None;
                }
                Postfix::Ternary => {
                    let (made, short) = ternary(i, lhs)?;
                    lhs = match bare_ternary {
                        Some(inner_short) if !(inner_short && short) => Expr::Refused {
                            what: "a ternary inside a ternary without parentheses",
                            line: t.line,
                        },
                        _ => made,
                    };
                    bare_ternary = Some(short);
                }
            }
            continue;
        }
        if let Some((power, assoc, op)) = infix(t.kind) {
            if power < min || last_neither == Some(power) {
                break;
            }
            let right = match assoc {
                Assoc::Left | Assoc::Neither => power + 1,
                Assoc::Right => power - 1,
            };
            last_neither = matches!(assoc, Assoc::Neither).then_some(power);
            any.parse_next(i)?;
            let rhs = expr_from(i, right)?;
            lhs = fold_infix(op, lhs, rhs, t.line)?;
            bare_ternary = None;
            continue;
        }
        break;
    }
    Ok(lhs)
}

/// After `?`: `then : else`, or `: else` for the short form. Gives back whether it is short.
fn ternary<'s>(i: &mut In<'_, 's>, cond: Expr<'s>) -> PResult<(Expr<'s>, bool)> {
    let then = opt(expr).parse_next(i)?;
    ch(b':').parse_next(i)?;
    let otherwise = expr_from(i, power::TERNARY + 1)?;
    let short = then.is_none();
    Ok((
        Expr::Ternary(Box::new(cond), then.map(Box::new), Box::new(otherwise)),
        short,
    ))
}

/// operand: a value, then any `[key]` after it.
fn operand<'s>(i: &mut In<'_, 's>) -> PResult<Expr<'s>> {
    let mut base = primary(i)?;
    loop {
        if let Some(t) = opt(ch(b'[')).parse_next(i)? {
            let key = opt(expr).parse_next(i)?;
            ch(b']').parse_next(i)?;
            base = Expr::Dim(Box::new(base), key.map(Box::new), t.line);
            continue;
        }
        if let Some(t) = i.first() {
            if matches!(
                t.kind,
                Kind::Op(Op::Arrow | Op::NullsafeArrow | Op::DoubleColon)
            ) {
                let line = t.line;
                any.parse_next(i)?;
                return Ok(Expr::Refused {
                    what: "objects",
                    line,
                });
            }
            if t.kind == Kind::Char(b'(') && !matches!(base, Expr::Call { .. }) {
                return Ok(Expr::Refused {
                    what: "calling a value",
                    line: t.line,
                });
            }
        }
        return Ok(base);
    }
}

/// primary: a literal, a variable, a call, a parenthesised expression, an array, or a construct.
fn primary<'s>(i: &mut In<'_, 's>) -> PResult<Expr<'s>> {
    let Some(t) = i.first().copied() else {
        return fail.parse_next(i);
    };
    match t.kind {
        Kind::Variable => {
            any.parse_next(i)?;
            Ok(Expr::Var(&t.text[1..], t.line))
        }
        Kind::LNumber => {
            any.parse_next(i)?;
            Ok(Expr::Int(int_literal(t.text)))
        }
        Kind::DNumber => {
            any.parse_next(i)?;
            Ok(Expr::Refused {
                what: "floats",
                line: t.line,
            })
        }
        Kind::ConstantEncapsedString => {
            any.parse_next(i)?;
            Ok(match constant_string(t.text) {
                Ok(s) => Expr::Str(s),
                Err(what) => Expr::Refused { what, line: t.line },
            })
        }
        Kind::Char(b'"') => double_quoted(i),
        Kind::StartHeredoc => heredoc(i),
        Kind::Char(b'(') => paren(i),
        Kind::Char(b'[') => delimited(ch(b'['), array_items, ch(b']'))
            .map(|items| Expr::Array(items, t.line))
            .parse_next(i),
        Kind::Keyword(Kw::Array) if i.get(1).is_some_and(|t| t.kind == Kind::Char(b'(')) => {
            delimited((any, ch(b'(')), array_items, ch(b')'))
                .map(|items| Expr::Array(items, t.line))
                .parse_next(i)
        }
        Kind::Keyword(Kw::Isset) => delimited(
            (any, ch(b'(')),
            terminated(separated(1.., expr, ch(b',')), opt(ch(b','))),
            ch(b')'),
        )
        .map(Expr::Isset)
        .parse_next(i),
        Kind::Keyword(Kw::Empty) => delimited((any, ch(b'(')), expr, ch(b')'))
            .map(|e| Expr::Empty(Box::new(e)))
            .parse_next(i),
        Kind::Keyword(Kw::Match) => match_expr(i),
        Kind::Magic(Magic::Dir) => any.map(|_| Expr::Dir).parse_next(i),
        Kind::Magic(Magic::File) => any.map(|_| Expr::File).parse_next(i),
        Kind::Magic(Magic::Line) => any.map(|t: Tok<'s>| Expr::Line(t.line)).parse_next(i),
        Kind::String | Kind::NameFullyQualified => name(i),
        Kind::Keyword(Kw::Function | Kw::Fn) => {
            any.parse_next(i)?;
            Ok(Expr::Refused {
                what: "closures",
                line: t.line,
            })
        }
        _ => fail.parse_next(i),
    }
}

/// A name: a call `name(args)`, or a constant.
fn name<'s>(i: &mut In<'_, 's>) -> PResult<Expr<'s>> {
    let t = any.parse_next(i)?;
    let text = if t.kind == Kind::NameFullyQualified {
        &t.text[1..]
    } else {
        t.text
    };
    if opt(ch(b'(')).parse_next(i)?.is_some() {
        let args = terminated(separated(0.., argument, ch(b',')), opt(ch(b','))).parse_next(i)?;
        ch(b')').parse_next(i)?;
        return Ok(Expr::Call {
            name: text,
            args,
            line: t.line,
        });
    }
    let lower = text.to_ascii_lowercase();
    Ok(match lower.as_slice() {
        b"true" => Expr::Bool(true),
        b"false" => Expr::Bool(false),
        b"null" => Expr::Null,
        _ => match text {
            b"PHP_EOL" => Expr::Str(b"\n".to_vec()),
            b"PHP_INT_MAX" => Expr::Int(i64::MAX),
            b"PHP_INT_MIN" => Expr::Int(i64::MIN),
            b"PHP_INT_SIZE" => Expr::Int(8),
            _ => Expr::Refused {
                what: "constants",
                line: t.line,
            },
        },
    })
}

fn argument<'s>(i: &mut In<'_, 's>) -> PResult<Expr<'s>> {
    if let (Some(a), Some(b)) = (i.first(), i.get(1)) {
        if a.kind == Kind::String && b.kind == Kind::Char(b':') {
            return Ok(Expr::Refused {
                what: "named arguments",
                line: a.line,
            });
        }
        if a.kind == Kind::Op(Op::Ellipsis) {
            return Ok(Expr::Refused {
                what: "spreads",
                line: a.line,
            });
        }
    }
    expr(i)
}

/// array_items: `value`, `key => value`, separated by commas, a trailing comma allowed.
#[allow(clippy::type_complexity)]
fn array_items<'s>(i: &mut In<'_, 's>) -> PResult<Vec<(Option<Expr<'s>>, Expr<'s>)>> {
    terminated(separated(0.., array_item, ch(b',')), opt(ch(b','))).parse_next(i)
}

fn array_item<'s>(i: &mut In<'_, 's>) -> PResult<(Option<Expr<'s>>, Expr<'s>)> {
    if let Some(t) = i.first()
        && matches!(
            t.kind,
            Kind::Op(Op::Ellipsis) | Kind::AmpVar | Kind::AmpNotVar
        )
    {
        return fail.parse_next(i);
    }
    let first = expr(i)?;
    match opt(preceded(kind(Kind::Op(Op::DoubleArrow)), expr)).parse_next(i)? {
        Some(value) => Ok((Some(first), value)),
        None => Ok((None, first)),
    }
}

/// match_expr: `match (subject) { a, b => x, default => y }`.
fn match_expr<'s>(i: &mut In<'_, 's>) -> PResult<Expr<'s>> {
    let t = kw(Kw::Match).parse_next(i)?;
    let subject = paren(i)?;
    let arms = delimited(
        ch(b'{'),
        terminated(separated(1.., match_arm, ch(b',')), opt(ch(b','))),
        ch(b'}'),
    )
    .parse_next(i)?;
    Ok(Expr::Match {
        subject: Box::new(subject),
        arms,
        line: t.line,
    })
}

#[allow(clippy::type_complexity)]
fn match_arm<'s>(i: &mut In<'_, 's>) -> PResult<(Option<Vec<Expr<'s>>>, Expr<'s>)> {
    let conds = alt((
        kw(Kw::Default).map(|_| None),
        terminated(separated(1.., expr, ch(b',')), opt(ch(b','))).map(Some),
    ))
    .parse_next(i)?;
    let value = preceded(kind(Kind::Op(Op::DoubleArrow)), expr).parse_next(i)?;
    Ok((conds, value))
}

// ---------------------------------------------------------------------------------------------
// Strings

/// A double-quoted string with interpolation: `"` parts `"`.
fn double_quoted<'s>(i: &mut In<'_, 's>) -> PResult<Expr<'s>> {
    let open = ch(b'"').parse_next(i)?;
    let mut parts = Vec::new();
    let mut refused = None;
    loop {
        let t = any.parse_next(i)?;
        match t.kind {
            Kind::Char(b'"') => break,
            Kind::EncapsedAndWhitespace => match cook(t.text, Quote::Double) {
                Ok(text) => parts.push(Part::Text(text)),
                Err(what) => {
                    refused.get_or_insert(Expr::Refused { what, line: t.line });
                }
            },
            _ => parts.push(Part::Expr(interpolated(i, t)?)),
        }
    }
    Ok(refused.unwrap_or(Expr::Interp(parts, open.line)))
}

/// A heredoc or a nowdoc: `<<<LABEL` parts `LABEL`. The closing label's indentation is taken
/// from every line, and the last newline dropped. Every line is indented with the closing label's
/// character as far as it is indented; a line with content, or one an interpolation begins, at
/// least as far as the closing label, where a blank line may stop short.
fn heredoc<'s>(i: &mut In<'_, 's>) -> PResult<Expr<'s>> {
    let start = kind(Kind::StartHeredoc).parse_next(i)?;
    let nowdoc = start.text.contains(&b'\'');
    let mut raw: Vec<(Result<&'s [u8], Expr<'s>>, u32)> = Vec::new();
    let end = loop {
        let t = any.parse_next(i)?;
        match t.kind {
            Kind::EndHeredoc => break t,
            Kind::EncapsedAndWhitespace => raw.push((Ok(t.text), t.line)),
            _ => raw.push((Err(interpolated(i, t)?), t.line)),
        }
    };
    let refused = |what, line| Ok(Expr::Refused { what, line });
    let blank = |b: &u8| *b == b' ' || *b == b'\t';
    let margin = &end.text[..end.text.iter().take_while(|b| blank(b)).count()];
    if margin.contains(&b' ') && margin.contains(&b'\t') {
        return refused(
            "a heredoc's closing label indented with tabs and spaces mixed",
            end.line,
        );
    }
    let mut parts = Vec::new();
    let mut at_line_start = true;
    let last = raw.len().saturating_sub(1);
    for (n, (piece, mut line)) in raw.into_iter().enumerate() {
        let first_line = line;
        match piece {
            Ok(text) => {
                let mut out = Vec::with_capacity(text.len());
                let mut k = 0;
                while k < text.len() {
                    if at_line_start {
                        at_line_start = false;
                        let blanks = text[k..].iter().take_while(|b| blank(b)).count();
                        // A line runs to its newline; where this piece ends first, an
                        // interpolation follows on the same line.
                        let content = match text.get(k + blanks) {
                            Some(b'\n' | b'\r') => false,
                            Some(_) => true,
                            None => n < last,
                        };
                        let taken = blanks.min(margin.len());
                        if content && blanks < margin.len() {
                            return refused(
                                "a heredoc line indented less than its closing label",
                                line,
                            );
                        }
                        if text[k..k + taken].iter().any(|&b| b != margin[0]) {
                            return refused(
                                "a heredoc line indented with tabs and spaces mixed",
                                line,
                            );
                        }
                        k += taken;
                        continue;
                    }
                    out.push(text[k]);
                    at_line_start = text[k] == b'\n';
                    if at_line_start {
                        line += 1;
                    }
                    k += 1;
                }
                if n == last {
                    if out.ends_with(b"\r\n") {
                        out.truncate(out.len() - 2);
                    } else if out.ends_with(b"\n") {
                        out.pop();
                    }
                }
                let text = if nowdoc {
                    out
                } else {
                    match cook(&out, Quote::Heredoc) {
                        Ok(text) => text,
                        Err(what) => {
                            // An escape lies on one line: the first that does not cook.
                            let at = out
                                .split(|&b| b == b'\n')
                                .position(|l| cook(l, Quote::Heredoc).is_err())
                                .unwrap_or(0);
                            return refused(what, first_line + at as u32);
                        }
                    }
                };
                parts.push(Part::Text(text));
            }
            Err(e) => {
                if at_line_start && !margin.is_empty() {
                    return refused("a heredoc line indented less than its closing label", line);
                }
                at_line_start = false;
                parts.push(Part::Expr(e));
            }
        }
    }
    Ok(Expr::Interp(parts, start.line))
}

/// One interpolation inside a string, its first token already taken.
fn interpolated<'s>(i: &mut In<'_, 's>, t: Tok<'s>) -> PResult<Expr<'s>> {
    match t.kind {
        Kind::Variable => {
            let var = Expr::Var(&t.text[1..], t.line);
            if opt(ch(b'[')).parse_next(i)?.is_none() {
                if let Some(a) = i.first()
                    && matches!(a.kind, Kind::Op(Op::Arrow | Op::NullsafeArrow))
                {
                    return Ok(Expr::Refused {
                        what: "objects",
                        line: a.line,
                    });
                }
                return Ok(var);
            }
            let k = any.parse_next(i)?;
            let key = match k.kind {
                Kind::String => Expr::Str(k.text.to_vec()),
                Kind::NumString => num_string(k.text, false),
                Kind::Variable => Expr::Var(&k.text[1..], k.line),
                Kind::Char(b'-') => {
                    let n = kind(Kind::NumString).parse_next(i)?;
                    num_string(n.text, true)
                }
                _ => return fail.parse_next(i),
            };
            ch(b']').parse_next(i)?;
            Ok(Expr::Dim(Box::new(var), Some(Box::new(key)), t.line))
        }
        Kind::CurlyOpen => terminated(expr, ch(b'}')).parse_next(i),
        Kind::DollarOpenCurlyBraces => Ok(Expr::Refused {
            what: "${…} in strings",
            line: t.line,
        }),
        _ => fail.parse_next(i),
    }
}

/// A key written bare inside a string: a whole decimal number written the one way PHP writes it
/// is an int, anything else a string (`-0` among them).
fn num_string<'s>(text: &[u8], negative: bool) -> Expr<'s> {
    let canonical =
        (text == b"0" && !negative) || (text[0] != b'0' && text.iter().all(u8::is_ascii_digit));
    let mut s = Vec::with_capacity(text.len() + 1);
    if negative {
        s.push(b'-');
    }
    s.extend_from_slice(text);
    if canonical
        && let Some(n) = std::str::from_utf8(&s)
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
    {
        return Expr::Int(n);
    }
    Expr::Str(s)
}

fn int_literal(text: &[u8]) -> i64 {
    let clean: String = text
        .iter()
        .filter(|&&b| b != b'_')
        .map(|&b| b as char)
        .collect();
    let lower = clean.to_ascii_lowercase();
    let parsed = if let Some(h) = lower.strip_prefix("0x") {
        i64::from_str_radix(h, 16)
    } else if let Some(b) = lower.strip_prefix("0b") {
        i64::from_str_radix(b, 2)
    } else if let Some(o) = lower.strip_prefix("0o") {
        i64::from_str_radix(o, 8)
    } else if lower.len() > 1 && lower.starts_with('0') {
        i64::from_str_radix(&lower[1..], 8)
    } else {
        lower.parse()
    };
    parsed.expect("the lexer passes only integers that fit")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    Double,
    Heredoc,
}

/// A whole constant string token: single-quoted, or double-quoted with nothing interpolated.
fn constant_string(text: &[u8]) -> Result<Vec<u8>, &'static str> {
    let body = &text[1..text.len() - 1];
    if text[0] == b'\'' {
        let mut out = Vec::with_capacity(body.len());
        let mut k = 0;
        while k < body.len() {
            if body[k] == b'\\' && matches!(body.get(k + 1), Some(b'\\') | Some(b'\'')) {
                out.push(body[k + 1]);
                k += 2;
            } else {
                out.push(body[k]);
                k += 1;
            }
        }
        Ok(out)
    } else {
        cook(body, Quote::Double)
    }
}

/// A code point as UTF-8, a surrogate's three bytes too, as PHP writes `\u{…}`.
fn push_utf8(out: &mut Vec<u8>, c: u32) {
    let cont = |shift: u32| 0x80 | ((c >> shift) & 0x3f) as u8;
    match c {
        0..=0x7f => out.push(c as u8),
        0x80..=0x7ff => out.extend([0xc0 | (c >> 6) as u8, cont(0)]),
        0x800..=0xffff => out.extend([0xe0 | (c >> 12) as u8, cont(6), cont(0)]),
        _ => out.extend([0xf0 | (c >> 18) as u8, cont(12), cont(6), cont(0)]),
    }
}

/// The escapes of a double-quoted string or a heredoc. An escape PHP does not know stays as it
/// is written, backslash and all. An escape PHP refuses, or warns about, is refused.
fn cook(raw: &[u8], quote: Quote) -> Result<Vec<u8>, &'static str> {
    let mut out = Vec::with_capacity(raw.len());
    let mut k = 0;
    while k < raw.len() {
        let b = raw[k];
        if b != b'\\' || k + 1 >= raw.len() {
            out.push(b);
            k += 1;
            continue;
        }
        let c = raw[k + 1];
        let simple = match c {
            b'n' => Some(b'\n'),
            b't' => Some(b'\t'),
            b'r' => Some(b'\r'),
            b'v' => Some(0x0b),
            b'e' => Some(0x1b),
            b'f' => Some(0x0c),
            b'\\' => Some(b'\\'),
            b'$' => Some(b'$'),
            b'"' if quote == Quote::Double => Some(b'"'),
            _ => None,
        };
        if let Some(s) = simple {
            out.push(s);
            k += 2;
            continue;
        }
        if (b'0'..=b'7').contains(&c) {
            let digits = raw[k + 1..]
                .iter()
                .take(3)
                .take_while(|b| (b'0'..=b'7').contains(*b))
                .count();
            let v = raw[k + 1..k + 1 + digits]
                .iter()
                .fold(0u32, |v, &d| v * 8 + (d - b'0') as u32);
            if v > 0o377 {
                return Err("an octal escape past \\377");
            }
            out.push(v as u8);
            k += 1 + digits;
            continue;
        }
        if c == b'x' {
            let digits = raw[k + 2..]
                .iter()
                .take(2)
                .take_while(|b| b.is_ascii_hexdigit())
                .count();
            if digits > 0 {
                let hex = std::str::from_utf8(&raw[k + 2..k + 2 + digits]).expect("hex digits");
                out.push(u8::from_str_radix(hex, 16).expect("hex digits"));
                k += 2 + digits;
                continue;
            }
        }
        if c == b'u' && raw.get(k + 2) == Some(&b'{') {
            let bad = "a \\u{…} escape that is not a code point";
            let close = raw[k + 3..].iter().position(|&b| b == b'}').ok_or(bad)?;
            let hex = &raw[k + 3..k + 3 + close];
            let code = (!hex.is_empty() && hex.iter().all(u8::is_ascii_hexdigit))
                .then(|| u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok())
                .flatten()
                .filter(|&c| c <= 0x10ffff)
                .ok_or(bad)?;
            push_utf8(&mut out, code);
            k += 4 + close;
            continue;
        }
        out.push(b);
        k += 1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lex::lex;

    fn try_parse(src: &str) -> Result<Vec<Stmt<'_>>, ParseError> {
        let toks = lex(src.as_bytes()).unwrap();
        parse_page(&significant(src.as_bytes(), &toks))
    }

    fn one_expr(src: &str) -> Expr<'_> {
        match try_parse(src).unwrap().remove(0).kind {
            StmtKind::Expr(e) => e,
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn concatenation_binds_looser_than_addition() {
        let e = one_expr("<?php 'x' . 1 + 2;");
        let Expr::Bin(BinOp::Concat, _, rhs, _) = e else {
            panic!("{e:?}")
        };
        assert!(matches!(*rhs, Expr::Bin(BinOp::Add, ..)));
    }

    #[test]
    fn unary_minus_binds_looser_than_pow() {
        let e = one_expr("<?php -2 ** 2;");
        assert!(
            matches!(e, Expr::Unary(UnOp::Neg, ref a, _) if matches!(**a, Expr::Bin(BinOp::Pow, ..)))
        );
    }

    #[test]
    fn coalesce_binds_tighter_than_the_ternary() {
        let e = one_expr("<?php $a ?? $b ? 1 : 2;");
        assert!(matches!(e, Expr::Ternary(ref c, ..) if matches!(**c, Expr::Coalesce(..))));
    }

    #[test]
    fn an_operator_carries_its_line() {
        let e = one_expr("<?php 7\n/\n2;");
        assert!(matches!(e, Expr::Bin(BinOp::Div, _, _, 2)), "{e:?}");
    }

    #[test]
    fn two_comparisons_in_a_row_do_not_parse() {
        assert!(try_parse("<?php 1 == 2 == 3;").is_err());
        assert!(try_parse("<?php 1 < 2 > 3;").is_err());
    }

    #[test]
    fn a_ternary_in_a_ternary_needs_parentheses_unless_both_are_short() {
        for src in [
            "<?php 1 ? 2 : 3 ? 4 : 5;",
            "<?php 1 ?: 2 ? 3 : 4;",
            "<?php 1 ? 2 : 3 ?: 4;",
        ] {
            assert!(matches!(one_expr(src), Expr::Refused { .. }), "{src}");
        }
        for src in [
            "<?php 1 ?: 2 ?: 3;",
            "<?php (1 ? 2 : 3) ? 4 : 5;",
            "<?php 1 ? 2 ? 3 : 4 : 5;",
        ] {
            assert!(matches!(one_expr(src), Expr::Ternary(..)), "{src}");
        }
    }

    #[test]
    fn escapes_cook_and_unknown_ones_stay() {
        assert_eq!(
            cook(br"a\tb\q\x41\101\u{263A}\u{D800}", Quote::Double).unwrap(),
            b"a\tb\\qAA\xe2\x98\xba\xed\xa0\x80"
        );
        for bad in [
            &br"\u{}"[..],
            br"\u{zz}",
            br"\u{41",
            br"\u{110000}",
            br"\400",
        ] {
            assert!(cook(bad, Quote::Double).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_heredoc_line_is_held_to_its_closing_label() {
        let refused = |src: &str| matches!(one_expr(src), Expr::Refused { .. });
        assert!(refused("<?php <<<X\n  a\n    X;"));
        assert!(refused("<?php <<<X\n\t    a\n    X;"));
        assert!(refused("<?php <<<X\n\t a\n\t X;"));
        assert!(refused("<?php <<<X\n{$v} a\n    X;"));
        assert!(refused("<?php <<<X\n    a\n\t\n    X;"));
        assert!(!refused("<?php <<<X\n    a\n\n  \n    \tb\n    X;"));
    }

    #[test]
    fn a_minus_zero_key_in_a_string_stays_a_string() {
        assert_eq!(num_string(b"0", true), Expr::Str(b"-0".to_vec()));
        assert_eq!(num_string(b"0", false), Expr::Int(0));
        assert_eq!(num_string(b"7", true), Expr::Int(-7));
    }

    #[test]
    fn a_class_is_refused_by_name() {
        let err = try_parse("<?php class A {}").unwrap_err();
        assert_eq!(err.what, "whp does not run classes yet");
    }

    #[test]
    fn a_typed_function_is_refused_by_name() {
        let stmts = try_parse("<?php\nfunction f(int $a): string { return 'x'; }").unwrap();
        let StmtKind::Function(f) = &stmts[0].kind else {
            panic!()
        };
        assert_eq!(f.refused, Some(("type declarations", 2)));
    }
}
