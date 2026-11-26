//! The local control channel: `status`, `reload`, `run` and `events`.
//!
//! A Unix socket in `$XDG_RUNTIME_DIR/touchpad-snap/` (a named pipe on
//! Windows), newline-delimited JSON, one request per connection. The socket
//! is created before the platform loop starts, so its address is also the
//! "another daemon is already running" check. Requests that only read state
//! (`status`, `subscribe`) are served on the accept thread; `run` and jump
//! requests open the backend directly, exactly like the CLI does.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{Action, Binding, Config, Trigger};
use crate::cycle::{Order, Pick, Step, WorkspaceScope};

/// A request from a client. One per connection, except [`Request::Subscribe`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    /// Version, backend, protocol, bindings and the hotkeys that failed.
    Status,
    /// Re-read the config file now instead of at the next poll.
    Reload,
    /// Run a binding by name, or one action given inline.
    Run {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        binding: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        action: Option<Action>,
    },
    /// The CLI's `next` / `prev` / `next-app` / `prev-app`, with overrides.
    Jump {
        step: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        order: Option<Order>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<WorkspaceScope>,
    },
    /// Stream events as JSON lines until the client disconnects.
    Subscribe,
    /// Claim every touchpad gesture for a moment and return the first one as
    /// a trigger. Times out with an error.
    Record {
        #[serde(default = "default_record_timeout")]
        timeout_ms: u64,
    },
}

fn default_record_timeout() -> u64 {
    15_000
}

/// The answer to a request. `error` is set instead of the payload on failure.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pick: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<Value>,
}

impl Response {
    fn ok() -> Self {
        Self {
            ok: true,
            ..Default::default()
        }
    }

    fn error(message: impl ToString) -> Self {
        Self {
            ok: false,
            error: Some(message.to_string()),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub version: String,
    pub backend: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<u32>,
    pub pid: u32,
    pub uptime_ms: u64,
    pub bindings: Vec<BindingStatus>,
    pub hotkeys_failed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingStatus {
    pub index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub trigger: String,
    pub actions: Vec<String>,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Status {
    pub fn build(
        cfg: &Config,
        platform: &crate::when::PlatformDescription,
        backend: String,
        protocol: Option<u32>,
        started: std::time::Instant,
        hotkeys_failed: Vec<String>,
    ) -> Self {
        let inactive: std::collections::HashMap<usize, String> = cfg
            .inactive_on(platform)
            .into_iter()
            .map(|i| (i.index, i.reason))
            .collect();
        let bindings = cfg
            .bindings
            .iter()
            .map(|binding| BindingStatus {
                index: binding.index,
                name: binding.name.clone(),
                trigger: binding.trigger.label(),
                actions: binding.actions.iter().map(action_label).collect(),
                active: binding.enabled && !inactive.contains_key(&binding.index),
                reason: if binding.enabled {
                    inactive.get(&binding.index).cloned()
                } else {
                    Some("disabled".into())
                },
            })
            .collect();
        Self {
            version: env!("CARGO_PKG_VERSION").into(),
            backend,
            protocol,
            pid: std::process::id(),
            uptime_ms: started.elapsed().as_millis() as u64,
            bindings,
            hotkeys_failed,
        }
    }
}

fn action_label(action: &Action) -> String {
    action.label()
}
struct Context {
    status: Mutex<Status>,
    subscribers: Mutex<Vec<mpsc::Sender<String>>>,
    config_path: PathBuf,
    state_path: PathBuf,
    verbose: bool,
    /// The waiting `Record` client, if any.
    recorder: Mutex<Option<mpsc::Sender<Value>>>,
}

static CONTEXT: OnceLock<Arc<Context>> = OnceLock::new();
static RELOAD: AtomicBool = AtomicBool::new(false);
static RECORDING: AtomicBool = AtomicBool::new(false);

/// Slot for a `Subscribe` client.
fn subscribe(context: &Context) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    context
        .subscribers
        .lock()
        .expect("control subscribers")
        .push(tx);
    rx
}

/// Sends an event to every `events` client. No-op before [`start`].
pub fn publish(event: Value) {
    let Some(context) = CONTEXT.get() else {
        return;
    };
    let text = event.to_string();
    let mut subscribers = context.subscribers.lock().expect("control subscribers");
    subscribers.retain(|tx| tx.send(text.clone()).is_ok());
}

/// Replaces the status after a config reload, keeping the backend identity.
pub fn config_changed(cfg: &Config) {
    let Some(context) = CONTEXT.get() else {
        return;
    };
    let mut status = context.status.lock().expect("control status");
    let started = std::time::Instant::now() - std::time::Duration::from_millis(status.uptime_ms);
    let fresh = Status::build(
        cfg,
        &crate::platform::current_platform(),
        status.backend.clone(),
        status.protocol,
        started,
        status.hotkeys_failed.clone(),
    );
    *status = fresh;
}

/// Records the hotkeys the desktop refused, for `status`.
pub fn set_hotkey_failures(failed: Vec<String>) {
    if let Some(context) = CONTEXT.get() {
        context
            .status
            .lock()
            .expect("control status")
            .hotkeys_failed = failed;
    }
}

/// Whether a client asked for a reload since the last call.
pub fn take_reload() -> bool {
    RELOAD.swap(false, Ordering::SeqCst)
}

/// Whether a `Record` request is waiting for a trigger.
pub fn recording() -> bool {
    RECORDING.load(Ordering::SeqCst)
}

/// Hands a recorded trigger to the waiting client. Returns whether one was
/// waiting (only the first event of a recording is delivered).
pub fn record(event: Value) -> bool {
    let Some(context) = CONTEXT.get() else {
        return false;
    };
    let sender = context.recorder.lock().expect("control recorder").take();
    let Some(sender) = sender else {
        return false;
    };
    sender.send(event).is_ok()
}

/// Starts the control channel for this process.
///
/// Fails with "another touchpad-snap daemon is already running" if the
/// address is taken, which is the daemon's second-instance check.
pub fn start(
    status: Status,
    config_path: PathBuf,
    state_path: PathBuf,
    verbose: bool,
) -> Result<()> {
    let context = Arc::new(Context {
        status: Mutex::new(status),
        subscribers: Mutex::new(Vec::new()),
        config_path,
        state_path,
        verbose,
        recorder: Mutex::new(None),
    });
    CONTEXT
        .set(context.clone())
        .map_err(|_| anyhow::anyhow!("the control channel was already started"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = socket_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // A socket left by a dead daemon is not an owner; a live one is.
        if path.exists() && std::os::unix::net::UnixStream::connect(&path).is_err() {
            let _ = std::fs::remove_file(&path);
        }
        let listener = std::os::unix::net::UnixListener::bind(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AddrInUse {
                anyhow::anyhow!("another touchpad-snap daemon is already running")
            } else {
                anyhow::anyhow!("binding the control socket {}: {e}", path.display())
            }
        })?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let context = context.clone();
                std::thread::spawn(move || serve(stream, &context));
            }
        });
    }
    #[cfg(windows)]
    {
        pipe::listen(context.clone())?;
    }
    Ok(())
}

/// The control socket path (Unix) or pipe name (Windows), for `doctor`.
pub fn address() -> String {
    #[cfg(unix)]
    {
        socket_path().display().to_string()
    }
    #[cfg(windows)]
    {
        pipe::name()
    }
}

#[cfg(unix)]
fn socket_path() -> PathBuf {
    let base = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
    base.join("touchpad-snap").join("control.sock")
}

fn serve<S: Read + Write>(mut stream: S, context: &Context) {
    let request = match read_line(&mut stream) {
        Ok(line) if !line.trim().is_empty() => serde_json::from_str::<Request>(&line),
        Ok(_) => return,
        Err(_) => return,
    };
    let response = match request {
        Ok(Request::Subscribe) => {
            if writeln!(
                stream,
                "{}",
                serde_json::to_string(&Response::ok()).unwrap()
            )
            .is_err()
            {
                return;
            }
            let _ = stream.flush();
            let rx = subscribe(context);
            for event in rx {
                if writeln!(stream, "{event}").is_err() {
                    break;
                }
                if stream.flush().is_err() {
                    break;
                }
            }
            return;
        }
        Ok(request) => handle(request, context),
        Err(e) => Response::error(format!("invalid request: {e}")),
    };
    let _ = writeln!(stream, "{}", serde_json::to_string(&response).unwrap());
    let _ = stream.flush();
}

fn handle(request: Request, context: &Context) -> Response {
    match request {
        Request::Status => Response {
            ok: true,
            status: Some(context.status.lock().expect("control status").clone()),
            ..Default::default()
        },
        Request::Reload => {
            RELOAD.store(true, Ordering::SeqCst);
            Response {
                message: Some("reload requested; the daemon picks it up within 2 s".into()),
                ..Response::ok()
            }
        }
        Request::Jump {
            step,
            order,
            workspace,
        } => run_jump(context, &step, order, workspace),
        Request::Run { binding, action } => run_binding(context, binding, action),
        Request::Record { timeout_ms } => record_trigger(context, timeout_ms),
        Request::Subscribe => Response::error("subscribe needs a streaming connection"),
    }
}

fn record_trigger(context: &Context, timeout_ms: u64) -> Response {
    let result = (|| -> Result<Value> {
        let backend = crate::platform::connect()?;
        let (tx, rx) = mpsc::channel();
        *context.recorder.lock().expect("control recorder") = Some(tx);
        RECORDING.store(true, Ordering::SeqCst);
        if let Err(e) = backend.set_recording(true) {
            RECORDING.store(false, Ordering::SeqCst);
            *context.recorder.lock().expect("control recorder") = None;
            return Err(e);
        }
        let outcome = rx.recv_timeout(std::time::Duration::from_millis(timeout_ms));
        RECORDING.store(false, Ordering::SeqCst);
        *context.recorder.lock().expect("control recorder") = None;
        let stopped = backend.set_recording(false);
        if let Err(e) = stopped {
            eprintln!("touchpad-snap: could not stop recording: {e:#}");
        }
        match outcome {
            Ok(event) => Ok(event),
            Err(_) => anyhow::bail!(
                "no gesture within {} s (is the daemon running and the touchpad active?)",
                timeout_ms / 1000
            ),
        }
    })();
    match result {
        Ok(event) => Response {
            event: Some(event),
            ..Response::ok()
        },
        Err(e) => Response::error(format!("{e:#}")),
    }
}

fn run_jump(
    context: &Context,
    step: &str,
    order: Option<Order>,
    workspace: Option<WorkspaceScope>,
) -> Response {
    let result = (|| -> Result<Pick> {
        let mut cfg = crate::config::load(&context.config_path)?;
        if let Some(order) = order {
            cfg.cycle.order = order;
        }
        if let Some(workspace) = workspace {
            cfg.cycle.workspace = workspace;
        }
        let backend = crate::platform::connect()?;
        match step {
            "next" => crate::daemon::jump(
                backend.as_ref(),
                &cfg,
                Step::Next,
                crate::cycle::Session::ByTime,
                &context.state_path,
            ),
            "prev" => crate::daemon::jump(
                backend.as_ref(),
                &cfg,
                Step::Prev,
                crate::cycle::Session::ByTime,
                &context.state_path,
            ),
            "next-app" => crate::daemon::jump_app(
                backend.as_ref(),
                &cfg,
                Step::Next,
                crate::cycle::Session::ByTime,
                &context.state_path,
            ),
            "prev-app" => crate::daemon::jump_app(
                backend.as_ref(),
                &cfg,
                Step::Prev,
                crate::cycle::Session::ByTime,
                &context.state_path,
            ),
            other => anyhow::bail!("unknown step `{other}`"),
        }
    })();
    match result {
        Ok(Pick::NoFocus) => Response::error("no window is focused"),
        Ok(pick) => Response {
            pick: Some(format!("{pick:?}")),
            ..Response::ok()
        },
        Err(e) => Response::error(format!("{e:#}")),
    }
}

fn run_binding(context: &Context, binding: Option<String>, action: Option<Action>) -> Response {
    let result = (|| -> Result<String> {
        let cfg = crate::config::load(&context.config_path)?;
        let (actions, trigger, name) = match (binding, action) {
            (_, Some(action)) => (
                vec![action],
                Trigger::Hotkey {
                    keys: String::new(),
                },
                None,
            ),
            (Some(name), None) => {
                let binding = find_binding(&cfg, &name)?;
                (binding.actions.clone(), binding.trigger.clone(), Some(name))
            }
            (None, None) => anyhow::bail!("give a binding name or an action"),
        };
        let backend = crate::platform::connect()?;
        let snapshot = backend.snapshot().ok();
        crate::daemon::run_actions(
            backend.as_ref(),
            &cfg,
            &actions,
            &trigger,
            crate::cycle::Session::ByTime,
            snapshot.as_ref(),
            crate::hotkey::Mods::default(),
            &context.state_path,
            context.verbose,
        );
        Ok(name.unwrap_or_else(|| {
            actions
                .iter()
                .map(action_label)
                .collect::<Vec<_>>()
                .join(" + ")
        }))
    })();
    match result {
        Ok(name) => Response {
            message: Some(format!("ran {name}")),
            ..Response::ok()
        },
        Err(e) => Response::error(format!("{e:#}")),
    }
}

fn find_binding<'a>(cfg: &'a Config, name: &str) -> Result<&'a Binding> {
    cfg.bindings
        .iter()
        .find(|b| b.name.as_deref() == Some(name))
        .ok_or_else(|| anyhow::anyhow!("no binding named `{name}`"))
}

fn read_line<S: Read>(stream: &mut S) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                bytes.push(byte[0]);
                if bytes.len() > 1024 * 1024 {
                    return Err(std::io::Error::other("request too large"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    String::from_utf8(bytes)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "request is not UTF-8"))
}

/// Sends one request, returning `None` when no daemon is listening.
pub fn try_request(request: &Request) -> Option<std::io::Result<Response>> {
    #[cfg(unix)]
    {
        let stream = match std::os::unix::net::UnixStream::connect(socket_path()) {
            Ok(stream) => stream,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                return None;
            }
            Err(e) => return Some(Err(e)),
        };
        Some(exchange(stream, request))
    }
    #[cfg(windows)]
    {
        pipe::connect(request)
    }
}

fn exchange<S: Read + Write>(mut stream: S, request: &Request) -> std::io::Result<Response> {
    writeln!(stream, "{}", serde_json::to_string(request).unwrap())?;
    stream.flush()?;
    let line = read_line(&mut stream)?;
    serde_json::from_str(&line).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Sends a request and fails if the daemon says no.
pub fn request(request: &Request) -> Result<Response> {
    match try_request(request) {
        Some(Ok(response)) if response.ok => Ok(response),
        Some(Ok(response)) => anyhow::bail!(
            "{}",
            response
                .error
                .unwrap_or_else(|| "the daemon refused".into())
        ),
        Some(Err(e)) => Err(Into::into(e)),
        None => anyhow::bail!("the daemon is not running"),
    }
}

/// For `touchpad-snap events`: a stream of events as JSON lines.
pub fn events<F: FnMut(&str) -> bool>(mut on_event: F) -> Result<()> {
    let stream = connect()?;
    let mut stream = stream;
    writeln!(stream, "{}", serde_json::json!({"cmd": "subscribe"}))?;
    stream.flush()?;
    let first = read_line(&mut stream)?;
    let response: Response = serde_json::from_str(&first)?;
    if !response.ok {
        anyhow::bail!(
            "{}",
            response
                .error
                .unwrap_or_else(|| "the daemon refused".into())
        );
    }
    loop {
        let line = read_line(&mut stream)?;
        if line.is_empty() {
            return Ok(());
        }
        if !on_event(&line) {
            return Ok(());
        }
    }
}

#[cfg(unix)]
fn connect() -> Result<std::os::unix::net::UnixStream> {
    let path = socket_path();
    match std::os::unix::net::UnixStream::connect(&path) {
        Ok(stream) => Ok(stream),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!(
                "the daemon is not running (no {}; start it with `touchpad-snap daemon`)",
                path.display()
            )
        }
        Err(e) => Err(Into::into(e)),
    }
}

#[cfg(windows)]
fn connect() -> Result<pipe::Stream> {
    pipe::connect_stream()
}

#[cfg(windows)]
mod pipe {
    use super::*;
    use std::fs::File;
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
        PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };

    pub type Stream = File;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn name() -> String {
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
        let user: String = user
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        format!(r"\\.\pipe\touchpad-snap-{user}")
    }

    pub fn listen(context: Arc<Context>) -> Result<()> {
        let name = wide(&name());
        std::thread::spawn(move || {
            loop {
                let handle = unsafe {
                    CreateNamedPipeW(
                        name.as_ptr(),
                        PIPE_ACCESS_DUPLEX,
                        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                        PIPE_UNLIMITED_INSTANCES,
                        64 * 1024,
                        64 * 1024,
                        0,
                        std::ptr::null(),
                    )
                };
                if handle == INVALID_HANDLE_VALUE {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    continue;
                }
                unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
                let file = unsafe { File::from_raw_handle(handle as _) };
                let context = context.clone();
                std::thread::spawn(move || serve(file, &context));
            }
        });
        Ok(())
    }

    pub fn connect_stream() -> Result<Stream> {
        let handle = unsafe {
            CreateFileW(
                wide(&name()).as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            let code = std::io::Error::last_os_error();
            anyhow::bail!("the daemon is not running ({code})");
        }
        Ok(unsafe { File::from_raw_handle(handle as _) })
    }

    pub fn connect(request: &Request) -> Option<std::io::Result<Response>> {
        let handle = unsafe {
            CreateFileW(
                wide(&name()).as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            let error = std::io::Error::last_os_error();
            if matches!(
                error.raw_os_error(),
                Some(code) if code == ERROR_FILE_NOT_FOUND as i32 || code == ERROR_PIPE_BUSY as i32
            ) {
                return None;
            }
            return Some(Err(error));
        }
        let file = unsafe { File::from_raw_handle(handle as _) };
        Some(exchange(file, request))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip() {
        let request = Request::Run {
            binding: Some("windows".into()),
            action: None,
        };
        let text = serde_json::to_string(&request).unwrap();
        assert_eq!(text, r#"{"cmd":"run","binding":"windows"}"#);
        assert!(matches!(
            serde_json::from_str::<Request>(&text).unwrap(),
            Request::Run { .. }
        ));

        let jump: Request =
            serde_json::from_str(r#"{"cmd":"jump","step":"next-app","order":"mru"}"#).unwrap();
        assert!(matches!(jump, Request::Jump { .. }));

        let record: Request = serde_json::from_str(r#"{"cmd":"record"}"#).unwrap();
        assert!(matches!(record, Request::Record { timeout_ms: 15_000 }));
    }

    #[test]
    fn a_response_marks_failure() {
        let response = Response::error("nope");
        assert!(!response.ok);
        assert_eq!(response.error.as_deref(), Some("nope"));
    }

    #[test]
    fn status_lists_active_and_inactive_bindings() {
        let mut cfg = Config::default();
        cfg.bindings[0].name = Some("default".into());
        cfg.bindings.push(Binding {
            name: Some("mac only".into()),
            enabled: true,
            trigger: Trigger::Hotkey {
                keys: "Ctrl+Alt+M".into(),
            },
            when: Some(crate::when::When {
                os: vec!["macos".into()],
                ..Default::default()
            }),
            repeat: None,
            actions: vec![Action::Next],
            index: cfg.bindings.len(),
        });
        let status = Status::build(
            &cfg,
            &crate::when::PlatformDescription::new("linux", Some("gnome")),
            "test".into(),
            Some(5),
            std::time::Instant::now(),
            vec![],
        );
        assert!(status.bindings[0].active);
        assert!(!status.bindings.last().unwrap().active);
        assert_eq!(status.protocol, Some(5));
    }
}
