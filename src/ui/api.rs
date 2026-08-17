use super::assets;
use super::server::{HttpRequest, MAX_BODY, SsePool, take_stream, write_streaming_head};
use super::state::RuntimeState;
use super::watch::ChangeSignal;
use crate::error::{AppError, ErrorCategory};
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub struct ServerContext {
    pub root: PathBuf,
    pub state: RuntimeState,
    pub changes: Arc<ChangeSignal>,
    pub shutdown: Arc<AtomicBool>,
}

pub fn handle(request: HttpRequest, context: &ServerContext, sse_pool: &SsePool) {
    if !request.peer().ip().is_loopback() {
        respond_error(
            request,
            403,
            AppError::new(
                "ui_peer_rejected",
                "Only loopback peers are accepted",
                ErrorCategory::Input,
            ),
        );
        return;
    }
    let expected_host = format!("127.0.0.1:{}", context.state.port);
    if !header_exact(&request, "Host", &expected_host) {
        respond_error(
            request,
            400,
            AppError::input("Host header does not match the bound numeric loopback address"),
        );
        return;
    }
    let url = request.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((&url, ""));
    if path.contains('%') || path.contains('\\') || path.contains("..") || path.contains('\0') {
        respond_error(request, 400, AppError::input("Invalid request path"));
        return;
    }
    if path == "/__health" {
        if request.method() != "GET" {
            respond_error(request, 405, AppError::input("Health requires GET"));
        } else if !valid_token(&request, &context.state.token) {
            respond_error(request, 403, AppError::input("Invalid UI instance token"));
        } else {
            respond_ok(
                request,
                200,
                json!({
                    "instance_id":context.state.instance_id,
                    "launch_id":context.state.launch_id,
                    "root_fingerprint":context.state.root_fingerprint,
                    "server_version":context.state.server_version
                }),
            );
        }
        return;
    }
    if path == "/__shutdown" {
        if request.method() != "POST" {
            respond_error(request, 405, AppError::input("Shutdown requires POST"));
        } else if !valid_token(&request, &context.state.token) {
            respond_error(request, 403, AppError::input("Invalid UI instance token"));
        } else {
            context.shutdown.store(true, Ordering::Release);
            context.changes.wake_all();
            respond_ok(request, 200, json!({"shutting_down":true}));
        }
        return;
    }
    if path.starts_with("/api/") {
        handle_api(request, context, sse_pool, path, query);
        return;
    }
    if request.method() != "GET" {
        respond_error(request, 405, AppError::input("Static assets require GET"));
        return;
    }
    match assets::get(path) {
        Some(asset) => respond_asset(request, asset),
        None => respond_error(
            request,
            404,
            AppError::new(
                "ui_route_not_found",
                "UI route does not exist",
                ErrorCategory::NotFound,
            ),
        ),
    }
}

fn handle_api(
    request: HttpRequest,
    context: &ServerContext,
    sse_pool: &SsePool,
    path: &str,
    query: &str,
) {
    if context.shutdown.load(Ordering::Acquire) {
        respond_error(
            request,
            503,
            AppError::new(
                "ui_shutting_down",
                "Tasker UI is shutting down",
                ErrorCategory::Conflict,
            ),
        );
        return;
    }
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    if segments.as_slice() == ["api", "v1", "session"] {
        if request.method() != "GET" {
            respond_error(request, 405, AppError::input("Session requires GET"));
        } else {
            respond_ok(
                request,
                200,
                json!({
                    "token":context.state.token,
                    "instance_id":context.state.instance_id,
                    "server_version":context.state.server_version,
                    "change_generation":context.changes.generation()
                }),
            );
        }
        return;
    }
    if segments.as_slice() == ["api", "v1", "changes"] {
        if request.method() != "GET" {
            respond_error(request, 405, AppError::input("Changes requires GET"));
        } else if !valid_token(&request, &context.state.token) {
            respond_error(request, 403, AppError::input("Invalid UI session token"));
        } else {
            let since = query_value(query, "since")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            if let Err(request) = sse_pool.dispatch(
                request,
                Arc::clone(&context.changes),
                Arc::clone(&context.shutdown),
                since,
            ) {
                respond_error(
                    *request,
                    429,
                    AppError::new(
                        "ui_change_stream_limit",
                        "The maximum number of UI change streams is already open",
                        ErrorCategory::Conflict,
                    ),
                );
            }
        }
        return;
    }
    if segments.as_slice() == ["api", "v1", "projects"] {
        if request.method() != "GET" {
            respond_error(
                request,
                405,
                AppError::input("Project listing requires GET"),
            );
        } else {
            respond_result(request, crate::service::list_projects(&context.root));
        }
        return;
    }
    if segments.len() < 5 || segments[..3] != ["api", "v1", "projects"] {
        respond_error(
            request,
            404,
            AppError::new(
                "ui_route_not_found",
                "API route does not exist",
                ErrorCategory::NotFound,
            ),
        );
        return;
    }
    let prefix = segments[3];
    match segments[4..].as_ref() {
        ["snapshot"] => get_only(request, || {
            crate::service::project_snapshot(&context.root, prefix, context.changes.generation())
        }),
        ["tasks", task_id] => get_only(request, || {
            crate::service::read_task(&context.root, prefix, task_id)
        }),
        ["resources", resource_id] => get_only(request, || {
            crate::service::read_resource(&context.root, prefix, resource_id)
        }),
        ["decisions", decision_id] => get_only(request, || {
            crate::service::read_decision(&context.root, prefix, decision_id)
        }),
        ["project-brief"] => get_only(request, || {
            crate::service::project_brief(&context.root, prefix)
        }),
        ["brief"] => {
            let task_id = query_value(query, "task_id").filter(|value| !value.is_empty());
            let max_bytes = query_value(query, "max_bytes")
                .and_then(|value| value.parse().ok())
                .unwrap_or(65_536);
            let format = match query_value(query, "format").as_deref() {
                None | Some("markdown") => crate::cli::OutputFormat::Markdown,
                Some("json") => crate::cli::OutputFormat::Json,
                Some("yaml") => crate::cli::OutputFormat::Yaml,
                Some(_) => {
                    respond_error(
                        request,
                        400,
                        AppError::input("Brief format must be markdown, json, or yaml"),
                    );
                    return;
                }
            };
            get_only(request, || {
                crate::service::brief(&context.root, prefix, task_id, max_bytes, format)
            });
        }
        ["events"] => {
            let offset = query_value(query, "offset")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            let limit = query_value(query, "limit")
                .and_then(|value| value.parse().ok())
                .unwrap_or(50);
            get_only(request, || {
                crate::service::events(&context.root, prefix, offset, limit)
            });
        }
        ["mutations"] => mutate(request, context, prefix),
        _ => respond_error(
            request,
            404,
            AppError::new(
                "ui_route_not_found",
                "API route does not exist",
                ErrorCategory::NotFound,
            ),
        ),
    }
}

fn mutate(request: HttpRequest, context: &ServerContext, prefix: &str) {
    if request.method() != "POST" {
        respond_error(request, 405, AppError::input("Mutations require POST"));
        return;
    }
    let expected_origin = format!("http://127.0.0.1:{}", context.state.port);
    if !header_exact(&request, "Origin", &expected_origin) {
        respond_error(
            request,
            403,
            AppError::input("Mutation Origin must be same-origin"),
        );
        return;
    }
    if request
        .header_values("Sec-Fetch-Site")
        .first()
        .is_some_and(|value| *value != "same-origin")
        || request.header_values("Sec-Fetch-Site").len() > 1
    {
        respond_error(
            request,
            403,
            AppError::input("Sec-Fetch-Site must be same-origin when present"),
        );
        return;
    }
    if !valid_token(&request, &context.state.token) {
        respond_error(request, 403, AppError::input("Invalid UI session token"));
        return;
    }
    if !header_exact(&request, "Content-Type", "application/json") {
        respond_error(
            request,
            415,
            AppError::input("Mutations require Content-Type: application/json"),
        );
        return;
    }
    if request.body().len() > MAX_BODY {
        respond_error(request, 413, AppError::input("Mutation body exceeds 2 MiB"));
        return;
    }
    let mutation: crate::service::UiMutation = match serde_json::from_slice(request.body()) {
        Ok(mutation) => mutation,
        Err(error) => {
            respond_error(
                request,
                400,
                AppError::input(format!("Invalid mutation JSON: {error}")),
            );
            return;
        }
    };
    match crate::service::apply_mutation(&context.root, prefix, mutation) {
        Ok(result) => {
            context.changes.notify(Some(prefix.to_string()), "mutation");
            respond_ok(request, 200, result);
        }
        Err(error) => respond_app_error(request, error),
    }
}

fn get_only<T: Serialize>(
    request: HttpRequest,
    operation: impl FnOnce() -> crate::error::Result<T>,
) {
    if request.method() != "GET" {
        respond_error(request, 405, AppError::input("Read endpoint requires GET"));
    } else {
        respond_result(request, operation());
    }
}

fn respond_result<T: Serialize>(request: HttpRequest, result: crate::error::Result<T>) {
    match result {
        Ok(data) => respond_ok(request, 200, data),
        Err(error) => respond_app_error(request, error),
    }
}

fn respond_app_error(request: HttpRequest, error: AppError) {
    let status = match error.category {
        ErrorCategory::Input => 400,
        ErrorCategory::NotFound => 404,
        ErrorCategory::Conflict => 409,
        ErrorCategory::Validation | ErrorCategory::Cycle | ErrorCategory::Project => 422,
        ErrorCategory::Io => 500,
    };
    respond_error(request, status, error);
}

fn respond_ok<T: Serialize>(request: HttpRequest, status: u16, data: T) {
    let value = json!({"api_version":1,"ok":true,"data":data});
    respond_json(request, status, &value);
}

fn respond_error(request: HttpRequest, status: u16, error: AppError) {
    let mut details = Map::new();
    details.insert("code".into(), json!(error.code));
    details.insert("message".into(), json!(error.message));
    details.insert("exit_code".into(), json!(error.exit_code()));
    details.extend((*error.details).clone());
    let value = json!({"api_version":1,"ok":false,"error":Value::Object(details)});
    respond_json(request, status, &value);
}

fn respond_json(request: HttpRequest, status: u16, value: &Value) {
    let mut bytes = serde_json::to_vec(value).unwrap_or_else(|_| b"null".to_vec());
    bytes.push(b'\n');
    request.respond(
        status,
        &[
            ("Content-Type", "application/json; charset=utf-8"),
            ("Cache-Control", "no-store"),
        ],
        &bytes,
    );
}

pub(crate) fn serve_sse(
    request: HttpRequest,
    changes: Arc<ChangeSignal>,
    shutdown: Arc<AtomicBool>,
    mut generation: u64,
) {
    let mut stream = take_stream(request);
    if write_streaming_head(
        &mut stream,
        200,
        &[
            ("Content-Type", "text/event-stream; charset=utf-8"),
            ("Cache-Control", "no-store"),
        ],
    )
    .and_then(|()| stream.write_all(b": connected\n\n"))
    .is_err()
    {
        return;
    }
    while !shutdown.load(Ordering::Acquire) {
        let hint = changes.wait(generation, Duration::from_secs(5), &shutdown);
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        let frame = if hint.generation > generation {
            generation = hint.generation;
            let data = serde_json::to_string(&hint).unwrap_or_else(|_| "{}".to_string());
            format!("event: change\ndata: {data}\n\n")
        } else {
            ": keepalive\n\n".to_string()
        };
        if stream.write_all(frame.as_bytes()).is_err() || stream.flush().is_err() {
            break;
        }
    }
}

fn respond_asset(request: HttpRequest, asset: assets::Asset) {
    request.respond(
        200,
        &[
            ("Content-Type", asset.content_type),
            ("Cache-Control", "no-store"),
        ],
        asset.bytes,
    );
}

fn header_exact(request: &HttpRequest, name: &str, expected: &str) -> bool {
    request.header_values(name).as_slice() == [expected]
}

fn valid_token(request: &HttpRequest, expected: &str) -> bool {
    let values = request.header_values("X-Tasker-Token");
    values.len() == 1 && constant_time_equal(values[0].as_bytes(), expected.as_bytes())
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn query_value(query: &str, key: &str) -> Option<String> {
    form_urlencoded::parse(query.as_bytes())
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}
