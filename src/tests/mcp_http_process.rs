#![cfg(feature = "mcp-streamable-http")]

use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const ENDPOINT: &str = "http://127.0.0.1:12899/desktop";
const REQUEST: &str = r#"{"jsonrpc":"2.0","id":7,"method":"tools/list","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}"#;
const CALL_REQUEST: &str = r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"desktop_version","arguments":{},"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}"#;

struct ServerProcess(Child);

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn server_reports_ready_and_checks_http_authorization_and_headers() {
    let occupied = TcpListener::bind("127.0.0.1:12899").expect("fixed endpoint is available");
    let failure = Command::new(env!("CARGO_BIN_EXE_agent-desktop"))
        .args(["mcp", "--transport", "streamable-http"])
        .output()
        .expect("server process starts");
    assert!(!failure.status.success());
    assert!(
        !failure.stderr.is_empty(),
        "bind failure should explain the startup error"
    );
    drop(occupied);

    let mut process = start_server();
    assert_eq!(process.ready["event"], "ready");
    assert_eq!(process.ready["url"], ENDPOINT);
    assert!(
        process.ready["tools"]
            .as_array()
            .is_some_and(|tools| !tools.is_empty())
    );
    let token = process.ready["token"].as_str().expect("ready token");
    assert_eq!(request(None, None, None, "127.0.0.1:12899", REQUEST), 401);
    assert_eq!(
        request(Some("Bearer stale"), None, None, "127.0.0.1:12899", REQUEST),
        401
    );
    assert_eq!(
        request(
            Some(&format!("Bearer {token}")),
            Some("https://evil.example"),
            None,
            "127.0.0.1:12899",
            REQUEST
        ),
        403
    );
    assert_eq!(
        request(
            Some(&format!("Bearer {token}")),
            None,
            None,
            "evil.example",
            REQUEST
        ),
        403
    );
    let valid = raw_request(
        Some(&format!("Bearer {token}")),
        None,
        None,
        "127.0.0.1:12899",
        Some("2026-07-28"),
        "tools/list",
        REQUEST,
    );
    assert!(valid.starts_with("HTTP/1.1 200"), "{valid}");
    assert!(valid.contains("desktop_snapshot"), "{valid}");
    let call_result = raw_request(
        Some(&format!("Bearer {token}")),
        None,
        None,
        "127.0.0.1:12899",
        Some("2026-07-28"),
        "tools/call",
        CALL_REQUEST,
    );
    assert!(call_result.starts_with("HTTP/1.1 200"), "{call_result}");
    assert!(
        call_result.contains("\"resultType\":\"complete\""),
        "{call_result}"
    );
    let mismatch = raw_request(
        Some(&format!("Bearer {token}")),
        None,
        None,
        "127.0.0.1:12899",
        Some("2026-07-28"),
        "ping",
        REQUEST,
    );
    assert!(mismatch.starts_with("HTTP/1.1 400"), "{mismatch}");
    let missing_version = raw_request(
        Some(&format!("Bearer {token}")),
        None,
        None,
        "127.0.0.1:12899",
        None,
        "tools/list",
        REQUEST,
    );
    assert!(
        missing_version.starts_with("HTTP/1.1 400"),
        "{missing_version}"
    );
    let mismatched_version = raw_request(
        Some(&format!("Bearer {token}")),
        None,
        None,
        "127.0.0.1:12899",
        Some("2025-11-25"),
        "tools/list",
        REQUEST,
    );
    assert!(
        mismatched_version.starts_with("HTTP/1.1 400"),
        "{mismatched_version}"
    );
    let denied_shutdown = shutdown_request(None);
    assert!(
        denied_shutdown.starts_with("HTTP/1.1 401"),
        "{denied_shutdown}"
    );
    let shutdown = shutdown_request(Some(token));
    assert!(shutdown.starts_with("HTTP/1.1 202"), "{shutdown}");
    wait_for_server_exit(&mut process._process.0);
}

struct StartedServer {
    _process: ServerProcess,
    ready: Value,
}

fn start_server() -> StartedServer {
    let mut child = Command::new(env!("CARGO_BIN_EXE_agent-desktop"))
        .args(["mcp", "--transport", "streamable-http"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("server starts");
    let stdout = child.stdout.take().expect("server stdout");
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .expect("ready line");
    StartedServer {
        _process: ServerProcess(child),
        ready: serde_json::from_str(&line).expect("ready event is JSON"),
    }
}

fn request(
    authorization: Option<&str>,
    origin: Option<&str>,
    method_header: Option<&str>,
    host: &str,
    body: &str,
) -> u16 {
    let response = raw_request(
        authorization,
        origin,
        method_header,
        host,
        Some("2026-07-28"),
        "tools/list",
        body,
    );
    response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or_default()
}

fn raw_request(
    authorization: Option<&str>,
    origin: Option<&str>,
    method_header: Option<&str>,
    host: &str,
    version: Option<&str>,
    method: &str,
    body: &str,
) -> String {
    let mut stream =
        TcpStream::connect("127.0.0.1:12899").expect("service accepts loopback connection");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    let mut headers = format!(
        "POST /desktop HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nMcp-Method: {method}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    if let Some(version) = version {
        headers.push_str(&format!("MCP-Protocol-Version: {version}\r\n"));
    }
    if let Some(value) = authorization {
        headers.push_str(&format!("Authorization: {value}\r\n"));
    }
    if let Some(value) = origin {
        headers.push_str(&format!("Origin: {value}\r\n"));
    }
    if let Some(value) = method_header {
        headers.push_str(&format!("Mcp-Method: {value}\r\n"));
    }
    if method == "tools/call" {
        headers.push_str("Mcp-Name: desktop_version\r\n");
    }
    headers.push_str("\r\n");
    stream
        .write_all(headers.as_bytes())
        .expect("request headers written");
    stream
        .write_all(body.as_bytes())
        .expect("request body written");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("response read");
    response
}

fn shutdown_request(token: Option<&str>) -> String {
    let mut stream =
        TcpStream::connect("127.0.0.1:12899").expect("service accepts shutdown request");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("read timeout");
    write!(
        stream,
        "POST /__shutdown HTTP/1.1\r\nHost: 127.0.0.1:12899\r\n"
    )
    .expect("shutdown request written");
    if let Some(token) = token {
        write!(stream, "Authorization: Bearer {token}\r\n").expect("authorization header written");
    }
    stream
        .write_all(b"Content-Length: 0\r\nConnection: close\r\n\r\n")
        .expect("shutdown request finished");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("response read");
    response
}

fn wait_for_server_exit(process: &mut Child) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if process
            .try_wait()
            .expect("server status available")
            .is_some()
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("authenticated shutdown did not exit the server process");
}
