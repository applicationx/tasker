use super::api::{self, ServerContext};
use super::state::{ADDRESS, RuntimePaths, RuntimeState, SERVER_VERSION, root_fingerprint};
use super::watch::{ChangeSignal, start as start_watcher};
use crate::error::{AppError, ErrorCategory, Result};
use crate::service::snapshot::canonical_projects_root;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const WORKERS: usize = 6;
const INGRESS_QUEUE: usize = 8;
const SSE_CLIENTS: usize = 4;
const MAX_REQUEST_LINE: usize = 2 * 1024;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_HEADERS: usize = 64;
pub(crate) const MAX_BODY: usize = 2 * 1024 * 1024;
const HEADER_DEADLINE: Duration = Duration::from_millis(750);
const BODY_DEADLINE: Duration = Duration::from_secs(3);
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'; object-src 'none'";

pub struct HttpRequest {
    stream: TcpStream,
    peer: SocketAddr,
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpRequest {
    pub fn peer(&self) -> SocketAddr {
        self.peer
    }

    pub fn method(&self) -> &str {
        &self.method
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub fn header_values(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
            .collect()
    }

    pub fn respond(self, status: u16, headers: &[(&str, &str)], body: &[u8]) {
        let mut stream = self.stream;
        let _ = write_response(&mut stream, status, headers, body);
    }

    fn into_stream(self) -> TcpStream {
        self.stream
    }
}

struct SseJob {
    request: HttpRequest,
    changes: Arc<ChangeSignal>,
    shutdown: Arc<AtomicBool>,
    since: u64,
}

pub struct SsePool {
    sender: SyncSender<SseJob>,
    active: Arc<AtomicUsize>,
}

impl SsePool {
    fn start() -> Result<(Arc<Self>, Vec<thread::JoinHandle<()>>)> {
        let (sender, receiver) = mpsc::sync_channel::<SseJob>(SSE_CLIENTS);
        let receiver = Arc::new(Mutex::new(receiver));
        let active = Arc::new(AtomicUsize::new(0));
        let mut workers = Vec::new();
        for index in 0..SSE_CLIENTS {
            let receiver = Arc::clone(&receiver);
            let active = Arc::clone(&active);
            match thread::Builder::new()
                .name(format!("tasker-ui-sse-{index}"))
                .spawn(move || {
                    loop {
                        let job = receiver
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner())
                            .recv();
                        let Ok(job) = job else { break };
                        api::serve_sse(job.request, job.changes, job.shutdown, job.since);
                        active.fetch_sub(1, Ordering::AcqRel);
                    }
                }) {
                Ok(worker) => workers.push(worker),
                Err(error) => {
                    drop(sender);
                    for worker in workers {
                        let _ = worker.join();
                    }
                    return Err(AppError::new(
                        "ui_start_failed",
                        format!("Cannot start fixed UI change-stream worker: {error}"),
                        ErrorCategory::Io,
                    ));
                }
            }
        }
        Ok((Arc::new(Self { sender, active }), workers))
    }

    pub fn dispatch(
        &self,
        request: HttpRequest,
        changes: Arc<ChangeSignal>,
        shutdown: Arc<AtomicBool>,
        since: u64,
    ) -> std::result::Result<(), Box<HttpRequest>> {
        let mut current = self.active.load(Ordering::Acquire);
        loop {
            if current >= SSE_CLIENTS {
                return Err(Box::new(request));
            }
            match self.active.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(next) => current = next,
            }
        }
        let job = SseJob {
            request,
            changes,
            shutdown,
            since,
        };
        match self.sender.try_send(job) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(job) | TrySendError::Disconnected(job)) => {
                self.active.fetch_sub(1, Ordering::AcqRel);
                Err(Box::new(job.request))
            }
        }
    }
}

pub fn serve(projects_root: &Path, launch_id: &str) -> Result<()> {
    if !projects_root.is_absolute() || Uuid::parse_str(launch_id).is_err() {
        return Err(AppError::input(
            "Internal UI serve requires an absolute canonical root and UUID launch ID",
        ));
    }
    let root = canonical_projects_root(projects_root)?;
    if root != projects_root {
        return Err(AppError::new(
            "ui_root_unavailable",
            "Internal UI projects root must already be canonical",
            ErrorCategory::Project,
        ));
    }
    let root_text = root.to_str().ok_or_else(|| {
        AppError::new(
            "ui_root_unavailable",
            "The canonical projects root cannot be represented as UTF-8 for display",
            ErrorCategory::Project,
        )
    })?;
    let paths = RuntimePaths::prepare(&root)?;
    let instance_lock = paths.try_lock_instance()?.ok_or_else(|| {
        AppError::new(
            "ui_instance_running",
            "A Tasker UI child already holds this projects-root instance lock",
            ErrorCategory::Conflict,
        )
    })?;
    let listener = TcpListener::bind((ADDRESS, 0)).map_err(|error| {
        AppError::new(
            "ui_start_failed",
            format!("Cannot bind numeric IPv4 loopback: {error}"),
            ErrorCategory::Io,
        )
    })?;
    listener.set_nonblocking(true).map_err(|error| {
        AppError::new(
            "ui_start_failed",
            format!("Cannot bound UI listener ingress: {error}"),
            ErrorCategory::Io,
        )
    })?;
    let local = listener.local_addr().map_err(|error| {
        AppError::new(
            "ui_start_failed",
            format!("Cannot inspect UI listener address: {error}"),
            ErrorCategory::Io,
        )
    })?;
    if local.ip().to_string() != ADDRESS || local.port() == 0 {
        return Err(AppError::new(
            "ui_start_failed",
            "UI listener did not bind the required numeric loopback address",
            ErrorCategory::Io,
        ));
    }
    let shutdown = Arc::new(AtomicBool::new(false));
    let changes = Arc::new(ChangeSignal::new());
    let (watcher, warnings) = start_watcher(&root, Arc::clone(&changes), Arc::clone(&shutdown))?;
    let mut token = [0u8; 32];
    getrandom::fill(&mut token).map_err(|error| {
        AppError::new(
            "ui_start_failed",
            format!("Cannot generate UI instance token: {error}"),
            ErrorCategory::Io,
        )
    })?;
    let instance_id = Uuid::new_v4().to_string();
    let state = RuntimeState {
        schema_version: 1,
        pid: std::process::id(),
        address: ADDRESS.to_string(),
        port: local.port(),
        url: format!("http://{ADDRESS}:{}/", local.port()),
        projects_root: root_text.to_string(),
        root_fingerprint: root_fingerprint(&root),
        started_at: crate::app::now(),
        server_version: SERVER_VERSION.to_string(),
        instance_id: instance_id.clone(),
        launch_id: launch_id.to_string(),
        executable: std::env::current_exe()
            .ok()
            .and_then(|path| path.to_str().map(str::to_string))
            .unwrap_or_default(),
        watch_mode: watcher.mode().to_string(),
        warnings,
        token: hex(&token),
    };
    let context = Arc::new(ServerContext {
        root: root.clone(),
        state,
        changes,
        shutdown: Arc::clone(&shutdown),
    });
    let (sse_pool, sse_workers) = SsePool::start()?;
    let (sender, receiver) = mpsc::sync_channel::<(TcpStream, SocketAddr)>(INGRESS_QUEUE);
    let receiver = Arc::new(Mutex::new(receiver));
    let mut workers = Vec::new();
    for index in 0..WORKERS {
        let receiver = Arc::clone(&receiver);
        let context = Arc::clone(&context);
        let sse_pool = Arc::clone(&sse_pool);
        let worker = thread::Builder::new()
            .name(format!("tasker-ui-http-{index}"))
            .spawn(move || ingress_worker(receiver, context, sse_pool))
            .map_err(|error| {
                AppError::new(
                    "ui_start_failed",
                    format!("Cannot start bounded UI HTTP worker: {error}"),
                    ErrorCategory::Io,
                )
            })?;
        workers.push(worker);
    }
    if let Err(error) = paths.write_state(&context.state) {
        shutdown.store(true, Ordering::Release);
        context.changes.wake_all();
        drop(sender);
        for worker in workers {
            let _ = worker.join();
        }
        drop(sse_pool);
        for worker in sse_workers {
            let _ = worker.join();
        }
        drop(watcher);
        drop(instance_lock);
        return Err(error);
    }

    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, peer)) => {
                if !peer.ip().is_loopback() {
                    reject(&mut stream, 403, "Only loopback peers are accepted");
                    continue;
                }
                let _ = stream.set_nodelay(true);
                match sender.try_send((stream, peer)) {
                    Ok(()) => {}
                    Err(TrySendError::Full((mut stream, _))) => {
                        let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
                        reject(
                            &mut stream,
                            503,
                            "HTTP ingress capacity is temporarily full",
                        );
                    }
                    Err(TrySendError::Disconnected(_)) => break,
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) => {
                shutdown.store(true, Ordering::Release);
                context.changes.wake_all();
            }
        }
    }

    drop(sender);
    for worker in workers {
        let _ = worker.join();
    }
    drop(sse_pool);
    for worker in sse_workers {
        let _ = worker.join();
    }
    drop(watcher);
    paths.remove_state_if_instance(&instance_id);
    drop(instance_lock);
    Ok(())
}

fn ingress_worker(
    receiver: Arc<Mutex<mpsc::Receiver<(TcpStream, SocketAddr)>>>,
    context: Arc<ServerContext>,
    sse_pool: Arc<SsePool>,
) {
    loop {
        let accepted = receiver
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .recv();
        let Ok((stream, peer)) = accepted else { break };
        match parse_request(stream, peer) {
            Ok(request) => api::handle(request, &context, &sse_pool),
            Err(mut failure) => reject(&mut failure.stream, failure.status, failure.message),
        }
    }
}

struct ParseFailure {
    stream: TcpStream,
    status: u16,
    message: &'static str,
}

fn parse_request(
    mut stream: TcpStream,
    peer: SocketAddr,
) -> std::result::Result<HttpRequest, ParseFailure> {
    let fail = |stream, status, message| ParseFailure {
        stream,
        status,
        message,
    };
    if stream.set_read_timeout(Some(HEADER_DEADLINE)).is_err()
        || stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err()
    {
        return Err(fail(stream, 500, "Cannot set socket deadlines"));
    }
    let mut raw = [0u8; MAX_HEADER_BYTES];
    let mut used = 0;
    let header_deadline = Instant::now() + HEADER_DEADLINE;
    let header_end = loop {
        if let Some(offset) = raw[..used]
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
        {
            break offset + 4;
        }
        if used == raw.len() {
            return Err(fail(stream, 431, "Request headers exceed 16 KiB"));
        }
        let remaining = header_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(fail(stream, 408, "Timed out reading HTTP request headers"));
        }
        if stream.set_read_timeout(Some(remaining)).is_err() {
            return Err(fail(stream, 500, "Cannot enforce HTTP header deadline"));
        }
        match stream.read(&mut raw[used..]) {
            Ok(0) => return Err(fail(stream, 400, "Incomplete HTTP request headers")),
            Ok(count) => used += count,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return Err(fail(stream, 408, "Timed out reading HTTP request headers"));
            }
            Err(_) => return Err(fail(stream, 400, "Cannot read HTTP request headers")),
        }
    };
    let Some(request_line) = raw[..header_end]
        .windows(2)
        .position(|window| window == b"\r\n")
    else {
        return Err(fail(stream, 400, "Invalid HTTP request line"));
    };
    if request_line > MAX_REQUEST_LINE {
        return Err(fail(stream, 414, "HTTP request line exceeds 2 KiB"));
    }
    let mut parsed_headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut parsed = httparse::Request::new(&mut parsed_headers);
    match parsed.parse(&raw[..header_end]) {
        Ok(httparse::Status::Complete(_)) => {}
        Err(httparse::Error::TooManyHeaders) => {
            return Err(fail(stream, 431, "HTTP request has too many headers"));
        }
        _ => return Err(fail(stream, 400, "Invalid HTTP request headers")),
    }
    if parsed.version != Some(1) {
        return Err(fail(stream, 400, "Only HTTP/1.1 requests are accepted"));
    }
    let method = parsed.method.unwrap_or_default().to_string();
    let url = parsed.path.unwrap_or_default().to_string();
    if method.is_empty() || !url.starts_with('/') {
        return Err(fail(stream, 400, "Invalid HTTP request target"));
    }
    let mut headers = Vec::with_capacity(parsed.headers.len());
    for header in parsed.headers {
        let Ok(value) = std::str::from_utf8(header.value) else {
            return Err(fail(stream, 400, "HTTP header values must be UTF-8"));
        };
        headers.push((header.name.to_string(), value.trim().to_string()));
    }
    if headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("Transfer-Encoding"))
    {
        return Err(fail(stream, 400, "Transfer-Encoding is not accepted"));
    }
    let lengths = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("Content-Length"))
        .map(|(_, value)| value)
        .collect::<Vec<_>>();
    if lengths.len() > 1 {
        return Err(fail(stream, 400, "Content-Length must appear at most once"));
    }
    let body_length = match lengths.first() {
        Some(value) => match value.parse::<usize>() {
            Ok(value) => value,
            Err(_) => return Err(fail(stream, 400, "Content-Length is invalid")),
        },
        None => 0,
    };
    if body_length > MAX_BODY {
        return Err(fail(stream, 413, "Request body exceeds 2 MiB"));
    }
    let mut body = Vec::new();
    if body.try_reserve_exact(body_length).is_err() {
        return Err(fail(stream, 413, "Request body cannot be allocated"));
    }
    let available = used.saturating_sub(header_end).min(body_length);
    body.extend_from_slice(&raw[header_end..header_end + available]);
    let body_deadline = Instant::now() + BODY_DEADLINE;
    while body.len() < body_length {
        let deadline_remaining = body_deadline.saturating_duration_since(Instant::now());
        if deadline_remaining.is_zero() {
            return Err(fail(stream, 408, "Timed out reading HTTP request body"));
        }
        if stream.set_read_timeout(Some(deadline_remaining)).is_err() {
            return Err(fail(stream, 500, "Cannot enforce HTTP body deadline"));
        }
        let remaining = body_length - body.len();
        let mut chunk = [0u8; 8192];
        let read_length = remaining.min(chunk.len());
        match stream.read(&mut chunk[..read_length]) {
            Ok(0) => return Err(fail(stream, 400, "Incomplete HTTP request body")),
            Ok(count) => body.extend_from_slice(&chunk[..count]),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return Err(fail(stream, 408, "Timed out reading HTTP request body"));
            }
            Err(_) => return Err(fail(stream, 400, "Cannot read HTTP request body")),
        }
    }
    Ok(HttpRequest {
        stream,
        peer,
        method,
        url,
        headers,
        body,
    })
}

pub(crate) fn write_streaming_head(
    stream: &mut TcpStream,
    status: u16,
    headers: &[(&str, &str)],
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} {}\r\nConnection: close\r\nContent-Security-Policy: {CSP}\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nCross-Origin-Resource-Policy: same-origin\r\nX-Frame-Options: DENY\r\n",
        reason(status)
    )?;
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    stream.flush()
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    headers: &[(&str, &str)],
    body: &[u8],
) -> std::io::Result<()> {
    let mut response = format!(
        "HTTP/1.1 {status} {}\r\nConnection: close\r\nContent-Length: {}\r\nContent-Security-Policy: {CSP}\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nCross-Origin-Resource-Policy: same-origin\r\nX-Frame-Options: DENY\r\n",
        reason(status),
        body.len()
    )
    .into_bytes();
    for (name, value) in headers {
        response.extend_from_slice(name.as_bytes());
        response.extend_from_slice(b": ");
        response.extend_from_slice(value.as_bytes());
        response.extend_from_slice(b"\r\n");
    }
    response.extend_from_slice(b"\r\n");
    response.extend_from_slice(body);
    stream.write_all(&response)
}

fn reject(stream: &mut TcpStream, status: u16, message: &str) {
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let _ = write_response(
        stream,
        status,
        &[
            ("Content-Type", "text/plain; charset=utf-8"),
            ("Cache-Control", "no-store"),
        ],
        message.as_bytes(),
    );
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Payload Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

pub(crate) fn take_stream(request: HttpRequest) -> TcpStream {
    request.into_stream()
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}
