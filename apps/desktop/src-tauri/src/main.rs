#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use agent_desktop_mcp_console::service::{
    ReadyEvent, SHELL_SIDECAR_NAME, ServiceSnapshot, request_shutdown, terminate_process_tree,
    wait_for_port_release,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::{net::SocketAddr, time::Duration};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_shell::{
    ShellExt,
    process::{CommandChild, CommandEvent},
};
use tokio::sync::oneshot;

#[derive(Clone, Default)]
struct DesktopState {
    child: Arc<Mutex<Option<CommandChild>>>,
    snapshot: Arc<Mutex<ServiceSnapshot>>,
    generation: Arc<AtomicU64>,
    transition: Arc<tokio::sync::Mutex<()>>,
}

#[tauri::command]
async fn start_service(
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<ServiceSnapshot, String> {
    let _transition = state.transition.lock().await;
    if state
        .child
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .is_some()
    {
        return Ok(snapshot(&state));
    }
    let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let (mut events, child) = app
        .shell()
        .sidecar(SHELL_SIDECAR_NAME)
        .map_err(|error| error.to_string())?
        .args(["mcp", "--transport", "streamable-http"])
        .spawn()
        .map_err(|error| error.to_string())?;
    *state
        .child
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(child);
    set_snapshot(&app, &state, generation, ServiceSnapshot::starting());
    let state_copy = state.inner().clone();
    let app_copy = app.clone();
    let (ready_sender, ready_receiver) = oneshot::channel();
    tauri::async_runtime::spawn(async move {
        monitor_service(app_copy, state_copy, generation, &mut events, ready_sender).await;
    });
    match tokio::time::timeout(std::time::Duration::from_secs(15), ready_receiver).await {
        Ok(Ok(Ok(ready))) => Ok(ready),
        Ok(Ok(Err(message))) => Err(message),
        Ok(Err(_)) => Err("服务启动监听已关闭。".to_owned()),
        Err(_) => {
            stop_generation(&app, &state, generation);
            let failed =
                ServiceSnapshot::failed("启动服务超时，请确认 12899 端口可用。".to_owned());
            set_snapshot(&app, &state, generation + 1, failed);
            Err("启动服务超时，请确认 12899 端口可用。".to_owned())
        }
    }
}

#[tauri::command]
async fn stop_service(
    app: AppHandle,
    state: State<'_, DesktopState>,
) -> Result<ServiceSnapshot, String> {
    let _transition = state.transition.lock().await;
    let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
    let token = snapshot(&state).token;
    let child = state
        .child
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    if let Some(child) = child {
        let address = service_address();
        let shutdown_sent = if let Some(token) = token {
            tokio::task::spawn_blocking(move || request_shutdown(address, &token))
                .await
                .is_ok_and(|result| result.is_ok())
        } else {
            false
        };
        let released = tokio::task::spawn_blocking(move || {
            wait_for_port_release(address, Duration::from_secs(3))
        })
        .await
        .is_ok_and(|released| released);
        if !shutdown_sent || !released {
            if terminate_process_tree(child.pid()) {
                drop(child);
                let stopped = ServiceSnapshot::stopped();
                set_snapshot(&app, &state, generation, stopped.clone());
                return Ok(stopped);
            }
            let _ = child.kill();
            let message = if shutdown_sent {
                "服务收到停止请求，但端口未能及时释放；已尝试结束服务进程树。"
            } else {
                "无法确认在途工具已取消；已尝试结束服务进程树。"
            }
            .to_owned();
            let failed = ServiceSnapshot::failed(message.clone());
            set_snapshot(&app, &state, generation, failed);
            return Err(message);
        }
        drop(child);
    }
    let stopped = ServiceSnapshot::stopped();
    set_snapshot(&app, &state, generation, stopped.clone());
    Ok(stopped)
}

#[tauri::command]
fn current_service(state: State<'_, DesktopState>) -> ServiceSnapshot {
    snapshot(&state)
}

#[tauri::command]
fn copy_config(app: AppHandle, content: String) -> Result<(), String> {
    app.clipboard()
        .write_text(content)
        .map_err(|error| error.to_string())
}

async fn monitor_service(
    app: AppHandle,
    state: DesktopState,
    generation: u64,
    events: &mut tauri::async_runtime::Receiver<CommandEvent>,
    ready_sender: oneshot::Sender<Result<ServiceSnapshot, String>>,
) {
    let mut ready_sender = Some(ready_sender);
    let mut pending = Vec::new();
    let mut stderr = String::new();
    while let Some(event) = events.recv().await {
        match event {
            CommandEvent::Stdout(bytes) => {
                pending.extend(bytes);
                while let Some(newline) = pending.iter().position(|byte| *byte == b'\n') {
                    let line = pending.drain(..=newline).collect::<Vec<_>>();
                    if ready_sender.is_some() {
                        process_ready_line(&app, &state, generation, &line, &mut ready_sender);
                    }
                }
            }
            CommandEvent::Stderr(bytes) => {
                stderr.push_str(&String::from_utf8_lossy(&bytes));
                if stderr.len() > 2048 {
                    stderr.drain(..stderr.len() - 2048);
                }
            }
            CommandEvent::Error(error) => {
                fail_service(&app, &state, generation, error.clone(), &mut ready_sender);
            }
            CommandEvent::Terminated(payload) => {
                if state.generation.load(Ordering::SeqCst) == generation {
                    let detail = stderr.trim();
                    let message = if detail.is_empty() {
                        format!("服务已退出，退出码：{:?}", payload.code)
                    } else {
                        format!("服务已退出：{detail}")
                    };
                    if let Some(child) = state
                        .child
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .take()
                        && !terminate_process_tree(child.pid())
                    {
                        let _ = child.kill();
                    }
                    fail_service(&app, &state, generation, message, &mut ready_sender);
                }
                return;
            }
            _ => {}
        }
    }
}

fn process_ready_line(
    app: &AppHandle,
    state: &DesktopState,
    generation: u64,
    line: &[u8],
    sender: &mut Option<oneshot::Sender<Result<ServiceSnapshot, String>>>,
) {
    let parsed = serde_json::from_slice::<ReadyEvent>(line)
        .map_err(|error| format!("无法读取服务启动信息：{error}"))
        .and_then(|event| {
            if event.event == "ready" && event.url == "http://127.0.0.1:12899/desktop" {
                Ok(ServiceSnapshot::running(event))
            } else {
                Err("服务返回了无效的启动信息。".to_owned())
            }
        });
    match parsed {
        Ok(snapshot) => {
            set_snapshot(app, state, generation, snapshot.clone());
            if let Some(sender) = sender.take() {
                let _ = sender.send(Ok(snapshot));
            }
        }
        Err(message) => fail_service(app, state, generation, message, sender),
    }
}

fn fail_service(
    app: &AppHandle,
    state: &DesktopState,
    generation: u64,
    message: String,
    sender: &mut Option<oneshot::Sender<Result<ServiceSnapshot, String>>>,
) {
    let failed = ServiceSnapshot::failed(message.clone());
    set_snapshot(app, state, generation, failed);
    if let Some(sender) = sender.take() {
        let _ = sender.send(Err(message));
    }
}

fn stop_generation(app: &AppHandle, state: &DesktopState, generation: u64) {
    state.generation.store(generation + 1, Ordering::SeqCst);
    if let Some(child) = state
        .child
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
    {
        let _ = child.kill();
    }
    let stopped = ServiceSnapshot::stopped();
    set_snapshot(app, state, generation + 1, stopped);
}

fn set_snapshot(app: &AppHandle, state: &DesktopState, generation: u64, value: ServiceSnapshot) {
    if state.generation.load(Ordering::SeqCst) != generation {
        return;
    }
    *state
        .snapshot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = value.clone();
    let _ = app.emit("service-state", value);
}

fn snapshot(state: &DesktopState) -> ServiceSnapshot {
    state
        .snapshot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn service_address() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 12899))
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(DesktopState::default())
        .invoke_handler(tauri::generate_handler![
            start_service,
            stop_service,
            current_service,
            copy_config
        ])
        .build(tauri::generate_context!())?;
    app.run(|app, event| {
        if matches!(event, RunEvent::Exit)
            && let Some(state) = app.try_state::<DesktopState>()
            && let Some(token) = snapshot(&state).token
        {
            let address = service_address();
            let shutdown_sent = request_shutdown(address, &token).is_ok();
            let released = wait_for_port_release(address, Duration::from_secs(3));
            if let Some(child) = state
                .child
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
                && (!shutdown_sent || !released)
                && !terminate_process_tree(child.pid())
            {
                let _ = child.kill();
            }
        } else if matches!(event, RunEvent::Exit)
            && let Some(state) = app.try_state::<DesktopState>()
            && let Some(child) = state
                .child
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            && !terminate_process_tree(child.pid())
        {
            let _ = child.kill();
        }
    });
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("agent-desktop MCP console: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::DesktopState;
    use std::time::Duration;

    #[tokio::test]
    async fn overlapping_service_transitions_wait_for_the_current_transition() {
        let state = DesktopState::default();
        let held = state.transition.lock().await;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let next = state.clone();
        let waiter = tokio::spawn(async move {
            let _guard = next.transition.lock().await;
            let _ = sender.send(());
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(20), receiver)
                .await
                .is_err()
        );
        drop(held);
        waiter.await.expect("transition task completes");
    }
}
