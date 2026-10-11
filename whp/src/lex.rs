// Copyright (C) 2026 Kenneth Allen Flegal and DappleAI
// SPDX-License-Identifier: GPL-3.0-or-later

//! The lexer: a page's bytes into tokens, named as PHP's manual names them.
//!
//! A page is lexed in modes: the HTML outside the tags, the code inside them, and the parts of a
//! double-quoted string or a heredoc. Every byte of the page lies in exactly one token, so the
//! tokens' texts joined give the page back.

use std::fmt;

/// One token: its kind, where its text lies in the page, and the line it starts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    /// What the token is.
    pub kind: Kind,
    /// Where its text begins in the page, in bytes.
    pub start: u32,
    /// Where its text ends in the page, in bytes.
    pub end: u32,
    /// The line it starts on.
    pub line: u32,
}

impl Token {
    /// The token's text, out of the page it was lexed from.
    pub fn text<'s>(&self, src: &'s [u8]) -> &'s [u8] {
        &src[self.start as usize..self.end as usize]
    }
}

/// What a token is. Each kind is a token of the PHP manual's *List of Parser Tokens*, named
/// after it (`InlineHtml` is `T_INLINE_HTML`), and [`Kind::name`] gives the manual's name; a
/// token of one character is `Char`, named by its character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum Kind {
    InlineHtml,
    OpenTag,
    OpenTagWithEcho,
    CloseTag,
    Whitespace,
    Comment,
    DocComment,
    Attribute,
    Variable,
    String,
    NameQualified,
    NameFullyQualified,
    NameRelative,
    NsSeparator,
    LNumber,
    DNumber,
    ConstantEncapsedString,
    EncapsedAndWhitespace,
    StartHeredoc,
    EndHeredoc,
    CurlyOpen,
    DollarOpenCurlyBraces,
    NumString,
    StringVarname,
    Keyword(Kw),
    Magic(Magic),
    Op(Op),
    Cast(Cast),
    AmpVar,
    AmpNotVar,
    Char(u8),
    BadCharacter,
}

macro_rules! names {
    ($(#[$doc:meta])* $ty:ident { $($v:ident => $text:literal, $name:literal;)* }) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[allow(missing_docs)]
        pub enum $ty { $($v,)* }
        impl $ty {
            /// Every one, in the order the lexer tries them.
            pub const ALL: &'static [$ty] = &[$($ty::$v,)*];
            /// How it is written in a page.
            pub fn text(self) -> &'static str { match self { $($ty::$v => $text,)* } }
            /// The name the manual gives its token.
            pub fn name(self) -> &'static str { match self { $($ty::$v => $name,)* } }
        }
    };
}

names!(
    /// A keyword: a word PHP reserves, found without regard to case.
    Kw {
    Abstract => "abstract", "T_ABSTRACT";
    And => "and", "T_LOGICAL_AND";
    Array => "array", "T_ARRAY";
    As => "as", "T_AS";
    Break => "break", "T_BREAK";
    Callable => "callable", "T_CALLABLE";
    Case => "case", "T_CASE";
    Catch => "catch", "T_CATCH";
    Class => "class", "T_CLASS";
    Clone => "clone", "T_CLONE";
    Const => "const", "T_CONST";
    Continue => "continue", "T_CONTINUE";
    Declare => "declare", "T_DECLARE";
    Default => "default", "T_DEFAULT";
    Die => "die", "T_EXIT";
    Do => "do", "T_DO";
    Echo => "echo", "T_ECHO";
    Else => "else", "T_ELSE";
    ElseIf => "elseif", "T_ELSEIF";
    Empty => "empty", "T_EMPTY";
    EndDeclare => "enddeclare", "T_ENDDECLARE";
    EndFor => "endfor", "T_ENDFOR";
    EndForeach => "endforeach", "T_ENDFOREACH";
    EndIf => "endif", "T_ENDIF";
    EndSwitch => "endswitch", "T_ENDSWITCH";
    EndWhile => "endwhile", "T_ENDWHILE";
    Eval => "eval", "T_EVAL";
    Exit => "exit", "T_EXIT";
    Extends => "extends", "T_EXTENDS";
    Final => "final", "T_FINAL";
    Finally => "finally", "T_FINALLY";
    Fn => "fn", "T_FN";
    For => "for", "T_FOR";
    Foreach => "foreach", "T_FOREACH";
    Function => "function", "T_FUNCTION";
    Global => "global", "T_GLOBAL";
    Goto => "goto", "T_GOTO";
    If => "if", "T_IF";
    Implements => "implements", "T_IMPLEMENTS";
    Include => "include", "T_INCLUDE";
    IncludeOnce => "include_once", "T_INCLUDE_ONCE";
    InstanceOf => "instanceof", "T_INSTANCEOF";
    InsteadOf => "insteadof", "T_INSTEADOF";
    Interface => "interface", "T_INTERFACE";
    Isset => "isset", "T_ISSET";
    List => "list", "T_LIST";
    Match => "match", "T_MATCH";
    Namespace => "namespace", "T_NAMESPACE";
    New => "new", "T_NEW";
    Or => "or", "T_LOGICAL_OR";
    Print => "print", "T_PRINT";
    Private => "private", "T_PRIVATE";
    Protected => "protected", "T_PROTECTED";
    Public => "public", "T_PUBLIC";
    Readonly => "readonly", "T_READONLY";
    Require => "require", "T_REQUIRE";
    RequireOnce => "require_once", "T_REQUIRE_ONCE";
    Return => "return", "T_RETURN";
    Static => "static", "T_STATIC";
    Switch => "switch", "T_SWITCH";
    Throw => "throw", "T_THROW";
    Trait => "trait", "T_TRAIT";
    Try => "try", "T_TRY";
    Unset => "unset", "T_UNSET";
    Use => "use", "T_USE";
    Var => "var", "T_VAR";
    While => "while", "T_WHILE";
    Xor => "xor", "T_LOGICAL_XOR";
    Yield => "yield", "T_YIELD";
});

names!(
    /// A magic constant, such as `__DIR__`, found without regard to case.
    Magic {
    Class => "__class__", "T_CLASS_C";
    Dir => "__dir__", "T_DIR";
    File => "__file__", "T_FILE";
    Function => "__function__", "T_FUNC_C";
    Line => "__line__", "T_LINE";
    Method => "__method__", "T_METHOD_C";
    Namespace => "__namespace__", "T_NS_C";
    Trait => "__trait__", "T_TRAIT_C";
    Property => "__property__", "T_PROPERTY_C";
});

names!(
    /// An operator of more than one character; the lexer tries the longest first.
    Op {
    PowEqual => "**=", "T_POW_EQUAL";
    Ellipsis => "...", "T_ELLIPSIS";
    ShiftLeftEqual => "<<=", "T_SL_EQUAL";
    ShiftRightEqual => ">>=", "T_SR_EQUAL";
    Identical => "===", "T_IS_IDENTICAL";
    NotIdentical => "!==", "T_IS_NOT_IDENTICAL";
    CoalesceEqual => "??=", "T_COALESCE_EQUAL";
    NullsafeArrow => "?->", "T_NULLSAFE_OBJECT_OPERATOR";
    Spaceship => "<=>", "T_SPACESHIP";
    Pow => "**", "T_POW";
    Inc => "++", "T_INC";
    Dec => "--", "T_DEC";
    Arrow => "->", "T_OBJECT_OPERATOR";
    DoubleArrow => "=>", "T_DOUBLE_ARROW";
    DoubleColon => "::", "T_DOUBLE_COLON";
    Equal => "==", "T_IS_EQUAL";
    NotEqual => "!=", "T_IS_NOT_EQUAL";
    NotEqualAngle => "<>", "T_IS_NOT_EQUAL";
    SmallerOrEqual => "<=", "T_IS_SMALLER_OR_EQUAL";
    GreaterOrEqual => ">=", "T_IS_GREATER_OR_EQUAL";
    BooleanAnd => "&&", "T_BOOLEAN_AND";
    BooleanOr => "||", "T_BOOLEAN_OR";
    Pipe => "|>", "T_PIPE";
    Coalesce => "??", "T_COALESCE";
    PlusEqual => "+=", "T_PLUS_EQUAL";
    MinusEqual => "-=", "T_MINUS_EQUAL";
    MulEqual => "*=", "T_MUL_EQUAL";
    DivEqual => "/=", "T_DIV_EQUAL";
    ConcatEqual => ".=", "T_CONCAT_EQUAL";
    ModEqual => "%=", "T_MOD_EQUAL";
    AndEqual => "&=", "T_AND_EQUAL";
    OrEqual => "|=", "T_OR_EQUAL";
    XorEqual => "^=", "T_XOR_EQUAL";
    ShiftLeft => "<<", "T_SL";
    ShiftRight => ">>", "T_SR";
});

names!(
    /// A cast, such as `(int)`.
    Cast {
    Int => "int", "T_INT_CAST";
    Integer => "integer", "T_INT_CAST";
    Bool => "bool", "T_BOOL_CAST";
    Boolean => "boolean", "T_BOOL_CAST";
    Float => "float", "T_DOUBLE_CAST";
    Double => "double", "T_DOUBLE_CAST";
    Real => "real", "T_DOUBLE_CAST";
    String => "string", "T_STRING_CAST";
    Binary => "binary", "T_STRING_CAST";
    Array => "array", "T_ARRAY_CAST";
    Object => "object", "T_OBJECT_CAST";
    Unset => "unset", "T_UNSET_CAST";
    Void => "void", "T_VOID_CAST";
});

impl Kind {
    /// The name PHP gives the token; a single-character token is named by its character.
    pub fn name(&self) -> NameOf {
        NameOf(*self)
    }
}

/// A token's name as the manual gives it, written out by its `Display`.
pub struct NameOf(Kind);

impl fmt::Display for NameOf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self.0 {
            Kind::InlineHtml => "T_INLINE_HTML",
            Kind::OpenTag => "T_OPEN_TAG",
            Kind::OpenTagWithEcho => "T_OPEN_TAG_WITH_ECHO",
            Kind::CloseTag => "T_CLOSE_TAG",
            Kind::Whitespace => "T_WHITESPACE",
            Kind::Comment => "T_COMMENT",
            Kind::DocComment => "T_DOC_COMMENT",
            Kind::Attribute => "T_ATTRIBUTE",
            Kind::Variable => "T_VARIABLE",
            Kind::String => "T_STRING",
            Kind::NameQualified => "T_NAME_QUALIFIED",
            Kind::NameFullyQualified => "T_NAME_FULLY_QUALIFIED",
            Kind::NameRelative => "T_NAME_RELATIVE",
            Kind::NsSeparator => "T_NS_SEPARATOR",
            Kind::LNumber => "T_LNUMBER",
            Kind::DNumber => "T_DNUMBER",
            Kind::ConstantEncapsedString => "T_CONSTANT_ENCAPSED_STRING",
            Kind::EncapsedAndWhitespace => "T_ENCAPSED_AND_WHITESPACE",
            Kind::StartHeredoc => "T_START_HEREDOC",
            Kind::EndHeredoc => "T_END_HEREDOC",
            Kind::CurlyOpen => "T_CURLY_OPEN",
            Kind::DollarOpenCurlyBraces => "T_DOLLAR_OPEN_CURLY_BRACES",
            Kind::NumString => "T_NUM_STRING",
            Kind::StringVarname => "T_STRING_VARNAME",
            Kind::Keyword(k) => k.name(),
            Kind::Magic(m) => m.name(),
            Kind::Op(o) => o.name(),
            Kind::Cast(c) => c.name(),
            Kind::BadCharacter => "T_BAD_CHARACTER",
            Kind::AmpVar => "T_AMPERSAND_FOLLOWED_BY_VAR_OR_VARARG",
            Kind::AmpNotVar => "T_AMPERSAND_NOT_FOLLOWED_BY_VAR_OR_VARARG",
            Kind::Char(c) => return write!(f, "{}", c as char),
        };
        f.write_str(s)
    }
}

/// Why a page could not be lexed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexError {
    /// The line it stopped on.
    pub line: u32,
    /// What it met there, in plain words.
    pub what: &'static str,
}

/// How the lexer reads the next bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Html,
    Code,
    /// Code inside `{$ … }` in a string: a `}` at depth 0 returns to the string.
    CodeInString {
        depth: u32,
    },
    DoubleQuoted,
    Heredoc {
        label_start: u32,
        label_end: u32,
    },
    /// Just after `$name[` in a string: one key, then `]`.
    VarOffset,
    /// Just after `${` in a string.
    LookingForVarname,
}

fn ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b >= 0x80
}

fn ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

fn starts_with_ci(s: &[u8], prefix: &[u8]) -> bool {
    s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// The length of an identifier at the start of `s`, or 0.
fn ident(s: &[u8]) -> usize {
    match s.first() {
        Some(&b) if ident_start(b) => 1 + s[1..].iter().take_while(|&&b| ident_byte(b)).count(),
        _ => 0,
    }
}

/// The length of a run of spaces and tabs.
fn blanks(s: &[u8]) -> usize {
    s.iter().take_while(|&&b| b == b' ' || b == b'\t').count()
}

/// Lexes a whole page.
pub fn lex(src: &[u8]) -> Result<Vec<Token>, LexError> {
    let mut lx = Lexer {
        src,
        pos: 0,
        line: 1,
        out: Vec::new(),
        modes: vec![Mode::Html],
    };
    lx.run()?;
    Ok(lx.out)
}

struct Lexer<'s> {
    src: &'s [u8],
    pos: usize,
    line: u32,
    out: Vec<Token>,
    modes: Vec<Mode>,
}

impl<'s> Lexer<'s> {
    fn rest(&self) -> &'s [u8] {
        &self.src[self.pos..]
    }

    fn emit(&mut self, kind: Kind, len: usize) {
        let start = self.pos;
        let end = start + len;
        self.out.push(Token {
            kind,
            start: start as u32,
            end: end as u32,
            line: self.line,
        });
        self.line += self.src[start..end].iter().filter(|&&b| b == b'\n').count() as u32;
        self.pos = end;
    }

    fn mode(&self) -> Mode {
        *self.modes.last().expect("a mode")
    }

    fn set_mode(&mut self, m: Mode) {
        *self.modes.last_mut().expect("a mode") = m;
    }

    fn fail(&self, what: &'static str) -> LexError {
        LexError {
            line: self.line,
            what,
        }
    }

    fn run(&mut self) -> Result<(), LexError> {
        while self.pos < self.src.len() {
            match self.mode() {
                Mode::Html => self.html(),
                Mode::Code | Mode::CodeInString { .. } => self.code()?,
                Mode::DoubleQuoted => self.double_quoted()?,
                Mode::Heredoc {
                    label_start,
                    label_end,
                } => self.heredoc_body(label_start, label_end)?,
                Mode::VarOffset => self.var_offset()?,
                Mode::LookingForVarname => self.looking_for_varname(),
            }
        }
        match self.mode() {
            Mode::Html | Mode::Code => Ok(()),
            _ => Err(self.fail("a string or a heredoc is not closed")),
        }
    }

    /// HTML up to the next open tag, then the tag.
    fn html(&mut self) {
        let rest = self.rest();
        let at = rest
            .windows(2)
            .position(|w| w == b"<?")
            .unwrap_or(rest.len());
        if at > 0 {
            self.emit(Kind::InlineHtml, at);
            return;
        }
        let rest = self.rest();
        if starts_with_ci(rest, b"<?php") && (rest.len() == 5 || is_ws(rest[5])) {
            let len = if rest.len() == 5 {
                5
            } else if rest[5] == b'\r' && rest.get(6) == Some(&b'\n') {
                7
            } else {
                6
            };
            self.emit(Kind::OpenTag, len);
        } else if rest.starts_with(b"<?=") {
            self.emit(Kind::OpenTagWithEcho, 3);
        } else {
            self.emit(Kind::OpenTag, 2);
        }
        self.set_mode(Mode::Code);
    }

    /// One token of code.
    fn code(&mut self) -> Result<(), LexError> {
        let rest = self.rest();
        let b = rest[0];
        if is_ws(b) {
            let n = rest.iter().take_while(|&&b| is_ws(b)).count();
            self.emit(Kind::Whitespace, n);
            return Ok(());
        }
        if rest.starts_with(b"?>") && matches!(self.mode(), Mode::Code) {
            let len = if rest[2..].starts_with(b"\r\n") {
                4
            } else if rest[2..].starts_with(b"\n") {
                3
            } else {
                2
            };
            self.emit(Kind::CloseTag, len);
            self.set_mode(Mode::Html);
            return Ok(());
        }
        if rest.starts_with(b"#[") {
            self.emit(Kind::Attribute, 2);
            return Ok(());
        }
        if b == b'#' || rest.starts_with(b"//") {
            self.emit(Kind::Comment, line_comment(rest));
            return Ok(());
        }
        if rest.starts_with(b"/*") {
            let n = match rest[2..].windows(2).position(|w| w == b"*/") {
                Some(i) => i + 4,
                None => rest.len(),
            };
            let doc = rest.starts_with(b"/**") && n > 4 && is_ws(rest[3]);
            self.emit(if doc { Kind::DocComment } else { Kind::Comment }, n);
            return Ok(());
        }
        if b == b'$' {
            let n = ident(&rest[1..]);
            if n > 0 {
                self.emit(Kind::Variable, 1 + n);
            } else {
                self.emit(Kind::Char(b'$'), 1);
            }
            return Ok(());
        }
        if b == b'\'' {
            let n = single_quoted(rest)
                .ok_or_else(|| self.fail("a single-quoted string is not closed"))?;
            self.emit(Kind::ConstantEncapsedString, n);
            return Ok(());
        }
        if b == b'"' {
            match double_quoted_plain(rest) {
                Some(n) => self.emit(Kind::ConstantEncapsedString, n),
                None => {
                    self.emit(Kind::Char(b'"'), 1);
                    self.modes.push(Mode::DoubleQuoted);
                }
            }
            return Ok(());
        }
        if rest.starts_with(b"<<<")
            && let Some((len, label_start, label_end, nowdoc)) = heredoc_start(rest)
        {
            let base = self.pos as u32;
            self.emit(Kind::StartHeredoc, len);
            let (ls, le) = (base + label_start as u32, base + label_end as u32);
            if nowdoc {
                self.nowdoc_body(ls, le)?;
            } else {
                self.modes.push(Mode::Heredoc {
                    label_start: ls,
                    label_end: le,
                });
            }
            return Ok(());
        }
        if b.is_ascii_digit() || (b == b'.' && rest.get(1).is_some_and(|c| c.is_ascii_digit())) {
            let (n, kind) = number(rest);
            self.emit(kind, n);
            return Ok(());
        }
        if b == b'\\' {
            let n = qualified_tail(rest);
            if n > 0 {
                self.emit(Kind::NameFullyQualified, n);
            } else {
                self.emit(Kind::NsSeparator, 1);
            }
            return Ok(());
        }
        if ident_start(b) {
            let n = ident(rest);
            let tail = qualified_tail(&rest[n..]);
            if tail > 0 {
                let kind = if rest[..n].eq_ignore_ascii_case(b"namespace") {
                    Kind::NameRelative
                } else {
                    Kind::NameQualified
                };
                self.emit(kind, n + tail);
                return Ok(());
            }
            let word = &rest[..n];
            let kind = if self.after_member_operator() {
                Kind::String
            } else {
                word_kind(word)
            };
            self.emit(kind, n);
            return Ok(());
        }
        if b == b'('
            && let Some((n, c)) = cast(rest)
        {
            self.emit(Kind::Cast(c), n);
            return Ok(());
        }
        if b == b'{' {
            if let Mode::CodeInString { depth } = self.mode() {
                self.set_mode(Mode::CodeInString { depth: depth + 1 });
            } else {
                self.modes.push(Mode::Code);
            }
            self.emit(Kind::Char(b'{'), 1);
            return Ok(());
        }
        if b == b'}' {
            match self.mode() {
                Mode::CodeInString { depth: 0 } => {
                    self.modes.pop();
                }
                Mode::CodeInString { depth } => {
                    self.set_mode(Mode::CodeInString { depth: depth - 1 })
                }
                _ if self.modes.len() > 1 => {
                    self.modes.pop();
                }
                _ => {}
            }
            self.emit(Kind::Char(b'}'), 1);
            return Ok(());
        }
        for op in Op::ALL {
            if rest.starts_with(op.text().as_bytes()) {
                self.emit(Kind::Op(*op), op.text().len());
                return Ok(());
            }
        }
        if b == b'&' {
            let after = &rest[1..];
            let after = &after[after.iter().take_while(|&&b| is_ws(b)).count()..];
            let var = (after.first() == Some(&b'$')
                && after.get(1).is_some_and(|&c| ident_start(c)))
                || after.starts_with(b"...");
            self.emit(if var { Kind::AmpVar } else { Kind::AmpNotVar }, 1);
            return Ok(());
        }
        if b";,.=()[]+-*/%<>?:!~@|^`".contains(&b) {
            self.emit(Kind::Char(b), 1);
            return Ok(());
        }
        self.emit(Kind::BadCharacter, 1);
        Ok(())
    }

    /// A keyword after `->`, `?->` or `::` is a name.
    fn after_member_operator(&self) -> bool {
        let last = self
            .out
            .iter()
            .rev()
            .find(|t| !matches!(t.kind, Kind::Whitespace | Kind::Comment | Kind::DocComment));
        matches!(
            last.map(|t| t.kind),
            Some(Kind::Op(Op::Arrow | Op::NullsafeArrow | Op::DoubleColon))
        )
    }

    /// The parts of a double-quoted string: text, then a variable or a `{$`, until the closing `"`.
    fn double_quoted(&mut self) -> Result<(), LexError> {
        let rest = self.rest();
        if rest[0] == b'"' {
            self.emit(Kind::Char(b'"'), 1);
            self.modes.pop();
            return Ok(());
        }
        if self.interpolation()? {
            return Ok(());
        }
        let n = literal_run(rest, |r| r[0] == b'"');
        self.emit(Kind::EncapsedAndWhitespace, n);
        Ok(())
    }

    /// The body of a heredoc: text and interpolations, until the closing label on a line of its own.
    fn heredoc_body(&mut self, label_start: u32, label_end: u32) -> Result<(), LexError> {
        let label = &self.src[label_start as usize..label_end as usize];
        let at_line_start = self.pos == 0 || self.src[self.pos - 1] == b'\n';
        if at_line_start && let Some(n) = closing_label(self.rest(), label) {
            self.emit(Kind::EndHeredoc, n);
            self.modes.pop();
            return Ok(());
        }
        if self.interpolation()? {
            return Ok(());
        }
        // The text stops at an interpolation, or after the newline before the closing line.
        let rest = self.rest();
        let mut i = 0;
        while i < rest.len() {
            let r = &rest[i..];
            match r[0] {
                b'\\' if r.get(1) != Some(&b'\n') => {
                    i += 2;
                    continue;
                }
                b'$' if r.len() > 1 && (ident_start(r[1]) || r[1] == b'{') => break,
                b'{' if r.get(1) == Some(&b'$') => break,
                b'\n' => {
                    i += 1;
                    if closing_label(&rest[i..], label).is_some() {
                        break;
                    }
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        self.emit(Kind::EncapsedAndWhitespace, i.min(rest.len()));
        Ok(())
    }

    /// A nowdoc's body is one token of text, then its closing label.
    fn nowdoc_body(&mut self, label_start: u32, label_end: u32) -> Result<(), LexError> {
        let label = self.src[label_start as usize..label_end as usize].to_vec();
        if let Some(n) = closing_label(self.rest(), &label) {
            self.emit(Kind::EndHeredoc, n);
            return Ok(());
        }
        let rest = self.rest();
        let mut i = 0;
        loop {
            match rest[i..].iter().position(|&b| b == b'\n') {
                Some(nl) => {
                    i += nl + 1;
                    if closing_label(&rest[i..], &label).is_some() {
                        break;
                    }
                }
                None => return Err(self.fail("a nowdoc is not closed")),
            }
        }
        self.emit(Kind::EncapsedAndWhitespace, i);
        let n = closing_label(self.rest(), &label).expect("the closing label");
        self.emit(Kind::EndHeredoc, n);
        Ok(())
    }

    /// A `$name`, `{$` or `${` inside a string. Returns whether one was taken.
    fn interpolation(&mut self) -> Result<bool, LexError> {
        let rest = self.rest();
        if rest[0] == b'$' && rest.len() > 1 && ident_start(rest[1]) {
            let n = 1 + ident(&rest[1..]);
            self.emit(Kind::Variable, n);
            let rest = self.rest();
            if rest.first() == Some(&b'[') {
                self.emit(Kind::Char(b'['), 1);
                self.modes.push(Mode::VarOffset);
            } else if rest.starts_with(b"->") && rest.len() > 2 && ident_start(rest[2]) {
                self.emit(Kind::Op(Op::Arrow), 2);
                let n = ident(self.rest());
                self.emit(Kind::String, n);
            } else if rest.starts_with(b"?->") && rest.len() > 3 && ident_start(rest[3]) {
                self.emit(Kind::Op(Op::NullsafeArrow), 3);
                let n = ident(self.rest());
                self.emit(Kind::String, n);
            }
            return Ok(true);
        }
        if rest.starts_with(b"{$") {
            self.emit(Kind::CurlyOpen, 1);
            self.modes.push(Mode::CodeInString { depth: 0 });
            return Ok(true);
        }
        if rest.starts_with(b"${") {
            self.emit(Kind::DollarOpenCurlyBraces, 2);
            self.modes.push(Mode::LookingForVarname);
            return Ok(true);
        }
        Ok(false)
    }

    /// The key of `$name[…]` inside a string.
    fn var_offset(&mut self) -> Result<(), LexError> {
        let rest = self.rest();
        let b = rest[0];
        if b == b']' {
            self.emit(Kind::Char(b']'), 1);
            self.modes.pop();
        } else if b.is_ascii_digit() {
            let n = rest
                .iter()
                .take_while(|b| b.is_ascii_alphanumeric() || **b == b'_')
                .count();
            self.emit(Kind::NumString, n);
        } else if b == b'$' && rest.len() > 1 && ident_start(rest[1]) {
            let n = 1 + ident(&rest[1..]);
            self.emit(Kind::Variable, n);
        } else if ident_start(b) {
            let n = ident(rest);
            self.emit(Kind::String, n);
        } else if b == b'-' {
            self.emit(Kind::Char(b'-'), 1);
        } else {
            return Err(self.fail("an offset in a string is not a name, a number or a variable"));
        }
        Ok(())
    }

    /// Just after `${`: a name followed by `[` or `}` is a variable's name, else an expression.
    fn looking_for_varname(&mut self) {
        let rest = self.rest();
        let n = ident(rest);
        if n > 0 && matches!(rest.get(n), Some(b'[') | Some(b'}')) {
            self.emit(Kind::StringVarname, n);
        }
        self.set_mode(Mode::CodeInString { depth: 0 });
    }
}

/// A `//` or `#` comment runs to the end of the line or to `?>`, neither included.
fn line_comment(s: &[u8]) -> usize {
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'\n' || s[i] == b'\r' || s[i..].starts_with(b"?>") {
            break;
        }
        i += 1;
    }
    i
}

/// The length of a single-quoted string, quotes included.
fn single_quoted(s: &[u8]) -> Option<usize> {
    let mut i = 1;
    while i < s.len() {
        match s[i] {
            b'\\' => i += 2,
            b'\'' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// The length of a double-quoted string that interpolates nothing, quotes included.
fn double_quoted_plain(s: &[u8]) -> Option<usize> {
    let mut i = 1;
    while i < s.len() {
        match s[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            b'$' if s.get(i + 1).is_some_and(|&c| ident_start(c) || c == b'{') => return None,
            b'{' if s.get(i + 1) == Some(&b'$') => return None,
            _ => i += 1,
        }
    }
    None
}

/// The length of text inside a string, up to an interpolation or a stop.
fn literal_run(s: &[u8], stop: impl Fn(&[u8]) -> bool) -> usize {
    let mut i = 0;
    while i < s.len() {
        let r = &s[i..];
        if stop(r) {
            break;
        }
        match r[0] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'$' if r.len() > 1 && (ident_start(r[1]) || r[1] == b'{') => break,
            b'{' if r.get(1) == Some(&b'$') => break,
            _ => {}
        }
        i += 1;
    }
    i.min(s.len())
}

/// `<<<LABEL`, `<<<"LABEL"` or `<<<'LABEL'` and the newline after it.
/// Returns the token's length, where the label lies in it, and whether it is a nowdoc.
fn heredoc_start(s: &[u8]) -> Option<(usize, usize, usize, bool)> {
    let mut i = 3 + blanks(&s[3..]);
    let quote = match s.get(i) {
        Some(b'\'') => Some(b'\''),
        Some(b'"') => Some(b'"'),
        _ => None,
    };
    if quote.is_some() {
        i += 1;
    }
    let n = ident(&s[i..]);
    if n == 0 {
        return None;
    }
    let (ls, le) = (i, i + n);
    i += n;
    if let Some(q) = quote {
        if s.get(i) != Some(&q) {
            return None;
        }
        i += 1;
    }
    if s[i..].starts_with(b"\r\n") {
        i += 2;
    } else if s.get(i) == Some(&b'\n') {
        i += 1;
    } else {
        return None;
    }
    Some((i, ls, le, quote == Some(b'\'')))
}

/// At the start of a line: spaces or tabs, the label, and no name byte after it.
fn closing_label(s: &[u8], label: &[u8]) -> Option<usize> {
    let indent = blanks(s);
    let after = &s[indent..];
    if after.starts_with(label) && !after.get(label.len()).is_some_and(|&b| ident_byte(b)) {
        Some(indent + label.len())
    } else {
        None
    }
}

/// `\Name\Name…` after a name, or after a leading `\`.
fn qualified_tail(s: &[u8]) -> usize {
    let mut i = 0;
    while s.get(i) == Some(&b'\\') {
        let n = ident(&s[i + 1..]);
        if n == 0 {
            break;
        }
        i += 1 + n;
    }
    i
}

/// A number literal: an integer that fits in 64 bits, or else a float.
fn number(s: &[u8]) -> (usize, Kind) {
    let digits_of = |s: &[u8], ok: fn(u8) -> bool| -> usize {
        let mut i = 0;
        while i < s.len()
            && (ok(s[i]) || (s[i] == b'_' && i > 0 && s.get(i + 1).is_some_and(|&c| ok(c))))
        {
            i += 1;
        }
        i
    };
    let prefixed = |s: &[u8], radix: u32, ok: fn(u8) -> bool| -> (usize, Kind) {
        let n = digits_of(&s[2..], ok);
        if n == 0 {
            return (1, Kind::LNumber);
        }
        let clean: Vec<u8> = s[2..2 + n].iter().copied().filter(|&b| b != b'_').collect();
        let fits = i64::from_str_radix(std::str::from_utf8(&clean).unwrap_or("x"), radix).is_ok();
        (2 + n, if fits { Kind::LNumber } else { Kind::DNumber })
    };
    if s.len() > 2 && s[0] == b'0' {
        match s[1] {
            b'x' | b'X' => return prefixed(s, 16, |b| b.is_ascii_hexdigit()),
            b'b' | b'B' => return prefixed(s, 2, |b| b == b'0' || b == b'1'),
            b'o' | b'O' => return prefixed(s, 8, |b| (b'0'..=b'7').contains(&b)),
            _ => {}
        }
    }
    let int = digits_of(s, |b| b.is_ascii_digit());
    let mut i = int;
    let mut float = false;
    if s.get(i) == Some(&b'.') {
        let frac = digits_of(&s[i + 1..], |b| b.is_ascii_digit());
        if int > 0 || frac > 0 {
            float = true;
            i += 1 + frac;
        }
    }
    if matches!(s.get(i), Some(b'e') | Some(b'E')) {
        let mut j = i + 1;
        if matches!(s.get(j), Some(b'+') | Some(b'-')) {
            j += 1;
        }
        let exp = digits_of(&s[j..], |b| b.is_ascii_digit());
        if exp > 0 {
            float = true;
            i = j + exp;
        }
    }
    if float {
        return (i, Kind::DNumber);
    }
    let clean: Vec<u8> = s[..int].iter().copied().filter(|&b| b != b'_').collect();
    let text = std::str::from_utf8(&clean).unwrap_or("x");
    let fits = if clean.len() > 1 && clean[0] == b'0' {
        i64::from_str_radix(&text[1..], 8).is_ok()
    } else {
        text.parse::<i64>().is_ok()
    };
    (int, if fits { Kind::LNumber } else { Kind::DNumber })
}

/// `( int )` and the other casts.
fn cast(s: &[u8]) -> Option<(usize, Cast)> {
    let mut i = 1 + blanks(&s[1..]);
    let n = ident(&s[i..]);
    let word = &s[i..i + n];
    let c = Cast::ALL
        .iter()
        .find(|c| word.eq_ignore_ascii_case(c.text().as_bytes()))?;
    i += n;
    i += blanks(&s[i..]);
    (s.get(i) == Some(&b')')).then_some((i + 1, *c))
}

/// A word in code: a keyword, a magic constant, or a name.
fn word_kind(word: &[u8]) -> Kind {
    if let Some(k) = Kw::ALL
        .iter()
        .find(|k| word.eq_ignore_ascii_case(k.text().as_bytes()))
    {
        return Kind::Keyword(*k);
    }
    if let Some(m) = Magic::ALL
        .iter()
        .find(|m| word.eq_ignore_ascii_case(m.text().as_bytes()))
    {
        return Kind::Magic(*m);
    }
    Kind::String
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<String> {
        let src = src.as_bytes();
        lex(src)
            .unwrap()
            .iter()
            .map(|t| {
                format!(
                    "{} {:?}",
                    t.kind.name(),
                    String::from_utf8_lossy(t.text(src))
                )
            })
            .collect()
    }

    #[test]
    fn every_byte_lies_in_one_token() {
        let src = b"<p>a</p>\n<?php echo \"x $y {$z['k']}\", 'q'; ?>\nb<?= 1 ?>";
        let toks = lex(src).unwrap();
        let mut at = 0;
        for t in &toks {
            assert_eq!(t.start as usize, at);
            at = t.end as usize;
        }
        assert_eq!(at, src.len());
    }

    #[test]
    fn the_close_tag_takes_one_newline() {
        assert_eq!(
            kinds("<?php ?>\n\nx"),
            [
                "T_OPEN_TAG \"<?php \"",
                "T_CLOSE_TAG \"?>\\n\"",
                "T_INLINE_HTML \"\\nx\""
            ]
        );
    }

    #[test]
    fn a_line_comment_stops_at_the_close_tag() {
        let k = kinds("<?php // c ?> h");
        assert_eq!(k[1], "T_COMMENT \"// c \"");
        assert_eq!(k[2], "T_CLOSE_TAG \"?>\"");
    }

    #[test]
    fn a_plain_double_quoted_string_is_one_token() {
        assert_eq!(
            kinds("<?php \"a\\$b\"")[1],
            "T_CONSTANT_ENCAPSED_STRING \"\\\"a\\\\$b\\\"\""
        );
    }

    #[test]
    fn interpolation_is_split() {
        let k = kinds("<?php \"a $b[k] {$c}\"");
        assert_eq!(
            &k[1..],
            [
                "\" \"\\\"\"",
                "T_ENCAPSED_AND_WHITESPACE \"a \"",
                "T_VARIABLE \"$b\"",
                "[ \"[\"",
                "T_STRING \"k\"",
                "] \"]\"",
                "T_ENCAPSED_AND_WHITESPACE \" \"",
                "T_CURLY_OPEN \"{\"",
                "T_VARIABLE \"$c\"",
                "} \"}\"",
                "\" \"\\\"\"",
            ]
        );
    }

    #[test]
    fn an_integer_too_big_is_a_float() {
        assert_eq!(
            kinds("<?php 9223372036854775807")[1],
            "T_LNUMBER \"9223372036854775807\""
        );
        assert_eq!(
            kinds("<?php 9223372036854775808")[1],
            "T_DNUMBER \"9223372036854775808\""
        );
    }
}
