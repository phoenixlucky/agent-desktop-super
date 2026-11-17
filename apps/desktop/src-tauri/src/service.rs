use serde::{Deserialize, Serialize};
use std::{
    io::{self, BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub const SHELL_SIDECAR_NAME: &str = "agent-desktop";

pub fn request_shutdown(address: SocketAddr, token: &str) -> io::Result<()> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(1))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    write!(
        stream,
        "POST /__shutdown HTTP/1.1\r\nHost: 127.0.0.1:12899\r\nAuthorization: Bearer {token}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )?;
    let mut response = String::new();
    BufReader::new(stream).read_line(&mut response)?;
    if response.starts_with("HTTP/1.1 202 ") {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "shutdown request was rejected: {response}"
        )))
    }
}

pub fn wait_for_port_release(address: SocketAddr, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_err() {
            return true;
        }
        thread::sleep(Duration::from_millis(40));
    }
    false
}

pub fn terminate_process_tree(pid: u32) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        let mut command = Command::new("taskkill.exe");
        command
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000);
        command.status().is_ok_and(|status| status.success())
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        false
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSummary {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Deserialize)]
pub struct ReadyEvent {
    pub event: String,
    pub url: String,
    pub token: String,
    pub tools: Vec<ToolSummary>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ServicePhase {
    Starting,
    Running,
    Stopped,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSnapshot {
    pub phase: ServicePhase,
    pub url: Option<String>,
    pub token: Option<String>,
    pub tools: Vec<ToolSummary>,
    pub message: Option<String>,
}

impl Default for ServiceSnapshot {
    fn default() -> Self {
        Self::stopped()
    }
}

impl ServiceSnapshot {
    pub fn starting() -> Self {
        Self {
            phase: ServicePhase::Starting,
            url: None,
            token: None,
            tools: Vec::new(),
            message: None,
        }
    }

    pub fn running(event: ReadyEvent) -> Self {
        Self {
            phase: ServicePhase::Running,
            url: Some(event.url),
            token: Some(event.token),
            tools: event.tools,
            message: None,
        }
    }

    pub fn stopped() -> Self {
        Self {
            phase: ServicePhase::Stopped,
            url: None,
            token: None,
            tools: Vec::new(),
            message: None,
        }
    }

    pub fn failed(message: String) -> Self {
        Self {
            phase: ServicePhase::Failed,
            url: None,
            token: None,
            tools: Vec::new(),
            message: Some(message),
        }
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;
