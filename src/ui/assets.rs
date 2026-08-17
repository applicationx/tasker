pub struct Asset {
    pub bytes: &'static [u8],
    pub content_type: &'static str,
}

const FAVICON: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><rect width="64" height="64" rx="14" fill="#e0a95a"/><path d="M14 16h36v9H37v25H27V25H14z" fill="#211f1b"/></svg>"##;

pub fn get(path: &str) -> Option<Asset> {
    let (bytes, content_type) = match path {
        "/" | "/index.html" => (
            include_bytes!("assets/index.html").as_slice(),
            "text/html; charset=utf-8",
        ),
        "/favicon.ico" | "/favicon.svg" => (FAVICON, "image/svg+xml"),
        "/assets/app.css" => (
            include_bytes!("assets/app.css").as_slice(),
            "text/css; charset=utf-8",
        ),
        "/assets/app.js" => (
            include_bytes!("assets/app.js").as_slice(),
            "text/javascript; charset=utf-8",
        ),
        "/assets/api.js" => (
            include_bytes!("assets/api.js").as_slice(),
            "text/javascript; charset=utf-8",
        ),
        "/assets/dom.js" => (
            include_bytes!("assets/dom.js").as_slice(),
            "text/javascript; charset=utf-8",
        ),
        "/assets/model.js" => (
            include_bytes!("assets/model.js").as_slice(),
            "text/javascript; charset=utf-8",
        ),
        "/assets/router.js" => (
            include_bytes!("assets/router.js").as_slice(),
            "text/javascript; charset=utf-8",
        ),
        "/assets/views/board.js" => (
            include_bytes!("assets/views/board.js").as_slice(),
            "text/javascript; charset=utf-8",
        ),
        "/assets/views/brief.js" => (
            include_bytes!("assets/views/brief.js").as_slice(),
            "text/javascript; charset=utf-8",
        ),
        "/assets/views/dependencies.js" => (
            include_bytes!("assets/views/dependencies.js").as_slice(),
            "text/javascript; charset=utf-8",
        ),
        "/assets/views/history.js" => (
            include_bytes!("assets/views/history.js").as_slice(),
            "text/javascript; charset=utf-8",
        ),
        "/assets/views/knowledge.js" => (
            include_bytes!("assets/views/knowledge.js").as_slice(),
            "text/javascript; charset=utf-8",
        ),
        "/assets/views/overview.js" => (
            include_bytes!("assets/views/overview.js").as_slice(),
            "text/javascript; charset=utf-8",
        ),
        "/assets/views/projects.js" => (
            include_bytes!("assets/views/projects.js").as_slice(),
            "text/javascript; charset=utf-8",
        ),
        "/assets/views/shared.js" => (
            include_bytes!("assets/views/shared.js").as_slice(),
            "text/javascript; charset=utf-8",
        ),
        "/assets/views/task.js" => (
            include_bytes!("assets/views/task.js").as_slice(),
            "text/javascript; charset=utf-8",
        ),
        "/assets/views/tasks.js" => (
            include_bytes!("assets/views/tasks.js").as_slice(),
            "text/javascript; charset=utf-8",
        ),
        "/assets/views/workflow.js" => (
            include_bytes!("assets/views/workflow.js").as_slice(),
            "text/javascript; charset=utf-8",
        ),
        _ => return None,
    };
    Some(Asset {
        bytes,
        content_type,
    })
}
