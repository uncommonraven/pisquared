//! A local HTTP server for tests, answering GET requests by path so that the download and verification code
//! runs against real sockets without reaching the publisher.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

/// How the local server answers one path.
#[derive(Clone)]
pub(crate) enum Reply {
    /// The full body with its length declared.
    Body(Vec<u8>),
    /// A status with no body.
    Status(u16),
    /// A declared length of `declared` bytes, of which only `sent` are written before the connection closes.
    Truncated { declared: usize, sent: Vec<u8> },
    /// The connection closes without any response.
    Silent,
    /// No response is ever sent; the connection is held open, so a client waits indefinitely.
    Hang,
    /// The body is written in pieces with a pause between them, then the connection is held open without an
    /// end declared. A client receives some bytes and then waits.
    Slow {
        body: Vec<u8>,
        pieces: usize,
        pause_ms: u64,
    },
}

/// Starts a local server answering GET requests by path, returning its base address. Paths not in
/// `routes` answer 404.
pub(crate) fn serve(routes: HashMap<String, Reply>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
    let port = listener.local_addr().expect("local address").port();
    let routes = Arc::new(routes);

    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let routes = Arc::clone(&routes);
            thread::spawn(move || answer(stream, &routes));
        }
    });

    format!("http://127.0.0.1:{port}/images/")
}

fn answer(mut stream: TcpStream, routes: &HashMap<String, Reply>) {
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
    }
    let text = String::from_utf8_lossy(&head);
    let path = text
        .lines()
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .unwrap_or_default();

    let reply = routes.get(path).cloned().unwrap_or(Reply::Status(404));
    match reply {
        Reply::Body(body) => {
            let _ = write_response(&mut stream, 200, body.len(), &body);
        }
        Reply::Status(code) => {
            let _ = write_response(&mut stream, code, 0, &[]);
        }
        Reply::Truncated { declared, sent } => {
            let _ = write_response(&mut stream, 200, declared, &sent);
            let _ = stream.shutdown(Shutdown::Both);
        }
        Reply::Silent => {
            let _ = stream.shutdown(Shutdown::Both);
        }
        Reply::Hang => {
            // The request has been read and nothing more is ever written. Parking this thread holds the
            // connection open; the thread is never joined, so this is a way of never answering.
            std::thread::park();
        }
        Reply::Slow {
            body,
            pieces,
            pause_ms,
        } => {
            if write_response(&mut stream, 200, body.len(), &[]).is_err() {
                return;
            }
            let piece = (body.len() / pieces.max(1)).max(1);
            for chunk in body.chunks(piece) {
                if stream
                    .write_all(chunk)
                    .and_then(|_| stream.flush())
                    .is_err()
                {
                    return;
                }
                thread::sleep(std::time::Duration::from_millis(pause_ms));
            }
            std::thread::park();
        }
    }
}

fn write_response(stream: &mut TcpStream, code: u16, length: usize, body: &[u8]) -> io::Result<()> {
    let reason = match code {
        200 => "OK",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "Unknown",
    };
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)
}

pub(crate) fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("client")
}
