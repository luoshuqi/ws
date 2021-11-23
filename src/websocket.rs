//! websocket 服务端实现，基于 rfc6455 https://datatracker.ietf.org/doc/html/rfc6455

use std::hint::unreachable_unchecked;
use std::io;

use sha1::{digest::Output, Digest, Sha1};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::Error;
use crate::request::Request;
use crate::response::{Response, BAD_REQUEST, SWITCHING_PROTOCOLS};

// opcode
const OPCODE_CONTINUATION: u8 = 0x0;
const OPCODE_TEXT: u8 = 0x1;
const OPCODE_BINARY: u8 = 0x2;
const OPCODE_CLOSE: u8 = 0x8;
const OPCODE_PING: u8 = 0x9;
const OPCODE_PONG: u8 = 0xA;

// 最大消息长度, 8 MiB
const MAX_MESSAGE_LEN: usize = 8388608;

#[repr(u8)]
#[derive(Copy, Clone, Debug)]
pub enum Opcode {
    Text = OPCODE_TEXT,
    Binary = OPCODE_BINARY,
    Close = OPCODE_CLOSE,
    Ping = OPCODE_PING,
    Pong = OPCODE_PONG,
}

impl Opcode {
    pub fn new(opcode: u8) -> Option<Self> {
        match opcode {
            OPCODE_TEXT => Some(Self::Text),
            OPCODE_BINARY => Some(Self::Binary),
            OPCODE_CLOSE => Some(Self::Close),
            OPCODE_PING => Some(Self::Ping),
            OPCODE_PONG => Some(Self::Pong),
            _ => None,
        }
    }

    fn is_valid(opcode: u8) -> bool {
        match opcode {
            OPCODE_CONTINUATION | OPCODE_TEXT | OPCODE_BINARY | OPCODE_PING | OPCODE_PONG
            | OPCODE_CLOSE => true,
            _ => false,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Message<'a> {
    opcode: Opcode,
    payload: &'a [u8],
}

impl<'a> Message<'a> {
    #[inline]
    pub fn new(opcode: Opcode, payload: &'a [u8]) -> Self {
        Self { opcode, payload }
    }

    #[inline]
    pub fn text(payload: &'a [u8]) -> Self {
        Self::new(Opcode::Text, payload)
    }

    #[inline]
    pub fn binary(payload: &'a [u8]) -> Self {
        Self::new(Opcode::Binary, payload)
    }

    #[inline]
    pub fn close(payload: &'a [u8]) -> Self {
        Self::new(Opcode::Close, payload)
    }

    #[inline]
    pub fn opcode(self) -> Opcode {
        self.opcode
    }

    #[inline]
    pub fn payload(self) -> &'a [u8] {
        self.payload
    }
}

// 状态
enum State {
    // 开始读取
    Start([u8; 2]),
    // 读取长度，长度为2个字节
    ReadLen2([u8; 2]),
    // 读取长度，长度为8个字节
    ReadLen8([u8; 8]),
    // 读取 mask 和 payload
    ReadMaskPayload(usize),
}

pub struct WebSocket<T: AsyncRead + AsyncWrite + Unpin> {
    stream: T,
    // payload buffer
    payload: Vec<u8>,
    // 已完整读取的 payload 的长度
    payload_len: usize,
    // 读取状态
    state: State,
    // 当前状态下已读取的字节数
    read: usize,
    // opcode, OPCODE_CONTINUATION 表示读取第一个分片
    opcode: u8,
    // 是否为最后一个分片
    fin: bool,
}

impl<T: AsyncRead + AsyncWrite + Unpin> WebSocket<T> {
    // 升级到 websocket
    pub async fn upgrade(req: &Request<'_>, mut stream: T) -> io::Result<Option<Self>> {
        if can_upgrade(req) {
            let mut response = Response::status(SWITCHING_PROTOCOLS);
            response.add_header("Upgrade", "websocket");
            response.add_header("Connection", "Upgrade");

            let key = req.header("sec-websocket-key").unwrap();
            let accept = format!("{}258EAFA5-E914-47DA-95CA-C5AB0DC85B11", key);
            let accept = base64::encode(sha1(accept));
            response.add_header("Sec-WebSocket-Accept", &accept);
            response.write(&mut stream).await?;
            Ok(Some(WebSocket::new(stream)))
        } else {
            Response::status(BAD_REQUEST).write(&mut stream).await?;
            Ok(None)
        }
    }

    fn new(stream: T) -> Self {
        Self {
            stream,
            payload: Vec::new(),
            payload_len: 0,
            state: State::Start([0; 2]),
            read: 0,
            opcode: OPCODE_CONTINUATION,
            fin: false,
        }
    }

    // 接收消息
    // 可安全取消 (cancellation safe), 下次接收会从中断的地方继续
    pub async fn receive(&mut self) -> Result<Option<Message<'_>>, Error> {
        loop {
            match self.state {
                State::Start(ref mut buf) => {
                    // 清除上个消息的 payload
                    if self.opcode == OPCODE_CONTINUATION && self.payload_len > 0 {
                        self.payload_len = 0;
                        unsafe {
                            self.payload.set_len(0);
                        }
                    }

                    read(&mut self.stream, buf, &mut self.read).await?;
                    if self.read == 0 {
                        // 连接关闭
                        return Ok(None);
                    }

                    if self.read < buf.len() {
                        error!("early eof");
                    }

                    if (buf[0] & 0x70) > 0 {
                        // 不支持扩展, RSV1, RSV2, RSV3 必须为0
                        error!("unexpected rsv {}", (buf[0] >> 4) & 0x7);
                    }

                    self.fin = (buf[0] >> 7) == 1;
                    let opcode = buf[0] & 0xF;
                    if !Opcode::is_valid(opcode) {
                        error!("unexpected opcode {}", opcode);
                    }

                    if self.opcode == OPCODE_CONTINUATION {
                        // 首个分片, opcode 不能为 OPCODE_CONTINUATION
                        if opcode != OPCODE_CONTINUATION {
                            self.opcode = opcode;
                        } else {
                            error!("unexpected opcode {}", opcode);
                        }
                    } else if opcode != OPCODE_CONTINUATION {
                        // 非首个分片, opcode 必须为 OPCODE_CONTINUATION
                        error!("unexpected opcode {}", opcode);
                    }

                    if (buf[1] >> 7) == 0 {
                        error!("mask bit not set");
                    }

                    self.state = match buf[1] & 0x7F {
                        len @ 0..=125 => State::ReadMaskPayload(len as _),
                        126 => State::ReadLen2([0; 2]),
                        127 => State::ReadLen8([0; 8]),
                        _ => unsafe { unreachable_unchecked() },
                    };
                    self.read = 0;
                }
                State::ReadLen2(mut buf) => {
                    read(&mut self.stream, &mut buf, &mut self.read).await?;
                    if self.read == buf.len() {
                        self.state = State::ReadMaskPayload(u16::from_be_bytes(buf) as _);
                        self.read = 0;
                    } else {
                        error!("early eof");
                    }
                }
                State::ReadLen8(mut buf) => {
                    read(&mut self.stream, &mut buf, &mut self.read).await?;
                    if self.read == buf.len() {
                        self.state = State::ReadMaskPayload(u64::from_be_bytes(buf) as _);
                        self.read = 0;
                    } else {
                        error!("early eof");
                    }
                }
                State::ReadMaskPayload(len) => {
                    if self.read == 0 {
                        if self.payload_len + len > MAX_MESSAGE_LEN {
                            error!("message too large");
                        }
                        self.payload.reserve(len + 4);
                        unsafe {
                            self.payload.set_len(self.payload_len + len + 4);
                        }
                    }

                    let buf = &mut self.payload[self.payload_len..];
                    read(&mut self.stream, buf, &mut self.read).await?;
                    if self.read != len + 4 {
                        error!("early eof");
                    }

                    let mut mask = [0u8; 4];
                    mask.copy_from_slice(&buf[..4]);
                    for i in 0..len {
                        buf[i] = buf[i + 4] ^ mask[i % 4];
                    }

                    self.payload_len += len;
                    unsafe {
                        self.payload.set_len(self.payload_len);
                    }

                    if self.fin {
                        //消息读取完成
                        let opcode = Opcode::new(self.opcode).unwrap();
                        let msg = Message::new(opcode, &self.payload);
                        self.state = State::Start([0; 2]);
                        self.read = 0;
                        self.opcode = OPCODE_CONTINUATION;
                        return Ok(Some(msg));
                    } else {
                        // 继续读取下个分片
                        self.read = 0;
                        self.state = State::Start([0; 2]);
                    }
                }
            }
        }
    }

    // 发送消息, 不分片
    pub async fn send(&mut self, msg: Message<'_>) -> io::Result<()> {
        let mut buf = [0u8; 10];
        buf[0] = (1 << 7) | msg.opcode() as u8;

        let payload = msg.payload();
        let len = payload.len();
        if len <= 125 {
            buf[1] = len as u8;
            self.stream.write_all(&buf[..2]).await?;
        } else if len <= u16::MAX as usize {
            buf[1] = 126;
            buf[2..4].copy_from_slice(&(len as u16).to_be_bytes());
            self.stream.write_all(&buf[..4]).await?;
        } else {
            buf[1] = 127;
            buf[2..].copy_from_slice(&(len as u64).to_be_bytes());
            self.stream.write_all(&buf).await?;
        }
        self.stream.write_all(payload).await?;
        Ok(())
    }
}

async fn read(
    reader: &mut (impl AsyncRead + Unpin),
    buf: &mut [u8],
    read: &mut usize,
) -> io::Result<()> {
    loop {
        let n = reader.read(&mut buf[*read..]).await?;
        *read += n;
        if *read == buf.len() || n == 0 {
            return Ok(());
        }
    }
}

fn can_upgrade(req: &Request) -> bool {
    fn check_connection_header(value: &str) -> bool {
        for v in value.split(',') {
            if v.trim().eq_ignore_ascii_case("Upgrade") {
                return true;
            }
        }
        false
    }

    req.method() == "GET"
        && req.version() >= 1.1
        && req.header("host").is_some()
        && req.header("sec-websocket-version") == Some("13")
        && req.header("sec-websocket-key").is_some()
        && match req.header("upgrade") {
            Some(value) if value.eq_ignore_ascii_case("websocket") => true,
            _ => return false,
        }
        && match req.header("connection") {
            Some(value) if check_connection_header(value) => true,
            _ => false,
        }
}

#[inline]
fn sha1(data: impl AsRef<[u8]>) -> Output<Sha1> {
    let mut digest = Sha1::new();
    digest.update(data);
    digest.finalize()
}
