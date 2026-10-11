// Copyright (C) 2026 Kenneth Allen Flegal and DappleAI
// SPDX-License-Identifier: GPL-3.0-or-later

//! Values, as a request holds them: everything a request makes lives in its blocks.
//!
//! An array is shared between its holders and copied before a holder writes to it, so each holder
//! sees an array of its own, as PHP's arrays behave. `refs` counts the holders: a variable, an
//! element of another array, a slot of the operand stack, or a loop walking it. A count too high
//! costs a copy and never an answer; a count too low would let one holder's write reach another.

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::fmt::Write as _;

use bumpalo::Bump;
use bumpalo::collections::Vec as BVec;
use hashbrown::{DefaultHashBuilder, HashMap};

use crate::Refusal;

/// PHP's default `memory_limit`, 128M, in bytes: PHP holds no string this long or longer under it.
pub const STRING_LIMIT: usize = 134_217_728;

/// The most elements one array holds under PHP 8.5's default `memory_limit`.
pub const ARRAY_LIMIT: usize = 4_194_304;

/// Refuses a string of `len` bytes, which PHP could not hold under its `memory_limit` either.
pub fn string_fits(len: usize, line: u32) -> Result<(), Refusal> {
    if len >= STRING_LIMIT {
        return Err(Refusal::at(
            "a string longer than PHP's memory_limit holds",
            line,
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub enum Value<'r> {
    Null,
    Bool(bool),
    Int(i64),
    Str(&'r [u8]),
    Array(&'r Array<'r>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key<'r> {
    Int(i64),
    Str(&'r [u8]),
}

impl<'r> Key<'r> {
    pub fn value(self) -> Value<'r> {
        match self {
            Key::Int(i) => Value::Int(i),
            Key::Str(s) => Value::Str(s),
        }
    }
}

pub struct Array<'r> {
    refs: Cell<u32>,
    map: RefCell<Map<'r>>,
}

impl std::fmt::Debug for Array<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Array(len {}, refs {})", self.len(), self.refs.get())
    }
}

struct Map<'r> {
    entries: BVec<'r, Option<(Key<'r>, Value<'r>)>>,
    index: HashMap<Key<'r>, usize, DefaultHashBuilder, &'r Bump>,
    len: usize,
    /// The key the next appended element takes; none until an int key has been used.
    next: Option<i64>,
}

impl<'r> Map<'r> {
    fn new(bump: &'r Bump) -> Self {
        Map {
            entries: BVec::new_in(bump),
            index: HashMap::with_hasher_in(DefaultHashBuilder::default(), bump),
            len: 0,
            next: None,
        }
    }
}

impl<'r> Array<'r> {
    /// A new, empty array with one holder: whoever made it.
    pub fn new(bump: &'r Bump) -> &'r Array<'r> {
        bump.alloc(Array {
            refs: Cell::new(1),
            map: RefCell::new(Map::new(bump)),
        })
    }

    pub fn len(&self) -> usize {
        self.map.borrow().len
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn refs(&self) -> u32 {
        self.refs.get()
    }

    pub fn get(&self, key: Key<'r>) -> Option<Value<'r>> {
        let m = self.map.borrow();
        m.index
            .get(&key)
            .and_then(|&i| m.entries[i].map(|(_, v)| v))
    }

    /// The entry at a position of the walk, live or not, and how many positions there are.
    pub fn at(&self, pos: usize) -> (Option<(Key<'r>, Value<'r>)>, usize) {
        let m = self.map.borrow();
        (m.entries.get(pos).copied().flatten(), m.entries.len())
    }

    /// Every live entry, in order.
    pub fn entries(&self) -> Vec<(Key<'r>, Value<'r>)> {
        self.map
            .borrow()
            .entries
            .iter()
            .flatten()
            .copied()
            .collect()
    }

    /// Sets `key` to `value`, which the array now holds; an old value is given back to release.
    /// A new key past the elements PHP holds in one array is refused.
    pub fn set(
        &self,
        key: Key<'r>,
        value: Value<'r>,
        line: u32,
    ) -> Result<Option<Value<'r>>, Refusal> {
        let mut m = self.map.borrow_mut();
        let found = m.index.get(&key).copied();
        if found.is_none() && m.len >= ARRAY_LIMIT {
            return Err(Refusal::at(
                "an array with more elements than PHP's memory_limit holds",
                line,
            ));
        }
        if let Key::Int(i) = key {
            m.next = Some(match m.next {
                Some(n) => n.max(i.saturating_add(1)),
                None => i.saturating_add(1),
            });
        }
        if let Some(pos) = found {
            let old = m.entries[pos].map(|(_, v)| v);
            m.entries[pos] = Some((key, value));
            return Ok(old);
        }
        let pos = m.entries.len();
        m.entries.push(Some((key, value)));
        m.index.insert(key, pos);
        m.len += 1;
        Ok(None)
    }

    /// Appends `value` under the next int key.
    pub fn push(&self, value: Value<'r>, line: u32) -> Result<(), Refusal> {
        let key = self.map.borrow().next.unwrap_or(0);
        if key == i64::MAX && self.get(Key::Int(key)).is_some() {
            return Err(Refusal::at("an array that has run out of int keys", line));
        }
        self.set(Key::Int(key), value, line)?;
        Ok(())
    }

    /// Removes `key`; its value is given back to release.
    pub fn remove(&self, key: Key<'r>) -> Option<Value<'r>> {
        let mut m = self.map.borrow_mut();
        let pos = m.index.remove(&key)?;
        let old = m.entries[pos].take().map(|(_, v)| v);
        m.len -= 1;
        old
    }

    /// A copy of the array, which shares every element value with this one.
    pub fn copy(&self, bump: &'r Bump) -> &'r Array<'r> {
        let new = Array::new(bump);
        {
            let src = self.map.borrow();
            let mut dst = new.map.borrow_mut();
            for (k, v) in src.entries.iter().flatten().copied() {
                share(v);
                let pos = dst.entries.len();
                dst.entries.push(Some((k, v)));
                dst.index.insert(k, pos);
            }
            dst.len = src.len;
            dst.next = src.next;
        }
        new
    }

    /// A copy of the array in `bump`, each entry passed through `f`. The copy takes over the
    /// array's holds on its elements.
    pub fn moved(
        &self,
        bump: &'r Bump,
        mut f: impl FnMut(Key<'r>, Value<'r>) -> (Key<'r>, Value<'r>),
    ) -> &'r Array<'r> {
        let new = Array::new(bump);
        {
            let src = self.map.borrow();
            let mut dst = new.map.borrow_mut();
            for (k, v) in src.entries.iter().flatten().copied() {
                let (k, v) = f(k, v);
                let pos = dst.entries.len();
                dst.entries.push(Some((k, v)));
                dst.index.insert(k, pos);
            }
            dst.len = src.len;
            dst.next = src.next;
        }
        new
    }

    /// Rewrites every entry in place through `f`, and the index with it.
    pub fn rewrite(&self, mut f: impl FnMut(Key<'r>, Value<'r>) -> (Key<'r>, Value<'r>)) {
        let mut m = self.map.borrow_mut();
        for e in m.entries.iter_mut().flatten() {
            *e = f(e.0, e.1);
        }
        let Map { entries, index, .. } = &mut *m;
        index.clear();
        for (pos, e) in entries.iter().enumerate() {
            if let Some((k, _)) = e {
                index.insert(*k, pos);
            }
        }
    }
}

/// One more holder of a value.
pub fn share(v: Value<'_>) {
    if let Value::Array(a) = v {
        a.refs.set(a.refs.get() + 1);
    }
}

/// One holder fewer. An array nobody holds lets go of its elements.
pub fn release(v: Value<'_>) {
    if let Value::Array(a) = v {
        let n = a.refs.get();
        debug_assert!(n > 0, "an array released more often than shared");
        a.refs.set(n.saturating_sub(1));
        if n == 1 {
            for (_, e) in a.map.borrow().entries.iter().flatten() {
                release(*e);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Conversions

pub fn truthy(v: Value<'_>) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => b,
        Value::Int(i) => i != 0,
        Value::Str(s) => !(s.is_empty() || s == b"0"),
        Value::Array(a) => !a.is_empty(),
    }
}

pub fn type_name(v: Value<'_>) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Str(_) => "string",
        Value::Array(_) => "array",
    }
}

/// A value written as text: what `echo` prints.
pub fn to_bytes<'r>(bump: &'r Bump, v: Value<'r>, line: u32) -> Result<&'r [u8], Refusal> {
    Ok(match v {
        Value::Null | Value::Bool(false) => b"",
        Value::Bool(true) => b"1",
        Value::Int(i) => int_bytes(bump, i),
        Value::Str(s) => s,
        Value::Array(_) => return Err(Refusal::at("an array written as text", line)),
    })
}

pub fn int_bytes(bump: &Bump, i: i64) -> &[u8] {
    let mut s = String::new();
    let _ = write!(s, "{i}");
    bump.alloc_slice_copy(s.as_bytes())
}

/// How a string reads as a number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Numeric {
    Int(i64),
    /// Numeric, but a float: whp does not hold floats yet.
    Float,
    /// A number followed by other text.
    Leading,
    No,
}

fn is_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// A numeric string, as the manual defines one: whitespace, a sign, digits with an optional
/// fraction and exponent, whitespace.
pub fn numeric(s: &[u8]) -> Numeric {
    let mut i = s.iter().take_while(|&&b| is_ws(b)).count();
    let start = i;
    if matches!(s.get(i), Some(b'+') | Some(b'-')) {
        i += 1;
    }
    let int_digits = s[i..].iter().take_while(|b| b.is_ascii_digit()).count();
    i += int_digits;
    let mut float = false;
    let mut frac_digits = 0;
    if s.get(i) == Some(&b'.') {
        frac_digits = s[i + 1..].iter().take_while(|b| b.is_ascii_digit()).count();
        if int_digits + frac_digits > 0 {
            float = true;
            i += 1 + frac_digits;
        }
    }
    if int_digits + frac_digits == 0 {
        return Numeric::No;
    }
    if matches!(s.get(i), Some(b'e') | Some(b'E')) {
        let mut j = i + 1;
        if matches!(s.get(j), Some(b'+') | Some(b'-')) {
            j += 1;
        }
        let exp = s[j..].iter().take_while(|b| b.is_ascii_digit()).count();
        if exp > 0 {
            float = true;
            i = j + exp;
        }
    }
    let number_end = i;
    i += s[i..].iter().take_while(|&&b| is_ws(b)).count();
    if i < s.len() {
        return Numeric::Leading;
    }
    if float {
        return Numeric::Float;
    }
    match std::str::from_utf8(&s[start..number_end])
        .ok()
        .and_then(|t| t.parse::<i64>().ok())
    {
        Some(n) => Numeric::Int(n),
        None => Numeric::Float,
    }
}

/// A value as an int operand of arithmetic.
pub fn to_int_operand(v: Value<'_>, line: u32) -> Result<i64, Refusal> {
    match v {
        Value::Null => Ok(0),
        Value::Bool(b) => Ok(b as i64),
        Value::Int(i) => Ok(i),
        Value::Str(s) => match numeric(s) {
            Numeric::Int(i) => Ok(i),
            Numeric::Float => Err(Refusal::at("floats", line)),
            Numeric::Leading => Err(Refusal::at("a string that only starts with a number", line)),
            Numeric::No => Err(Refusal::at("arithmetic on a non-numeric string", line)),
        },
        Value::Array(_) => Err(Refusal::at("arithmetic on an array", line)),
    }
}

/// A value used as an array key.
pub fn to_key<'r>(v: Value<'r>, line: u32) -> Result<Key<'r>, Refusal> {
    match v {
        Value::Int(i) => Ok(Key::Int(i)),
        Value::Bool(b) => Ok(Key::Int(b as i64)),
        Value::Str(s) => Ok(string_key(s)),
        Value::Null => Err(Refusal::at("null as an array key", line)),
        Value::Array(_) => Err(Refusal::at("an array as an array key", line)),
    }
}

/// A string key that is a whole decimal int, written the one way PHP writes it, becomes the int.
pub fn string_key(s: &[u8]) -> Key<'_> {
    let digits = s.strip_prefix(b"-").unwrap_or(s);
    let canonical = !digits.is_empty()
        && digits.iter().all(u8::is_ascii_digit)
        && (digits == b"0" && s.len() == 1 || digits[0] != b'0');
    if canonical
        && let Some(n) = std::str::from_utf8(s)
            .ok()
            .and_then(|t| t.parse::<i64>().ok())
    {
        return Key::Int(n);
    }
    Key::Str(s)
}

// ---------------------------------------------------------------------------------------------
// Comparison, as the manual's comparison table sets it out

/// `a <=> b`, or none where two arrays cannot be compared.
pub fn compare<'r>(a: Value<'r>, b: Value<'r>, line: u32) -> Result<Option<Ordering>, Refusal> {
    use Value::*;
    Ok(Some(match (a, b) {
        (Null, Str(s)) => compare_strings(b"", s, line)?,
        (Str(s), Null) => compare_strings(s, b"", line)?,
        (Bool(_) | Null, _) | (_, Bool(_) | Null) => truthy(a).cmp(&truthy(b)),
        (Int(x), Int(y)) => x.cmp(&y),
        (Int(x), Str(s)) => match numeric(s) {
            Numeric::Int(y) => x.cmp(&y),
            Numeric::Float => return Err(Refusal::at("comparing with a float", line)),
            _ => x.to_string().as_bytes().cmp(s),
        },
        (Str(s), Int(y)) => match numeric(s) {
            Numeric::Int(x) => x.cmp(&y),
            Numeric::Float => return Err(Refusal::at("comparing with a float", line)),
            _ => s.cmp(y.to_string().as_bytes()),
        },
        (Str(x), Str(y)) => compare_strings(x, y, line)?,
        (Array(x), Array(y)) => return compare_arrays(x, y, line),
        (Array(_), _) => Ordering::Greater,
        (_, Array(_)) => Ordering::Less,
    }))
}

fn compare_strings(x: &[u8], y: &[u8], line: u32) -> Result<Ordering, Refusal> {
    match (numeric(x), numeric(y)) {
        (Numeric::Int(a), Numeric::Int(b)) => Ok(a.cmp(&b)),
        (Numeric::Int(_) | Numeric::Float, Numeric::Int(_) | Numeric::Float) => {
            Err(Refusal::at("comparing numeric strings as floats", line))
        }
        _ => Ok(x.cmp(y)),
    }
}

fn compare_arrays<'r>(
    x: &Array<'r>,
    y: &Array<'r>,
    line: u32,
) -> Result<Option<Ordering>, Refusal> {
    match x.len().cmp(&y.len()) {
        Ordering::Equal => {}
        o => return Ok(Some(o)),
    }
    for (k, v) in x.entries() {
        let Some(w) = y.get(k) else { return Ok(None) };
        match compare(v, w, line)? {
            Some(Ordering::Equal) => {}
            other => return Ok(other),
        }
    }
    Ok(Some(Ordering::Equal))
}

/// `a == b`.
pub fn loose_equal<'r>(a: Value<'r>, b: Value<'r>, line: u32) -> Result<bool, Refusal> {
    Ok(compare(a, b, line)? == Some(Ordering::Equal))
}

/// `a === b`: the same type and the same value; arrays hold the same pairs in the same order.
pub fn identical<'r>(a: Value<'r>, b: Value<'r>) -> bool {
    use Value::*;
    match (a, b) {
        (Null, Null) => true,
        (Bool(x), Bool(y)) => x == y,
        (Int(x), Int(y)) => x == y,
        (Str(x), Str(y)) => x == y,
        (Array(x), Array(y)) => {
            let (ex, ey) = (x.entries(), y.entries());
            ex.len() == ey.len()
                && ex
                    .iter()
                    .zip(&ey)
                    .all(|((kx, vx), (ky, vy))| kx == ky && identical(*vx, *vy))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_strings() {
        assert_eq!(numeric(b" 7"), Numeric::Int(7));
        assert_eq!(numeric(b"7 "), Numeric::Int(7));
        assert_eq!(numeric(b"1e1"), Numeric::Float);
        assert_eq!(numeric(b"9a"), Numeric::Leading);
        assert_eq!(numeric(b"abc"), Numeric::No);
        assert_eq!(numeric(b""), Numeric::No);
    }

    #[test]
    fn only_canonical_int_strings_become_int_keys() {
        assert_eq!(string_key(b"8"), Key::Int(8));
        assert_eq!(string_key(b"-5"), Key::Int(-5));
        assert_eq!(string_key(b"0"), Key::Int(0));
        assert_eq!(string_key(b"08"), Key::Str(b"08"));
        assert_eq!(string_key(b"-0"), Key::Str(b"-0"));
        assert_eq!(string_key(b""), Key::Str(b""));
    }

    #[test]
    fn a_copy_is_written_apart() {
        let bump = Bump::new();
        let a = Array::new(&bump);
        a.push(Value::Int(1), 0).unwrap();
        let b = a.copy(&bump);
        b.push(Value::Int(2), 0).unwrap();
        assert_eq!((a.len(), b.len()), (1, 2));
    }

    #[test]
    fn the_next_key_follows_the_largest_int_key() {
        let bump = Bump::new();
        let a = Array::new(&bump);
        a.set(Key::Int(-5), Value::Null, 0).unwrap();
        a.push(Value::Null, 0).unwrap();
        assert!(a.get(Key::Int(-4)).is_some());
    }
}
