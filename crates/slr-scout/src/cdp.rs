//! A minimal Chrome DevTools Protocol client: a WebSocket over
//! `std::net::TcpStream` to Chrome's local debugging port (HTTP upgrade,
//! masked client text frames, unmasked server frames with 16/64-bit lengths
//! and continuations; pings answered), and JSON-RPC calls on top.

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const OP_CONTINUATION: u8 = 0x0;
const OP_TEXT: u8 = 0x1;
const OP_BINARY: u8 = 0x2;
const OP_CLOSE: u8 = 0x8;
const OP_PING: u8 = 0x9;
const OP_PONG: u8 = 0xA;

/// Pseudo-random bytes for the handshake key and frame masks (no crypto
/// needed: masking only stops proxies from caching).
fn noise(seed: u64) -> u64 {
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let mut x = t ^ seed.rotate_left(17) ^ u64::from(std::process::id()) << 32;
    // xorshift64*
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D) | 1
}

pub fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A client frame: FIN set, masked with `mask`.
pub fn encode_frame(opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut f = Vec::with_capacity(payload.len() + 14);
    f.push(0x80 | opcode);
    let len = payload.len();
    if len < 126 {
        f.push(0x80 | len as u8);
    } else if len <= 0xFFFF {
        f.push(0x80 | 126);
        f.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        f.push(0x80 | 127);
        f.extend_from_slice(&(len as u64).to_be_bytes());
    }
    f.extend_from_slice(&mask);
    f.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    f
}

/// One frame as read.
#[derive(Debug, PartialEq)]
pub struct Frame {
    pub fin: bool,
    pub opcode: u8,
    pub payload: Vec<u8>,
}

pub fn read_frame(r: &mut impl Read) -> Result<Frame, String> {
    let mut head = [0u8; 2];
    r.read_exact(&mut head)
        .map_err(|e| format!("websocket: {e}"))?;
    let fin = head[0] & 0x80 != 0;
    let opcode = head[0] & 0x0F;
    let masked = head[1] & 0x80 != 0;
    let len = match head[1] & 0x7F {
        126 => {
            let mut b = [0u8; 2];
            r.read_exact(&mut b)
                .map_err(|e| format!("websocket: {e}"))?;
            u64::from(u16::from_be_bytes(b))
        }
        127 => {
            let mut b = [0u8; 8];
            r.read_exact(&mut b)
                .map_err(|e| format!("websocket: {e}"))?;
            u64::from_be_bytes(b)
        }
        n => u64::from(n),
    };
    if len > 256 << 20 {
        return Err(format!("websocket: frame of {len} bytes"));
    }
    let mut mask = [0u8; 4];
    if masked {
        r.read_exact(&mut mask)
            .map_err(|e| format!("websocket: {e}"))?;
    }
    let mut payload = vec![0u8; len as usize];
    r.read_exact(&mut payload)
        .map_err(|e| format!("websocket: {e}"))?;
    if masked {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
    }
    Ok(Frame {
        fin,
        opcode,
        payload,
    })
}

/// A WebSocket connection over any byte stream.
pub struct WebSocket<S: Read + Write> {
    stream: S,
    sent: u64,
}

impl<S: Read + Write> WebSocket<S> {
    /// Wrap a stream whose upgrade handshake is done.
    pub fn new(stream: S) -> Self {
        WebSocket { stream, sent: 0 }
    }

    fn send(&mut self, opcode: u8, payload: &[u8]) -> Result<(), String> {
        self.sent += 1;
        let mask = (noise(self.sent) as u32).to_be_bytes();
        self.stream
            .write_all(&encode_frame(opcode, payload, mask))
            .map_err(|e| format!("websocket: {e}"))
    }

    pub fn send_text(&mut self, text: &str) -> Result<(), String> {
        self.send(OP_TEXT, text.as_bytes())
    }

    /// The next text message (continuations joined, pings answered, pongs
    /// skipped); an error once the peer closes.
    pub fn recv_text(&mut self) -> Result<String, String> {
        let mut message: Vec<u8> = Vec::new();
        let mut in_message = false;
        loop {
            let f = read_frame(&mut self.stream)?;
            match f.opcode {
                OP_PING => self.send(OP_PONG, &f.payload)?,
                OP_PONG => {}
                OP_CLOSE => return Err("websocket: closed by Chrome".into()),
                OP_TEXT | OP_BINARY | OP_CONTINUATION => {
                    if f.opcode != OP_CONTINUATION {
                        message.clear();
                        in_message = true;
                    } else if !in_message {
                        return Err("websocket: continuation without a message".into());
                    }
                    message.extend_from_slice(&f.payload);
                    if f.fin {
                        return String::from_utf8(message)
                            .map_err(|_| "websocket: message is not UTF-8".to_string());
                    }
                }
                op => return Err(format!("websocket: unknown opcode {op}")),
            }
        }
    }
}

/// `ws://127.0.0.1:<port><path>` → (port, path).
pub fn parse_ws_url(url: &str) -> Result<(u16, String), String> {
    let rest = url
        .strip_prefix("ws://127.0.0.1:")
        .or_else(|| url.strip_prefix("ws://localhost:"))
        .ok_or_else(|| format!("{url}: not a local DevTools URL"))?;
    let (port, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let port = port.parse().map_err(|_| format!("{url}: bad port"))?;
    Ok((
        port,
        if path.is_empty() {
            "/".into()
        } else {
            path.into()
        },
    ))
}

/// Open a WebSocket to Chrome's DevTools endpoint.
pub fn connect(url: &str, timeout: Duration) -> Result<WebSocket<TcpStream>, String> {
    let (port, path) = parse_ws_url(url)?;
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .map_err(|e| format!("DevTools port {port}: {e}"))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|e| format!("DevTools: {e}"))?;
    let key = base64(&[noise(1).to_be_bytes(), noise(2).to_be_bytes()].concat());
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    )
    .map_err(|e| format!("DevTools: {e}"))?;
    // Read the response head byte by byte, so no frame bytes are consumed.
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        stream
            .read_exact(&mut byte)
            .map_err(|e| format!("DevTools handshake: {e}"))?;
        head.push(byte[0]);
        if head.len() > 16 * 1024 {
            return Err("DevTools handshake: response too long".into());
        }
    }
    let status = String::from_utf8_lossy(&head);
    if status.split_whitespace().nth(1) != Some("101") {
        return Err(format!(
            "DevTools handshake: {}",
            status.lines().next().unwrap_or_default()
        ));
    }
    Ok(WebSocket::new(stream))
}

/// JSON-RPC over the socket: [`Cdp::call`] waits for its answer and skips
/// events.
pub struct Cdp<S: Read + Write> {
    ws: WebSocket<S>,
    next_id: u64,
}

impl<S: Read + Write> Cdp<S> {
    pub fn new(ws: WebSocket<S>) -> Self {
        Cdp { ws, next_id: 0 }
    }

    /// Call `method` (on a page session when given); returns `result`.
    pub fn call(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = json!(s);
        }
        self.ws.send_text(&msg.to_string())?;
        loop {
            let text = self.ws.recv_text()?;
            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if v["id"].as_u64() != Some(id) {
                continue;
            }
            if let Some(err) = v.get("error") {
                return Err(format!(
                    "{method}: {}",
                    err["message"].as_str().unwrap_or("error")
                ));
            }
            return Ok(v["result"].clone());
        }
    }

    /// Evaluate a JavaScript expression in the page; returns its value.
    pub fn eval(&mut self, session: &str, expression: &str) -> Result<Value, String> {
        let r = self.call(
            "Runtime.evaluate",
            json!({ "expression": expression, "awaitPromise": true, "returnByValue": true }),
            Some(session),
        )?;
        if let Some(e) = r.get("exceptionDetails") {
            return Err(format!(
                "page script: {}",
                e["exception"]["description"]
                    .as_str()
                    .or(e["text"].as_str())
                    .unwrap_or("exception")
            ));
        }
        Ok(r["result"]["value"].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Reads canned server bytes; collects what the client writes.
    struct Duplex {
        input: Cursor<Vec<u8>>,
        output: Vec<u8>,
    }

    impl Read for Duplex {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(buf)
        }
    }

    impl Write for Duplex {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.output.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// An unmasked server frame.
    fn server(fin: bool, opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mut f = vec![if fin { 0x80 } else { 0 } | opcode];
        let len = payload.len();
        if len < 126 {
            f.push(len as u8);
        } else if len <= 0xFFFF {
            f.push(126);
            f.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            f.push(127);
            f.extend_from_slice(&(len as u64).to_be_bytes());
        }
        f.extend_from_slice(payload);
        f
    }

    fn duplex(frames: &[Vec<u8>]) -> Duplex {
        Duplex {
            input: Cursor::new(frames.concat()),
            output: Vec::new(),
        }
    }

    #[test]
    fn encodes_masked_frames_of_every_length() {
        let mask = [1, 2, 3, 4];
        for len in [0usize, 5, 125, 126, 300, 0xFFFF, 0x10000] {
            let payload: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let f = encode_frame(OP_TEXT, &payload, mask);
            assert_eq!(f[0], 0x81);
            assert_eq!(f[1] & 0x80, 0x80, "client frames are masked");
            let back = read_frame(&mut Cursor::new(f)).unwrap();
            assert_eq!(back.opcode, OP_TEXT);
            assert!(back.fin);
            assert_eq!(back.payload, payload, "len {len}");
        }
        let f = encode_frame(OP_TEXT, b"abcd", mask);
        assert_eq!(&f[..2], &[0x81, 0x84]);
        assert_eq!(&f[2..6], &mask);
        assert_eq!(&f[6..], &[b'a' ^ 1, b'b' ^ 2, b'c' ^ 3, b'd' ^ 4]);
    }

    #[test]
    fn reads_server_frames_with_long_lengths_and_continuations() {
        let big = "x".repeat(70_000);
        let mut ws = WebSocket::new(duplex(&[
            server(true, OP_TEXT, b"short"),
            server(true, OP_TEXT, big.as_bytes()),
            server(false, OP_TEXT, b"hel"),
            server(true, OP_PING, b"p"),
            server(false, OP_CONTINUATION, b"lo "),
            server(true, OP_CONTINUATION, b"world"),
            server(true, OP_PONG, b""),
            server(true, OP_CLOSE, b""),
        ]));
        assert_eq!(ws.recv_text().unwrap(), "short");
        assert_eq!(ws.recv_text().unwrap(), big);
        assert_eq!(ws.recv_text().unwrap(), "hello world");
        // The ping was answered with a masked pong carrying its payload.
        let pong = read_frame(&mut Cursor::new(ws.stream.output.clone())).unwrap();
        assert_eq!((pong.opcode, pong.payload.as_slice()), (OP_PONG, &b"p"[..]));
        assert!(ws.recv_text().is_err());
    }

    #[test]
    fn calls_skip_events_and_other_ids() {
        let mut cdp = Cdp::new(WebSocket::new(duplex(&[
            server(
                true,
                OP_TEXT,
                br#"{"method":"Network.requestWillBeSent","params":{}}"#,
            ),
            server(true, OP_TEXT, br#"{"id":99,"result":{}}"#),
            server(true, OP_TEXT, br#"{"id":1,"result":{"targetId":"T1"}}"#),
            server(
                true,
                OP_TEXT,
                br#"{"id":2,"error":{"message":"No target"}}"#,
            ),
        ])));
        let r = cdp
            .call("Target.createTarget", json!({"url": "about:blank"}), None)
            .unwrap();
        assert_eq!(r["targetId"], "T1");
        let sent = read_frame(&mut Cursor::new(cdp.ws.stream.output.clone())).unwrap();
        let sent: Value = serde_json::from_slice(&sent.payload).unwrap();
        assert_eq!(sent["method"], "Target.createTarget");
        assert_eq!(sent["id"], 1);
        assert_eq!(
            cdp.call("Page.navigate", json!({}), Some("S1"))
                .unwrap_err(),
            "Page.navigate: No target"
        );
    }

    #[test]
    fn base64_and_urls() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0u8; 16]).len(), 24);
        assert_eq!(
            parse_ws_url("ws://127.0.0.1:9222/devtools/browser/abc").unwrap(),
            (9222, "/devtools/browser/abc".to_string())
        );
        assert!(parse_ws_url("ws://example.com:1/x").is_err());
        assert!(parse_ws_url("ws://127.0.0.1:x/").is_err());
    }
}
