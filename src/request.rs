use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::str::from_utf8;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::time::{sleep, Duration};

use crate::error::Error;

#[derive(Debug)]
pub struct Request<'a> {
    method: &'a str,
    uri: &'a str,
    version: f32,
    headers: HashMap<HeaderName<'a>, &'a str>,
}

impl<'a> Request<'a> {
    pub async fn new(
        stream: &mut (impl AsyncRead + Unpin),
        buf: &'a mut [u8],
        timeout: Duration,
    ) -> Result<Request<'a>, Error> {
        tokio::select! {
            req = read_request(stream, buf) => {
                parse_request(req?)
            }
            _ = sleep(timeout) => error!("request read timeout"),
        }
    }

    #[inline]
    pub fn uri(&self) -> &str {
        self.uri
    }

    #[inline]
    pub fn method(&self) -> &str {
        self.method
    }

    #[inline]
    pub fn version(&self) -> f32 {
        self.version
    }

    #[inline]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&HeaderName(name)).map(|v| *v)
    }
}

async fn read_request<'a>(
    stream: &mut (impl AsyncRead + Unpin),
    buf: &'a mut [u8],
) -> Result<&'a [u8], Error> {
    let mut read: usize = 0;
    loop {
        let n = stream.read(&mut buf[read..]).await?;
        read += n;
        if read > 3 && !buf.starts_with(b"GET ") {
            error!("method not allowed");
        }

        if buf[..read].ends_with(b"\r\n\r\n") {
            return Ok(&buf[..read]);
        }

        if read == buf.len() {
            error!("request too large");
        }

        if n == 0 {
            error!("early eof");
        }
    }
}

// 解析不带 body 的请求
fn parse_request(buf: &[u8]) -> Result<Request, Error> {
    let mut lines = from_utf8(&buf[..buf.len() - 4])?.split("\r\n");
    let req_line = match lines.next() {
        Some(line) => line,
        None => error!("missing request line"),
    };
    let (method, uri, version) = match parse_request_line(req_line) {
        Some(v) => v,
        None => error!("parse request line failed: {}", req_line),
    };

    let mut headers = HashMap::new();
    loop {
        match lines.next() {
            Some(line) => match parse_header_line(line) {
                Some((name, value)) => {
                    headers.insert(HeaderName(name), value);
                }
                None => error!("parse header line failed: {}", line),
            },
            None => break,
        }
    }

    Ok(Request {
        method,
        uri,
        version,
        headers,
    })
}

fn parse_request_line(line: &str) -> Option<(&str, &str, f32)> {
    let mut splits = line.split(' ');
    let method = splits.next()?;
    let uri = splits.next()?;
    let protocol = splits.next()?;

    if protocol.starts_with("HTTP/") {
        let version: f32 = protocol[5..].parse().ok()?;
        Some((method, uri, version))
    } else {
        None
    }
}

fn parse_header_line(line: &str) -> Option<(&str, &str)> {
    let mut splits = line.splitn(2, ':');
    Some((splits.next()?, splits.next()?.trim()))
}

// 大小写不敏感
#[derive(Debug)]
struct HeaderName<'a>(&'a str);

impl<'a> Hash for HeaderName<'a> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_ascii_lowercase().hash(state)
    }
}

impl<'a> PartialEq for HeaderName<'a> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.0.eq_ignore_ascii_case(other.0)
    }
}

impl<'a> Eq for HeaderName<'a> {}
