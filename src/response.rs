use std::collections::HashMap;
use std::io;
use std::time::SystemTime;

use httpdate::fmt_http_date;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

pub const SWITCHING_PROTOCOLS: Status = Status(101, "Switching Protocols");
pub const OK: Status = Status(200, "OK");
pub const BAD_REQUEST: Status = Status(400, "Bad Request");
pub const NOT_FOUND: Status = Status(404, "Not Found");

pub struct Status(pub u16, pub &'static str);

pub struct Response<'a, T: AsyncRead + Unpin> {
    status: Status,
    headers: HashMap<&'a str, &'a str>,
    content_length: u64,
    body: Option<T>,
}

impl<'a, T: AsyncRead + Unpin> Response<'a, T> {
    #[inline]
    pub fn new(
        status: Status,
        headers: HashMap<&'a str, &'a str>,
        content_length: u64,
        body: Option<T>,
    ) -> Self {
        Self {
            status,
            headers,
            content_length,
            body,
        }
    }

    #[inline]
    pub fn add_header(&mut self, key: &'a str, value: &'a str) {
        self.headers.insert(key, value);
    }

    pub async fn write(mut self, writer: &mut (impl AsyncWrite + Unpin)) -> io::Result<()> {
        let mut buf = format!("HTTP/1.1 {} {}\r\n", self.status.0, self.status.1);
        buf.push_str(&format!("date: {}\r\n", fmt_http_date(SystemTime::now())));
        buf.push_str(&format!("content-length: {}\r\n", self.content_length));
        for (k, v) in self.headers {
            buf.push_str(&format!("{}: {}\r\n", k, v));
        }
        buf.push_str("\r\n");
        writer.write_all(buf.as_bytes()).await?;

        if let Some(ref mut body) = self.body {
            tokio::io::copy(body, writer).await?;
        }

        Ok(())
    }
}

impl<'a> Response<'a, &'a [u8]> {
    #[inline]
    pub fn status(status: Status) -> Self {
        Self::new(status, HashMap::new(), 0, None)
    }

    #[inline]
    pub fn bytes(status: Status, body: &'a [u8]) -> Self {
        Self::new(status, HashMap::new(), body.len() as _, Some(body))
    }
}
