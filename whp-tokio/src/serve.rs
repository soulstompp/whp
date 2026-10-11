// Copyright (C) 2026 Kenneth Allen Flegal and DappleAI
// SPDX-License-Identifier: GPL-3.0-or-later

//! `whp serve <dir>`: the pages under a directory, over HTTP/1.1. One HTTP request is one
//! request, on a task of its own. Its page goes out as it is written, a frame at every wait, as
//! PHP's own server sends a page.

use std::convert::Infallible;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};

use hyper::body::{Bytes, Frame, Incoming};
use hyper::header::CONTENT_TYPE;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use whp::{Page, Refusal, Store};

use crate::host;
use crate::store::FileStore;

/// What a page is sent as: PHP's default type and charset.
const HTML: &str = "text/html; charset=UTF-8";

/// A response's body: the frames its request sends, in order. It ends when the request ends, and
/// breaks where the request was refused.
pub struct Body {
    frames: UnboundedReceiver<Result<Bytes, Refusal>>,
}

impl Body {
    fn empty() -> Self {
        let (_, frames) = unbounded_channel();
        Body { frames }
    }
}

impl hyper::body::Body for Body {
    type Data = Bytes;
    type Error = Refusal;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Refusal>>> {
        self.frames
            .poll_recv(cx)
            .map(|frame| frame.map(|r| r.map(Frame::data)))
    }
}

/// Where a request writes its page: what it wrote before a wait is one frame, sent at the wait.
/// Sending never makes the request wait. A body its request did not end is broken, so a client
/// never takes the page so far for the whole page.
struct Frames {
    pending: Vec<u8>,
    to: UnboundedSender<Result<Bytes, Refusal>>,
    ended: bool,
}

impl Frames {
    fn new(to: UnboundedSender<Result<Bytes, Refusal>>) -> Self {
        Frames {
            pending: Vec::new(),
            to,
            ended: false,
        }
    }

    /// The request ended and its page has gone out whole.
    fn end(&mut self) {
        self.ended = true;
    }

    /// Breaks the body.
    fn refuse(&mut self, r: Refusal) {
        let _ = self.to.send(Err(r));
        self.ended = true;
    }
}

impl Drop for Frames {
    /// A request that stopped without ending, as a panicking one does, breaks its body.
    fn drop(&mut self) {
        if !self.ended {
            self.refuse(Refusal::at("the request stopped before its page ended", 0));
        }
    }
}

impl Write for Frames {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.pending.is_empty() {
            return Ok(());
        }
        let frame = Bytes::from(std::mem::take(&mut self.pending));
        self.to
            .send(Ok(frame))
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the client has gone"))
    }
}

/// What a server serves, and where it writes its round.
#[derive(Clone, Copy)]
pub struct Server {
    pub store: &'static FileStore,
    #[cfg(feature = "round")]
    pub log: Option<&'static crate::round::Log>,
}

/// The page a URL's path names under the served directory, if it names one: a `.php` file, by a
/// path with no empty, `.` or `..` segment. The path is taken as written, never decoded.
fn page_named(path: &str) -> Option<&str> {
    let rel = path.strip_prefix('/')?;
    let fine = rel.ends_with(".php")
        && rel
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..");
    fine.then_some(rel)
}

/// The page `name` names in the served directory: kept, or found there and loaded; none when the
/// directory has no such file.
async fn entry(store: &'static FileStore, name: &str) -> Result<Option<&'static Page>, Refusal> {
    let cwd = store.cwd().as_os_str().as_encoded_bytes();
    if let Some(page) = store.kept(name.as_bytes(), cwd) {
        return Ok(Some(page));
    }
    let name = name.to_owned();
    let dir = cwd.to_vec();
    tokio::task::spawn_blocking(move || {
        if !store.cwd().join(&name).is_file() {
            return Ok(None);
        }
        store.load(name.as_bytes(), &dir).map(Some)
    })
    .await
    .map_err(|e| Refusal::at(format!("loading a page: {e}"), 0))?
}

fn status(code: StatusCode) -> Response<Body> {
    let mut r = Response::new(Body::empty());
    *r.status_mut() = code;
    r
}

/// One HTTP request: its page runs as one request on a task of its own, and the response's body
/// is the page as it goes out.
async fn answer(server: Server, req: Request<Incoming>) -> Result<Response<Body>, Infallible> {
    let Some(name) = page_named(req.uri().path()) else {
        return Ok(status(StatusCode::NOT_FOUND));
    };
    let entry = match entry(server.store, name).await {
        Ok(Some(page)) => page,
        Ok(None) => return Ok(status(StatusCode::NOT_FOUND)),
        Err(r) => {
            eprintln!("{r}");
            return Ok(status(StatusCode::INTERNAL_SERVER_ERROR));
        }
    };
    let (to, frames) = unbounded_channel();
    let mut sink = Frames::new(to);
    let store = server.store;
    let request = async move {
        let result = host::run(store, entry, &mut sink).await;
        match &result {
            Ok(_) => sink.end(),
            Err(r) => {
                eprintln!("{r}");
                sink.refuse(r.clone());
            }
        }
        result
    };
    #[cfg(feature = "round")]
    match server.log {
        Some(log) => crate::round::spawn_served(request, log),
        None => drop(tokio::spawn(request)),
    }
    #[cfg(not(feature = "round"))]
    drop(tokio::spawn(request));
    let mut response = Response::new(Body { frames });
    response
        .headers_mut()
        .insert(CONTENT_TYPE, hyper::header::HeaderValue::from_static(HTML));
    Ok(response)
}

/// Serves the store's directory at `addr` until the process is stopped.
pub async fn listen(server: Server, addr: SocketAddr) -> io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    eprintln!(
        "whp: serving {} at http://{}",
        server.store.cwd().display(),
        listener.local_addr()?
    );
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(e) => {
                eprintln!("whp: accepting a connection: {e}");
                continue;
            }
        };
        // A body's frames are small writes, and its last would wait on the client's delayed
        // acknowledgement of the one before: 40 ms on every request after a connection's first.
        if let Err(e) = stream.set_nodelay(true) {
            eprintln!("whp: a connection's delay: {e}");
        }
        tokio::spawn(async move {
            let service = service_fn(move |req| answer(server, req));
            // A connection ends in an error when its client goes away or its body breaks; the
            // refusal that broke a body is already reported.
            let _ = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_its_request_did_not_end_is_broken() {
        let (to, mut frames) = unbounded_channel();
        let mut sink = Frames::new(to);
        sink.write_all(b"so far").unwrap();
        sink.flush().unwrap();
        drop(sink);
        assert_eq!(
            frames.try_recv().unwrap().unwrap(),
            Bytes::from_static(b"so far")
        );
        assert!(frames.try_recv().unwrap().is_err());

        let (to, mut frames) = unbounded_channel();
        let mut sink = Frames::new(to);
        sink.write_all(b"whole").unwrap();
        sink.flush().unwrap();
        sink.end();
        drop(sink);
        assert_eq!(
            frames.try_recv().unwrap().unwrap(),
            Bytes::from_static(b"whole")
        );
        assert!(frames.try_recv().is_err());
    }

    #[test]
    fn a_path_names_a_page_only_under_the_directory() {
        assert_eq!(page_named("/list.php"), Some("list.php"));
        assert_eq!(page_named("/inc/head.php"), Some("inc/head.php"));
        for refused in [
            "/",
            "/list.html",
            "/../list.php",
            "/inc/../list.php",
            "/./list.php",
            "//list.php",
            "/inc//list.php",
            "list.php",
        ] {
            assert_eq!(page_named(refused), None, "{refused}");
        }
    }
}
