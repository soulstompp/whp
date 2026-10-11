// Copyright (C) 2026 Kenneth Allen Flegal and DappleAI
// SPDX-License-Identifier: GPL-3.0-or-later

//! The functions PHP gives every page, as far as whp runs them. A call outside what a function
//! takes here is refused, never answered differently.

use std::collections::HashSet;
use std::sync::LazyLock;

use bumpalo::Bump;
use bumpalo::collections::Vec as BVec;

use crate::Refusal;
use crate::value::{
    Array, Numeric, Value, identical, loose_equal, numeric, string_fits, to_bytes, truthy,
};

/// The functions PHP has, by lowercased name, as PHP 8.5.11 lists them (`functions.txt`).
static PHP_FUNCTIONS: LazyLock<HashSet<&'static [u8]>> = LazyLock::new(|| {
    include_str!("functions.txt")
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(str::as_bytes)
        .collect()
});

/// Whether PHP has a function by this lowercased name, which a page may not declare again.
pub fn php_has(name: &[u8]) -> bool {
    PHP_FUNCTIONS.contains(name)
}

pub type Builtin = for<'r> fn(&'r Bump, &[Value<'r>], u32) -> Result<Value<'r>, Refusal>;

/// The function PHP calls by this lowercased name, if whp runs it.
pub fn find(name: &[u8]) -> Option<Builtin> {
    Some(match name {
        b"count" => count,
        b"strlen" => strlen,
        b"implode" => implode,
        b"str_repeat" => str_repeat,
        b"htmlspecialchars" => htmlspecialchars,
        b"array_keys" => array_keys,
        b"in_array" => in_array,
        b"is_int" => is_int,
        b"is_string" => is_string,
        b"intdiv" => intdiv,
        _ => return None,
    })
}

/// Whether PHP has a function by this lowercased name that waits, which the request hands to
/// its host instead of running.
pub fn waits(name: &[u8]) -> bool {
    matches!(name, b"sleep" | b"usleep")
}

fn arity(args: &[Value<'_>], min: usize, max: usize, name: &str, line: u32) -> Result<(), Refusal> {
    if args.len() < min || args.len() > max {
        return Err(Refusal::at(
            format!("{name}() with {} arguments", args.len()),
            line,
        ));
    }
    Ok(())
}

/// A string parameter, as a page outside strict types passes one.
fn string_arg<'r>(bump: &'r Bump, v: Value<'r>, line: u32) -> Result<&'r [u8], Refusal> {
    match v {
        Value::Null => Err(Refusal::at("null passed where a string is wanted", line)),
        Value::Array(_) => Err(Refusal::at(
            "an array passed where a string is wanted",
            line,
        )),
        v => to_bytes(bump, v, line),
    }
}

fn int_arg(v: Value<'_>, line: u32) -> Result<i64, Refusal> {
    match v {
        Value::Int(i) => Ok(i),
        Value::Bool(b) => Ok(b as i64),
        Value::Str(s) => match numeric(s) {
            Numeric::Int(i) => Ok(i),
            _ => Err(Refusal::at("a string passed where an int is wanted", line)),
        },
        Value::Null => Err(Refusal::at("null passed where an int is wanted", line)),
        Value::Array(_) => Err(Refusal::at("an array passed where an int is wanted", line)),
    }
}

fn array_arg<'r>(v: Value<'r>, name: &str, line: u32) -> Result<&'r Array<'r>, Refusal> {
    match v {
        Value::Array(a) => Ok(a),
        _ => Err(Refusal::at(
            format!("{name}() given something other than an array"),
            line,
        )),
    }
}

fn count<'r>(_: &'r Bump, args: &[Value<'r>], line: u32) -> Result<Value<'r>, Refusal> {
    arity(args, 1, 1, "count", line)?;
    Ok(Value::Int(array_arg(args[0], "count", line)?.len() as i64))
}

fn strlen<'r>(bump: &'r Bump, args: &[Value<'r>], line: u32) -> Result<Value<'r>, Refusal> {
    arity(args, 1, 1, "strlen", line)?;
    Ok(Value::Int(string_arg(bump, args[0], line)?.len() as i64))
}

fn implode<'r>(bump: &'r Bump, args: &[Value<'r>], line: u32) -> Result<Value<'r>, Refusal> {
    arity(args, 1, 2, "implode", line)?;
    let (sep, items): (&[u8], _) = match args {
        [a] => (b"", array_arg(*a, "implode", line)?),
        [s, a] => (string_arg(bump, *s, line)?, array_arg(*a, "implode", line)?),
        _ => unreachable!(),
    };
    let mut out = Vec::new();
    for (n, (_, v)) in items.entries().into_iter().enumerate() {
        if n > 0 {
            out.extend_from_slice(sep);
        }
        out.extend_from_slice(to_bytes(bump, v, line)?);
        string_fits(out.len(), line)?;
    }
    Ok(Value::Str(bump.alloc_slice_copy(&out)))
}

fn str_repeat<'r>(bump: &'r Bump, args: &[Value<'r>], line: u32) -> Result<Value<'r>, Refusal> {
    arity(args, 2, 2, "str_repeat", line)?;
    let s = string_arg(bump, args[0], line)?;
    let n = int_arg(args[1], line)?;
    if n < 0 {
        return Err(Refusal::at("str_repeat() with a negative count", line));
    }
    if s.is_empty() || n == 0 {
        return Ok(Value::Str(b""));
    }
    let len = usize::try_from(n)
        .ok()
        .and_then(|n| s.len().checked_mul(n))
        .unwrap_or(usize::MAX);
    string_fits(len, line)?;
    let mut out = BVec::with_capacity_in(len, bump);
    for _ in 0..n {
        out.extend_from_slice(s);
    }
    Ok(Value::Str(out.into_bump_slice()))
}

/// How many bytes the character at the start of `s` takes, as PHP reads UTF-8, and whether they
/// are a character. Its lead byte says how many bytes it should have; it takes each byte that
/// follows from 0x80 up, to that many, and stops before a byte below 0x80.
fn utf8_at(s: &[u8]) -> (usize, bool) {
    let len = match s[0] {
        0x00..=0x7f => return (1, true),
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return (1, false),
    };
    let taken = 1 + s[1..]
        .iter()
        .take(len - 1)
        .take_while(|&&b| b >= 0x80)
        .count();
    if taken < len {
        return (taken, false);
    }
    (len, std::str::from_utf8(&s[..len]).is_ok())
}

/// `&`, `"`, `'`, `<` and `>` as entities, and each sequence of bytes that is not a character as
/// U+FFFD: PHP 8's default flags, with UTF-8 as the charset.
fn htmlspecialchars<'r>(
    bump: &'r Bump,
    args: &[Value<'r>],
    line: u32,
) -> Result<Value<'r>, Refusal> {
    arity(args, 1, 1, "htmlspecialchars", line)?;
    let s = string_arg(bump, args[0], line)?;
    let mut out = Vec::with_capacity(s.len());
    let mut at = 0;
    while at < s.len() {
        let (n, character) = utf8_at(&s[at..]);
        match (character, s[at]) {
            (false, _) => out.extend_from_slice("\u{FFFD}".as_bytes()),
            (true, b'&') => out.extend_from_slice(b"&amp;"),
            (true, b'"') => out.extend_from_slice(b"&quot;"),
            (true, b'\'') => out.extend_from_slice(b"&#039;"),
            (true, b'<') => out.extend_from_slice(b"&lt;"),
            (true, b'>') => out.extend_from_slice(b"&gt;"),
            (true, _) => out.extend_from_slice(&s[at..at + n]),
        }
        string_fits(out.len(), line)?;
        at += n;
    }
    Ok(Value::Str(bump.alloc_slice_copy(&out)))
}
fn array_keys<'r>(bump: &'r Bump, args: &[Value<'r>], line: u32) -> Result<Value<'r>, Refusal> {
    arity(args, 1, 1, "array_keys", line)?;
    let a = array_arg(args[0], "array_keys", line)?;
    let keys = Array::new(bump);
    for (k, _) in a.entries() {
        keys.push(k.value(), line)?;
    }
    Ok(Value::Array(keys))
}

fn in_array<'r>(_: &'r Bump, args: &[Value<'r>], line: u32) -> Result<Value<'r>, Refusal> {
    arity(args, 2, 3, "in_array", line)?;
    let a = array_arg(args[1], "in_array", line)?;
    let strict = match args.get(2) {
        None => false,
        Some(Value::Null) => return Err(Refusal::at("null passed where a bool is wanted", line)),
        Some(Value::Array(_)) => {
            return Err(Refusal::at("an array passed where a bool is wanted", line));
        }
        Some(v) => truthy(*v),
    };
    for (_, v) in a.entries() {
        let found = if strict {
            identical(args[0], v)
        } else {
            loose_equal(args[0], v, line)?
        };
        if found {
            return Ok(Value::Bool(true));
        }
    }
    Ok(Value::Bool(false))
}

fn is_int<'r>(_: &'r Bump, args: &[Value<'r>], line: u32) -> Result<Value<'r>, Refusal> {
    arity(args, 1, 1, "is_int", line)?;
    Ok(Value::Bool(matches!(args[0], Value::Int(_))))
}

fn is_string<'r>(_: &'r Bump, args: &[Value<'r>], line: u32) -> Result<Value<'r>, Refusal> {
    arity(args, 1, 1, "is_string", line)?;
    Ok(Value::Bool(matches!(args[0], Value::Str(_))))
}

fn intdiv<'r>(_: &'r Bump, args: &[Value<'r>], line: u32) -> Result<Value<'r>, Refusal> {
    arity(args, 2, 2, "intdiv", line)?;
    let (a, b) = (int_arg(args[0], line)?, int_arg(args[1], line)?);
    if b == 0 {
        return Err(Refusal::at("division by zero", line));
    }
    a.checked_div(b)
        .map(Value::Int)
        .ok_or_else(|| Refusal::at("intdiv() past the int range", line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_and_substitutes() {
        let bump = Bump::new();
        let v = htmlspecialchars(&bump, &[Value::Str(b"<a href='x'>&\xff")], 0).unwrap();
        let Value::Str(s) = v else { panic!() };
        assert_eq!(s, "&lt;a href=&#039;x&#039;&gt;&amp;\u{FFFD}".as_bytes());
    }
}
