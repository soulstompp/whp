# whp

whp, the warren home page, runs PHP 8 templates as requests, in Rust, on the tokio stack.

It does what PHP has always done best. A request comes in, a page runs from top to bottom, the
page goes out, and everything the request made is thrown away. Every page whp runs gives PHP's own
bytes. A page that uses something whp does not run yet is refused, with what whp met and the line
it met it on; whp never runs a page differently.

What changes is for Rust developers. A request in whp is an iterator over its waits: it runs until
it must sleep or load a page, hands that wait back, and goes on when it is resumed. whp does no
I/O of its own, so the timer, the loader, the store and the runtime are whichever Rust libraries
suit you. The `whp` command is one such host, on the tokio stack, where a request that waits gives
its thread to other requests.

## Install

```sh
cargo install whp-tokio
```

This installs the `whp` command. It needs Rust 1.88 or newer on a Unix system (it is built and
tested on Linux), and nothing else: no PHP, no web server, no configuration. `cargo uninstall
whp-tokio` takes it out again. From a checkout of this repository, `cargo install --path
whp-tokio` does the same.

## A first page

Put this in `hello.php`:

```php
<?php $name = 'world'; ?>
<p>Hello, <?= htmlspecialchars($name) ?>!</p>
```

Run it as one request:

```console
$ whp run hello.php
<p>Hello, world!</p>
```

Or serve the directory it is in, and ask for it over HTTP:

```console
$ whp serve .
whp: serving /home/you/site at http://127.0.0.1:8080
$ curl http://127.0.0.1:8080/hello.php
<p>Hello, world!</p>
```

A page that needs something whp does not run yet is refused. This one divides 7 by 2, and the
answer, 3.5, is a float:

```console
$ whp run share.php
whp refuses a division with a remainder, a float, at /home/you/site/share.php:4
$ echo $?
3
```

The refusal goes to stderr. Whatever the page printed before it is what PHP printed too: whp
stops where its page would have differed from PHP's.

## What a page can use

whp runs this much of PHP today, and refuses the rest:

- HTML outside the tags, and `<?php`, `<?=` and `<?`.
- `echo`, `print`, and `.` to join strings.
- `null`, booleans, integers, and strings: single- and double-quoted, with their escapes and
  interpolation (`"$a"`, `"$a[k]"`, `"{$a['k']}"`), heredocs and nowdocs, and string offsets.
- Integer arithmetic: `+`, `-`, `*`, `%`, `**`, `intdiv()`, and `/` where the answer is whole.
- Comparisons by PHP 8's rules for numeric strings: `==`, `!=`, `===`, `!==`, `<`, `<=`, `>`,
  `>=` and `<=>`. Logic: `!`, `&&`, `||`, `and`, `or`, `xor`, the ternary, `?:` and `??`.
- Arrays: ordered maps with integer and string keys, `[]` to append, nesting, copied on
  assignment, and `foreach` over keys and values.
- Assignment, the compound assignments `+=`, `-=`, `*=`, `/=`, `%=`, `**=` and `.=` on a
  variable, and `++` and `--`.
- `if`, `elseif` and `else`, `while`, `do`, `for`, `break` and `continue` (with a count),
  `match`, and the alternative syntax (`if: … endif;` and the rest).
- Functions: parameters with defaults, `return`, and recursion.
- `include`, `require` and their `_once` forms, sharing the caller's variables and giving back the
  included page's `return`.
- `isset`, `empty` and `unset`.
- The constants `true`, `false`, `null`, `PHP_EOL`, `PHP_INT_MAX`, `PHP_INT_MIN`,
  `PHP_INT_SIZE`, `__DIR__`, `__FILE__` and `__LINE__`.
- The functions `count`, `strlen`, `implode`, `str_repeat`, `htmlspecialchars` (with PHP 8's
  default flags), `array_keys`, `in_array`, `is_int`, `is_string`, `intdiv`, `sleep` and
  `usleep`.

Refused today: floats, classes and objects, closures and arrow functions, generators, references,
exceptions, `switch`, namespaces and `use`, `static` and `global`, casts, bitwise operators, the
pipe operator, type declarations, the variables PHP itself defines (`$_GET`, `$_SERVER`, `$argv`
and the rest), other constants, and any function whp does not have. So is anything PHP warns
about, deprecates or rejects, such as reading a variable or a key that is not set, or a ternary
inside a ternary without parentheses. whp prints no warnings: where PHP would print one, whp
refuses the page.

## One request, from start to end

- **A page is compiled once.** The first request that asks for a page reads and compiles it, and
  whp keeps it for as long as the process runs. Every later request runs the kept page and reads
  nothing from the disk, so a page whose file changes is read again only when whp starts again.
- **A request gives its thread back while it waits.** At `sleep()`, at `usleep()`, and at an
  include of a page not yet kept, the request stops and hands its thread back to the tokio stack,
  which runs other requests meanwhile. The page so far goes out at each wait.
- **Nothing a request made outlives it.** What a request makes is let go all at once when its
  page has gone out, and what a call makes, when the call returns, so requests share nothing but
  the kept pages. A string built with `.=` grows in place, and stays near its own length.
- **whp stops where PHP's memory would.** Under its default `memory_limit` of 128M, PHP cannot
  hold a string of 128 MiB or more, an array of more than 4,194,304 elements, or 915,135 calls at
  once. whp refuses a page that reaches one of these, and goes on serving the others.

## Serving

`whp serve <dir> [--addr 127.0.0.1:8080]` serves the pages under `<dir>` over HTTP/1.1, each HTTP
request one request.

- A path that ends in `.php` and names a file under `<dir>` gets 200 and the page, as
  `text/html; charset=UTF-8`. Anything else gets 404 with an empty body: other files, `/`, a
  missing page, and any path with a `..` segment. The path is taken as written, and the query
  string is not read.
- The page goes out as it is written, a part at every wait. A page refused partway breaks its
  response, so a client never takes the page so far for the whole page.
- Includes are found as PHP finds them, from `<dir>`.

## A request is an iterator over its waits

In PHP a request runs to its end inside the engine, which blocks at every `sleep()` and at every
page it loads. In whp you hold the request. The `whp` crate, the library under the command,
compiles pages and runs requests, and does no I/O of its own:

- a `Store` keeps the compiled pages: in a map, on a disk, behind a cache, wherever you keep
  them;
- `Request::resume` runs the request until it ends or must wait, writing its page to any
  `std::io::Write`, and gives back a `Step`: `Done`, or `Wait` with what it waits on;
- for a `Wait::Sleep`, you sleep, with any timer; for a `Wait::Page`, you load the page, with any
  loader, hand it over with `Request::loaded`, and resume; a page that cannot be loaded is refused
  at the include that asked for it, by `Request::failed`;
- `Step::Done` carries an `Outcome`: the bytes of the page, of its blocks, and its calls; a page
  whp will not run gives a `Refusal`, with what whp met, the page and the line;
- the request owns everything it made, so it moves between threads whole while it waits.

So any part of the host can be replaced with a better Rust library, and the page still gives
PHP's bytes. A host that does the waits in the simplest way:

```rust
use whp::{Page, Refusal, Request, Step, Store, Wait, compile_page};

/// A store holding one page, kept from the start.
struct One(&'static Page);

impl Store for One {
    fn kept(&self, _path: &[u8], _from_dir: &[u8]) -> Option<&'static Page> {
        Some(self.0)
    }
    fn load(&self, _path: &[u8], _from_dir: &[u8]) -> Result<&'static Page, Refusal> {
        Ok(self.0)
    }
}

let source = b"<p><?= 'Hello, ' . 'world' ?></p>\n";
let page: &'static Page = Box::leak(Box::new(compile_page(b"/hello.php", source).unwrap()));
let store = One(page);
let mut out = Vec::new();
let mut request = Request::new(&store, page);
loop {
    match request.resume(&mut out).unwrap() {
        Step::Done(_) => break,
        Step::Wait(Wait::Sleep(time)) => std::thread::sleep(time),
        Step::Wait(Wait::Page { path, from_dir }) => {
            let loaded = store.load(&path, &from_dir).map_err(|r| request.failed(r));
            request.loaded(loaded.unwrap());
        }
    }
}
assert_eq!(out, b"<p>Hello, world</p>\n");
```

`whp::run` drives one request to its end on the calling thread. The `whp-tokio` crate is the host
on the tokio stack, and the source of the `whp` command: each wait is an await there, so a
request that waits gives its thread to other requests.

## Next: iterators and plugins in Rust

This part is not in whp yet: it is the design whp is built toward, said here so that what is
built today is read for what it leads to.

- **Iterators, with no new syntax.** whp adds functions in its own `Whp\` namespace, never new
  keywords or globals, so every page the latest PHP 8 runs still runs unchanged. Each one, such as
  `map`, `filter` and `take`, takes its options and gives back a callable for PHP 8.5's pipe
  operator, and that callable walks any iterable lazily, as a generator:
  `$orders |> filter(fn($o) => $o->paid) |> map(fn($o) => $o->total) |> take(10)`.
- **In Rust under whp, in plain PHP elsewhere.** In whp these functions run in Rust. The same
  functions also exist in plain PHP, so a page that uses them runs on stock PHP too, only slower,
  and is never locked to whp.
- **One `use` brings them in.** `use function Whp\{array_map, array_filter, map, filter, take};`
  puts them in place of the old functions for that one file. Where one takes an old name, it gives
  the old answer wherever the old one gave one (an array in, the same array out), and is lazy only
  where the old one refused, such as a generator in.
- **Plugins are Rust crates.** A plugin's functions are called from a page like any built-in, so
  the heavy work moves into Rust and the page stays a template. A plugin is built into the binary,
  with no C between them.

These need closures, generators and the pipe operator, which whp refuses today, and they come
when those do.

## Reference

### The command

| Command | What it does |
|---|---|
| `whp run <page.php>` | runs the page as one request and writes it to stdout |
| `whp serve <dir> [--addr <addr>]` | serves `<dir>`'s pages over HTTP/1.1, at `127.0.0.1:8080` unless `--addr` says otherwise |

`whp` exits 0 when the page went out whole, 3 when whp refused it, and 2 when it could not start:
a wrong argument, or a directory it cannot serve.

### Features

- **`round`** counts what each request does. `whp run` gains `--repeat N` (N requests, one after
  another), `--concurrent N` (then N at once, each giving the same page) and `--workers W` (the
  stack's worker threads); `whp serve` gains `--workers W`; and both take `--round <file.jsonl>`,
  one JSON line a request: how often the stack polled it, its waits, the files it read, its heap
  at the peak, the bytes of its blocks, its calls, and its page's bytes.
- **`poison`** overwrites the memory of a call that has returned, and of a string that has moved,
  and keeps it, so a page that read either one afterwards would come out wrong. It is for testing.

### Tests

`cargo test` runs two corpora of pages, each page beside the output PHP 8.5.11 gave for it, in the
official `php:8.5-cli` image (its digest is in `tests/pages/ORACLE`):

- `tests/pages`: every page gives PHP's bytes;
- `tests/refuse`: pages at the edges of what whp runs, each giving PHP's bytes, or refused at the
  line it marks with `// refused here`.

### Compatibility

whp follows the latest PHP 8, now 8.5.11, under PHP's built-in defaults with no `php.ini`: among
them `short_open_tag` on, so `<?` opens PHP, and `default_charset` UTF-8.

## License

GPL-3.0-or-later. Copyright © 2026 Kenneth Allen Flegal and DappleAI.
