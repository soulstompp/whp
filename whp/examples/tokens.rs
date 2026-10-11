// Copyright (C) 2026 Kenneth Allen Flegal and DappleAI
// SPDX-License-Identifier: GPL-3.0-or-later

//! Prints a page's tokens, one JSON array a line: `[name, text, line]`.
//!
//!     cargo run --example tokens -- <page.php>

use std::io::Write;

fn main() {
    let path = std::env::args().nth(1).expect("a page");
    let src = std::fs::read(&path).expect("the page reads");
    let tokens = whp::lex::lex(&src).unwrap_or_else(|e| panic!("line {}: {}", e.line, e.what));
    let mut out = std::io::stdout().lock();
    for t in tokens {
        writeln!(
            out,
            "[{},{},{}]",
            json(t.kind.name().to_string().as_bytes()),
            json(t.text(&src)),
            t.line
        )
        .unwrap();
    }
}

/// A JSON string, as PHP's json_encode writes one: `/` escaped, and every byte above 0x7f as `\u` of
/// its UTF-8 character.
fn json(s: &[u8]) -> String {
    let text = String::from_utf8_lossy(s);
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '/' => out.push_str("\\/"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut buf = [0u16; 2];
                for u in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{:04x}", u));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
