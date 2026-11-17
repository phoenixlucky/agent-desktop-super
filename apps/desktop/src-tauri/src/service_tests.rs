use super::{
    ReadyEvent, SHELL_SIDECAR_NAME, ServicePhase, ServiceSnapshot, request_shutdown,
    terminate_process_tree,
};
use std::path::Path;

#[test]
fn ready_event_populates_the_running_snapshot() {
    let event = ReadyEvent {
        event: "ready".to_owned(),
        url: "http://127.0.0.1:12899/desktop".to_owned(),
        token: "fresh-token".to_owned(),
        tools: vec![super::ToolSummary {
            name: "desktop_snapshot".to_owned(),
            description: "Read the UI tree".to_owned(),
        }],
    };
    let state = ServiceSnapshot::running(event);
    assert_eq!(state.phase, ServicePhase::Running);
    assert_eq!(state.url.as_deref(), Some("http://127.0.0.1:12899/desktop"));
    assert_eq!(state.token.as_deref(), Some("fresh-token"));
    assert_eq!(state.tools.len(), 1);
}

#[test]
fn stopped_snapshot_drops_old_credentials_and_tool_data() {
    let state = ServiceSnapshot::stopped();
    assert_eq!(state.phase, ServicePhase::Stopped);
    assert!(state.url.is_none());
    assert!(state.token.is_none());
    assert!(state.tools.is_empty());
}

#[test]
fn failed_snapshot_drops_credentials_and_keeps_the_startup_error() {
    let state = ServiceSnapshot::failed("port is occupied".to_owned());
    assert_eq!(state.phase, ServicePhase::Failed);
    assert!(state.token.is_none());
    assert_eq!(state.message.as_deref(), Some("port is occupied"));
}

#[test]
fn shell_sidecar_selector_matches_the_external_binary_stem() {
    let config: serde_json::Value =
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("Tauri config parses");
    let external_binary = config["bundle"]["externalBin"][0]
        .as_str()
        .expect("external sidecar path");
    let expected = Path::new(external_binary)
        .file_stem()
        .and_then(|name| name.to_str())
        .expect("sidecar command name");
    assert_eq!(SHELL_SIDECAR_NAME, expected);
}

#[test]
fn shutdown_request_sends_the_active_token_and_requires_acceptance() {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        thread,
    };

    let listener = TcpListener::bind("127.0.0.1:0").expect("test listener binds");
    let address = listener.local_addr().expect("test listener address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("shutdown request connects");
        let mut request = String::new();
        let mut reader = BufReader::new(&mut stream);
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("request line reads");
            if line == "\r\n" {
                break;
            }
            request.push_str(&line);
        }
        assert!(request.contains("Authorization: Bearer active-token\r\n"));
        reader
            .get_mut()
            .write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n")
            .expect("acceptance response writes");
    });
    request_shutdown(address, "active-token").expect("server accepted shutdown");
    server.join().expect("test server completes");
}

#[cfg(windows)]
#[test]
fn terminate_process_tree_stops_the_target_process() {
    use std::{process::Command, thread, time::Duration};

    let mut child = Command::new("powershell.exe")
        .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"])
        .spawn()
        .expect("test process starts");
    assert!(terminate_process_tree(child.id()));
    for _ in 0..50 {
        if child
            .try_wait()
            .expect("test process status reads")
            .is_some()
        {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    panic!("target process remained alive after taskkill");
}
