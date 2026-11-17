use super::*;
use serde_json::json;

fn unspawnable() -> Command {
    Command::new("agent-desktop-calls-test-binary-that-does-not-exist")
}

#[test]
fn an_id_is_admitted_once_until_its_call_finishes() {
    let calls = Calls::default();
    assert!(calls.admit(&json!(1)));
    assert!(!calls.admit(&json!(1)));
    assert!(calls.admit(&json!("1")));
    assert!(calls.finish(&json!(1)));
    assert!(calls.admit(&json!(1)));
}

#[test]
fn a_call_cancelled_before_it_starts_never_spawns_and_gets_no_response() {
    let calls = Calls::default();
    assert!(calls.admit(&json!(7)));
    calls.cancel(&json!(7));
    assert!(calls.start(&json!(7), &mut unspawnable()).is_none());
    assert!(!calls.finish(&json!(7)));
}

#[test]
fn a_cancelled_id_stays_taken_until_its_own_call_finishes() {
    let calls = Calls::default();
    assert!(calls.admit(&json!(5)));
    calls.cancel(&json!(5));
    assert!(!calls.admit(&json!(5)));
    assert!(!calls.finish(&json!(5)));
    assert!(calls.admit(&json!(5)));
}

#[cfg(feature = "mcp-streamable-http")]
#[test]
fn shutdown_cancels_every_running_call_and_prevents_a_response() {
    use std::io::Read;

    let calls = Calls::default();
    let first = json!(11);
    let second = json!(12);
    assert!(calls.admit(&first));
    assert!(calls.admit(&second));
    let (first_stdout, _) = calls
        .start(&first, &mut long_running_command())
        .expect("first call admitted")
        .expect("first child starts");
    let (second_stdout, _) = calls
        .start(&second, &mut long_running_command())
        .expect("second call admitted")
        .expect("second child starts");

    calls.cancel_all();
    let cancelled_at = std::time::Instant::now();
    for stdout in [first_stdout, second_stdout] {
        let mut output = Vec::new();
        stdout
            .expect("stdout is piped")
            .read_to_end(&mut output)
            .expect("child exits");
    }
    assert!(cancelled_at.elapsed() < std::time::Duration::from_secs(2));
    assert!(calls.take(&first).is_none());
    assert!(calls.take(&second).is_none());
    assert!(!calls.finish(&first));
    assert!(!calls.finish(&second));
    assert!(!calls.admit(&json!(13)));
}

#[cfg(all(windows, feature = "mcp-streamable-http"))]
#[test]
fn dropping_the_http_child_job_kills_nested_tool_processes() {
    use std::{
        io::{BufRead, BufReader},
        process::Stdio,
        thread,
        time::Duration,
    };

    let calls = Calls::with_child_job().expect("Windows child job is created");
    let id = json!(19);
    assert!(calls.admit(&id));
    let mut command = Command::new("powershell.exe");
    command
        .args([
            "-NoProfile",
            "-Command",
            "$child = Start-Process powershell.exe -ArgumentList '-NoProfile','-Command','Start-Sleep -Seconds 30' -PassThru; Write-Output $child.Id; Start-Sleep -Seconds 30",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(Stdio::null());
    let (stdout, _) = calls
        .start(&id, &mut command)
        .expect("call is admitted")
        .expect("tool process starts");
    let child_pid = BufReader::new(stdout.expect("stdout is piped"))
        .lines()
        .next()
        .expect("child pid line exists")
        .expect("child pid line reads")
        .parse::<u32>()
        .expect("child pid parses");

    drop(calls);
    for _ in 0..100 {
        if !process_is_running(child_pid) {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("a nested tool process survived its job owner");
}

#[cfg(all(windows, feature = "mcp-streamable-http"))]
fn process_is_running(pid: u32) -> bool {
    Command::new("tasklist.exe")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()))
}

#[cfg(unix)]
#[cfg(all(feature = "mcp-streamable-http", unix))]
fn long_running_command() -> Command {
    let mut command = Command::new("sh");
    command
        .args(["-c", "exec sleep 5"])
        .stdout(std::process::Stdio::piped());
    command
}

#[cfg(windows)]
#[cfg(all(feature = "mcp-streamable-http", windows))]
fn long_running_command() -> Command {
    let mut command = Command::new("powershell.exe");
    command
        .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 5"])
        .stdout(std::process::Stdio::piped());
    command
}

#[cfg(unix)]
#[test]
fn a_cancelled_child_gets_sigterm_before_any_kill() {
    use std::io::Read;
    use std::process::Stdio;

    let calls = Calls::default();
    let id = json!(3);
    assert!(calls.admit(&id));
    let mut command = Command::new("sh");
    command
        .args([
            "-c",
            "trap 'echo released; exit 0' TERM; echo started; while :; do sleep 0.05; done",
        ])
        .stdout(Stdio::piped());
    let (stdout, _) = calls
        .start(&id, &mut command)
        .expect("admitted")
        .expect("sh starts");
    let mut stdout = stdout.expect("stdout is piped");
    let mut started = [0_u8; 8];
    stdout.read_exact(&mut started).expect("child started");
    calls.cancel(&id);
    let mut rest = String::new();
    stdout.read_to_string(&mut rest).expect("child exits");
    assert_eq!(rest, "released\n");
}
