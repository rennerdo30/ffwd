//! AI help against an OpenAI-compatible endpoint the user configures
//! (docs/SPEC.md §5). Off by default: nothing is sent until an endpoint is
//! set, and the caller shows the user exactly what will be sent first.
//!
//! - `http://` (local servers: Ollama, LM Studio, llama.cpp) goes through a
//!   small HTTP/1.1 client on `std::net`, with chunked decoding.
//! - `https://` goes through the system `curl`, configured on stdin so the
//!   key never shows on a command line or touches the disk.
//!
//! Replies stream (`stream: true`, server-sent events) and stop when the
//! caller sets the cancel flag. Everything here blocks: call it on a worker.

use std::io::{BufRead, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Where requests go. `base` ends before `/chat/completions`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Endpoint {
    pub base: String,
    pub model: String,
    /// A bearer token, if the endpoint needs one.
    pub key: Option<String>,
}

impl Endpoint {
    /// The full chat URL.
    pub fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.base.trim().trim_end_matches('/'))
    }

    /// Host and port, for showing the user where data goes.
    pub fn host(&self) -> String {
        let rest = self.base.split_once("://").map_or(self.base.as_str(), |x| x.1);
        rest.split('/').next().unwrap_or(rest).to_string()
    }

    /// Whether requests stay on this machine.
    pub fn is_local(&self) -> bool {
        let h = self.host();
        let host = h.rsplit_once(':').map_or(h.as_str(), |x| x.0);
        matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "::1")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub role: &'static str,
    pub content: String,
}

/// The request body.
pub fn body(ep: &Endpoint, messages: &[Message]) -> String {
    let msgs: Vec<serde_json::Value> =
        messages.iter().map(|m| serde_json::json!({ "role": m.role, "content": m.content })).collect();
    serde_json::json!({ "model": ep.model, "messages": msgs, "stream": true, "temperature": 0.2 }).to_string()
}

/// How long a reply may stay silent before ffwd gives up.
const IDLE: Duration = Duration::from_secs(120);
/// The most reply text kept.
const MAX_REPLY: usize = 64 * 1024;

/// Sends `messages` and calls `on_token` with each piece of the reply.
/// Returns the whole reply. Stops early (with an error) on cancel.
pub fn chat_stream(
    ep: &Endpoint,
    messages: &[Message],
    cancel: &AtomicBool,
    on_token: &mut dyn FnMut(&str),
) -> Result<String, String> {
    if ep.base.trim().is_empty() {
        return Err("AI is off: set an endpoint in Settings".into());
    }
    if ep.model.trim().is_empty() {
        return Err("Set the AI model in Settings".into());
    }
    let url = ep.chat_url();
    let body = body(ep, messages);
    let mut sse = Sse::default();
    let mut reply = String::new();
    let mut feed = |chunk: &[u8]| -> Result<bool, String> {
        for event in sse.push(chunk) {
            if event.trim() == "[DONE]" {
                return Ok(false);
            }
            if let Some(t) = token(&event)? {
                if reply.len() < MAX_REPLY {
                    reply.push_str(&t);
                }
                on_token(&t);
            }
        }
        Ok(true)
    };
    if let Some(rest) = url.strip_prefix("http://") {
        http_post(rest, ep.key.as_deref(), &body, cancel, &mut feed)?;
    } else if url.starts_with("https://") {
        curl_post(&url, ep.key.as_deref(), &body, cancel, &mut feed)?;
    } else {
        return Err("The endpoint must start with http:// or https://".into());
    }
    Ok(reply)
}

/// The text of one streamed event, or the server's error.
fn token(event: &str) -> Result<Option<String>, String> {
    let v: serde_json::Value = match serde_json::from_str(event) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    if let Some(e) = v.get("error") {
        let m = e.get("message").and_then(|m| m.as_str()).map(str::to_string).unwrap_or_else(|| e.to_string());
        return Err(format!("the endpoint answered: {m}"));
    }
    Ok(v.pointer("/choices/0/delta/content").and_then(|c| c.as_str()).filter(|s| !s.is_empty()).map(str::to_string))
}

/// Splits a byte stream into server-sent `data:` payloads.
#[derive(Default)]
struct Sse {
    line: Vec<u8>,
}

impl Sse {
    fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        for &b in bytes {
            if b == b'\n' {
                let l = String::from_utf8_lossy(&self.line).trim_end_matches('\r').to_string();
                self.line.clear();
                if let Some(d) = l.strip_prefix("data:") {
                    out.push(d.trim_start().to_string());
                } else if l.starts_with('{') {
                    // Some servers send bare JSON lines.
                    out.push(l);
                }
            } else if self.line.len() < 1 << 20 {
                self.line.push(b);
            }
        }
        out
    }
}

/// POST over plain HTTP/1.1 (`rest` is the URL after `http://`).
fn http_post(
    rest: &str,
    key: Option<&str>,
    body: &str,
    cancel: &AtomicBool,
    feed: &mut dyn FnMut(&[u8]) -> Result<bool, String>,
) -> Result<(), String> {
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let addr = if hostport.contains(':') && !hostport.ends_with(']') {
        hostport.to_string()
    } else {
        format!("{hostport}:80")
    };
    let addrs: Vec<std::net::SocketAddr> =
        std::net::ToSocketAddrs::to_socket_addrs(&addr).map_err(|e| format!("cannot find {hostport}: {e}"))?.collect();
    let mut stream = None;
    for a in addrs {
        if let Ok(s) = std::net::TcpStream::connect_timeout(&a, Duration::from_secs(5)) {
            stream = Some(s);
            break;
        }
    }
    let mut s = stream.ok_or_else(|| format!("nothing answers at {hostport}; is the AI server running?"))?;
    s.set_read_timeout(Some(Duration::from_millis(200))).map_err(|e| e.to_string())?;
    let mut req = format!(
        "POST {path} HTTP/1.1\r\nHost: {hostport}\r\nContent-Type: application/json\r\nAccept: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(k) = key {
        req.push_str(&format!("Authorization: Bearer {k}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).and_then(|_| s.write_all(body.as_bytes())).map_err(|e| format!("send failed: {e}"))?;
    let mut buf = [0u8; 8192];
    let mut head = Vec::new();
    let mut dechunk: Option<Chunked> = None;
    let mut status = 0u16;
    let mut error_body = Vec::new();
    let mut last = Instant::now();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let n = match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {
                if last.elapsed() > IDLE {
                    return Err("the AI server stopped answering".into());
                }
                continue;
            }
            Err(e) => return Err(format!("the connection broke: {e}")),
        };
        last = Instant::now();
        let after_head: Vec<u8>;
        let mut data = &buf[..n];
        if status == 0 {
            head.extend_from_slice(data);
            let Some(end) = head.windows(4).position(|w| w == b"\r\n\r\n") else {
                if head.len() > 64 * 1024 {
                    return Err("the server sent no valid HTTP answer".into());
                }
                continue;
            };
            let text = String::from_utf8_lossy(&head[..end]).to_string();
            status = text.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
            let chunked = text.lines().any(|l| l.to_ascii_lowercase().replace(' ', "") == "transfer-encoding:chunked");
            dechunk = chunked.then(Chunked::default);
            after_head = head[end + 4..].to_vec();
            data = &after_head;
        }
        let plain = match &mut dechunk {
            Some(c) => c.push(data),
            None => data.to_vec(),
        };
        if status != 200 {
            if error_body.len() < 16 * 1024 {
                error_body.extend_from_slice(&plain);
            }
            continue;
        }
        if !feed(&plain)? {
            return Ok(());
        }
    }
    if status != 200 {
        return Err(http_error(status, &error_body));
    }
    Ok(())
}

fn http_error(status: u16, body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let msg = serde_json::from_str::<serde_json::Value>(text.trim())
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(|m| m.as_str()).map(str::to_string))
        .unwrap_or_else(|| text.trim().chars().take(300).collect());
    if status == 0 {
        return "the server sent no valid HTTP answer".into();
    }
    format!("the endpoint answered HTTP {status}: {msg}")
}

/// Incremental `Transfer-Encoding: chunked` decoding.
#[derive(Default)]
struct Chunked {
    buf: Vec<u8>,
    /// Bytes left in the current chunk (plus its CRLF), or none between.
    left: Option<usize>,
}

impl Chunked {
    fn push(&mut self, data: &[u8]) -> Vec<u8> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        loop {
            match self.left {
                None => {
                    let Some(p) = self.buf.windows(2).position(|w| w == b"\r\n") else { break };
                    let size = String::from_utf8_lossy(&self.buf[..p]);
                    let size = usize::from_str_radix(size.split(';').next().unwrap_or("").trim(), 16).unwrap_or(0);
                    self.buf.drain(..p + 2);
                    if size == 0 {
                        self.buf.clear();
                        break;
                    }
                    self.left = Some(size + 2);
                }
                Some(n) => {
                    let take = n.min(self.buf.len());
                    let data_end = take.min(n.saturating_sub(2));
                    out.extend_from_slice(&self.buf[..data_end]);
                    self.buf.drain(..take);
                    self.left = if take == n { None } else { Some(n - take) };
                    if self.left.is_some() {
                        // The rest of this chunk has not arrived yet.
                        break;
                    }
                }
            }
        }
        out
    }
}

/// A string for a curl config file: quoted, with `\` and `"` escaped.
fn curl_quote(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// The curl config ffwd sends on stdin (URL, headers, body).
fn curl_config(url: &str, key: Option<&str>, body: &str) -> String {
    let mut c = format!("url = {}\n", curl_quote(url));
    c.push_str("header = \"Content-Type: application/json\"\n");
    c.push_str("header = \"Accept: text/event-stream\"\n");
    if let Some(k) = key {
        c.push_str(&format!("header = {}\n", curl_quote(&format!("Authorization: Bearer {k}"))));
    }
    c.push_str(&format!("data-binary = {}\n", curl_quote(body)));
    c
}

/// POST over HTTPS through the system curl.
fn curl_post(
    url: &str,
    key: Option<&str>,
    body: &str,
    cancel: &AtomicBool,
    feed: &mut dyn FnMut(&[u8]) -> Result<bool, String>,
) -> Result<(), String> {
    let mut cmd = std::process::Command::new("curl");
    cmd.args(["--silent", "--show-error", "--no-buffer", "--proto", "=https", "--max-time", "600", "--config", "-"])
        .args(["--write-out", "\n\u{1}%{http_code}\n"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| format!("could not run curl for the https endpoint: {e}"))?;
    if let Some(mut si) = child.stdin.take() {
        si.write_all(curl_config(url, key, body).as_bytes()).map_err(|e| format!("curl failed: {e}"))?;
    }
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    if let Some(so) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut r = std::io::BufReader::new(so);
            loop {
                let mut line = Vec::new();
                match r.read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                }
            }
        });
    }
    let err_reader = child.stderr.take().map(|mut se| {
        std::thread::spawn(move || {
            let mut b = Vec::new();
            let _ = (&mut se).take(16 * 1024).read_to_end(&mut b);
            b
        })
    });
    let mut status = String::new();
    let mut error_body = Vec::new();
    let mut last = Instant::now();
    let mut stopped = false;
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("cancelled".into());
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                last = Instant::now();
                if let Some(code) = line.strip_prefix(b"\x01") {
                    status = String::from_utf8_lossy(code).trim().to_string();
                    continue;
                }
                if stopped {
                    continue;
                }
                if error_body.len() < 16 * 1024 {
                    error_body.extend_from_slice(&line);
                }
                if !feed(&line)? {
                    stopped = true;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if last.elapsed() > IDLE {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("the AI server stopped answering".into());
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let ok = child.wait().map(|s| s.success()).unwrap_or(false);
    let err = err_reader.and_then(|h| h.join().ok()).unwrap_or_default();
    if status != "200" {
        let code: u16 = status.parse().unwrap_or(0);
        if code == 0 {
            let e = String::from_utf8_lossy(&err).trim().to_string();
            return Err(if e.is_empty() { "curl could not reach the endpoint".into() } else { format!("curl: {e}") });
        }
        return Err(http_error(code, &error_body));
    }
    if !ok && !stopped {
        return Err(format!("curl: {}", String::from_utf8_lossy(&err).trim()));
    }
    Ok(())
}

/// Paths whose content is never sent (secrets and keys).
pub fn is_secret_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let name = p.rsplit(['/', '\\']).next().unwrap_or(&p);
    name == ".env"
        || name.starts_with(".env.")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.ends_with(".p12")
        || name.ends_with(".pfx")
        || name.ends_with(".keystore")
        || name.ends_with(".jks")
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || name.starts_with("id_ecdsa")
        || name == ".npmrc"
        || name == ".pypirc"
        || name == ".netrc"
        || name.contains("credential")
        || name.contains("secret")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn sse_and_chunked_decoding() {
        let mut s = Sse::default();
        let ev = s.push(b"data: {\"a\":1}\n\ndata: [DO");
        assert_eq!(ev, ["{\"a\":1}"]);
        assert_eq!(s.push(b"NE]\r\n"), ["[DONE]"]);
        let mut c = Chunked::default();
        let mut out = c.push(b"5\r\nhel");
        out.extend(c.push(b"lo\r\n3;x=y\r\n wo\r\n"));
        out.extend(c.push(b"0\r\n\r\n"));
        assert_eq!(out, b"hello wo");
        assert_eq!(token(r#"{"choices":[{"delta":{"content":"Hi"}}]}"#).unwrap().as_deref(), Some("Hi"));
        assert!(token(r#"{"error":{"message":"model not found"}}"#).is_err());
        assert!(token("not json").unwrap().is_none());
    }

    #[test]
    fn curl_config_escapes_and_keeps_the_key_off_the_command_line() {
        let c = curl_config("https://x/v1/chat/completions", Some("s3cr\"t"), "{\"a\":\"b\\\\c\"}");
        assert!(c.contains("header = \"Authorization: Bearer s3cr\\\"t\""), "{c}");
        assert!(c.contains("data-binary = \"{\\\"a\\\":\\\"b\\\\\\\\c\\\"}\""), "{c}");
    }

    #[test]
    fn endpoints_and_secrets() {
        let e = Endpoint { base: "http://localhost:11434/v1/".into(), model: "m".into(), key: None };
        assert_eq!(e.chat_url(), "http://localhost:11434/v1/chat/completions");
        assert_eq!(e.host(), "localhost:11434");
        assert!(e.is_local());
        assert!(!Endpoint { base: "https://api.example.com/v1".into(), ..Default::default() }.is_local());
        for s in
            [".env", "config/.env.production", "certs/server.pem", "id_rsa", "aws_credentials.json", "app.secret.yml"]
        {
            assert!(is_secret_path(s), "{s}");
        }
        for s in ["src/main.rs", "README.md", "environment.ts"] {
            assert!(!is_secret_path(s), "{s}");
        }
    }

    /// A fake OpenAI-compatible server on localhost: chunked SSE replies
    /// arrive token by token; an HTTP error comes back as text.
    #[test]
    fn streams_from_a_local_server_and_reports_errors() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            for (i, conn) in listener.incoming().take(2).enumerate() {
                let mut s = conn.unwrap();
                let mut req = Vec::new();
                let mut buf = [0u8; 4096];
                // Read headers and the body (by Content-Length).
                loop {
                    let n = s.read(&mut buf).unwrap();
                    req.extend_from_slice(&buf[..n]);
                    if let Some(end) = req.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&req[..end]).to_lowercase();
                        let len: usize = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse().unwrap()))
                            .unwrap();
                        if req.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                let text = String::from_utf8_lossy(&req).to_string();
                assert!(text.starts_with("POST /v1/chat/completions HTTP/1.1"), "{text}");
                assert!(text.contains("\"stream\":true"));
                if i == 0 {
                    let events = [
                        "data: {\"choices\":[{\"delta\":{\"content\":\"Add \"}}]}\n\n",
                        "data: {\"choices\":[{\"delta\":{\"content\":\"tests\"}}]}\n\n",
                        "data: [DONE]\n\n",
                    ];
                    s.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
                    )
                    .unwrap();
                    for e in events {
                        s.write_all(format!("{:x}\r\n{e}\r\n", e.len()).as_bytes()).unwrap();
                        s.flush().unwrap();
                        std::thread::sleep(Duration::from_millis(20));
                    }
                    s.write_all(b"0\r\n\r\n").unwrap();
                } else {
                    let b = "{\"error\":{\"message\":\"model 'x' not found\"}}";
                    s.write_all(format!("HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\n\r\n{b}", b.len()).as_bytes())
                        .unwrap();
                }
            }
        });
        let ep = Endpoint { base: format!("http://127.0.0.1:{port}/v1"), model: "m".into(), key: None };
        let msgs = [Message { role: "user", content: "hi".into() }];
        let mut tokens = Vec::new();
        let reply = chat_stream(&ep, &msgs, &AtomicBool::new(false), &mut |t| tokens.push(t.to_string())).unwrap();
        assert_eq!(tokens, ["Add ", "tests"]);
        assert_eq!(reply, "Add tests");
        let e = chat_stream(&ep, &msgs, &AtomicBool::new(false), &mut |_| {}).unwrap_err();
        assert!(e.contains("404") && e.contains("model 'x' not found"), "{e}");
        server.join().unwrap();
        // Nothing listening: a clear message.
        let dead = Endpoint { base: "http://127.0.0.1:9/v1".into(), model: "m".into(), key: None };
        assert!(
            chat_stream(&dead, &msgs, &AtomicBool::new(false), &mut |_| {}).unwrap_err().contains("nothing answers")
        );
        // Off.
        assert!(chat_stream(&Endpoint::default(), &msgs, &AtomicBool::new(false), &mut |_| {}).is_err());
    }
}
