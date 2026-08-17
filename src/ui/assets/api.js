export class ApiError extends Error {
  constructor(payload = {}, status = 0) {
    const error = payload.error || payload;
    super(error.message || `Tasker API request failed (${status || "network"})`);
    this.name = "ApiError";
    this.status = status;
    this.code = error.code || (status === 0 ? "network_error" : "request_failed");
    this.exitCode = error.exit_code;
    const flattened = Object.fromEntries(Object.entries(error).filter(([key]) => !["code", "message", "exit_code", "details"].includes(key)));
    this.details = { ...flattened, ...(error.details && typeof error.details === "object" ? error.details : {}) };
    for (const key of ["expected_revision", "current_revision", "validation"]) {
      if (error[key] !== undefined) {
        this[key] = error[key];
        if (this.details[key] === undefined) this.details[key] = error[key];
      }
    }
  }
}

export class TaskerApi {
  #token = "";
  #session = null;
  #changeController = null;
  #changeTimer = null;

  async request(path, options = {}) {
    const headers = new Headers({ Accept: "application/json" });
    if (options.body !== undefined) headers.set("Content-Type", "application/json");
    if (options.token && this.#token) headers.set("X-Tasker-Token", this.#token);
    const request = {
      method: options.method || "GET",
      headers,
      cache: "no-store",
      credentials: "same-origin",
      signal: options.signal,
    };
    if (options.body !== undefined) request.body = JSON.stringify(options.body);

    let response;
    try {
      response = await fetch(path, request);
    } catch (error) {
      if (error?.name === "AbortError") throw error;
      throw new ApiError({ code: "network_error", message: "The local Tasker UI process could not be reached." }, 0);
    }

    let payload;
    try {
      payload = await response.json();
    } catch {
      throw new ApiError({ code: "invalid_api_response", message: "Tasker returned an unreadable API response." }, response.status);
    }
    if (!response.ok || payload?.ok === false) throw new ApiError(payload, response.status);
    return payload?.data !== undefined ? payload.data : payload;
  }

  async session(options = {}) {
    const data = await this.request("/api/v1/session", options);
    this.#session = data;
    this.#token = data?.token || data?.api_token || data?.session_token || "";
    return data;
  }

  get sessionData() { return this.#session; }

  projects(options = {}) {
    return this.request("/api/v1/projects", options);
  }

  snapshot(prefix, options = {}) {
    return this.request(`/api/v1/projects/${segment(prefix)}/snapshot`, options);
  }

  task(prefix, taskId, options = {}) {
    return this.request(`/api/v1/projects/${segment(prefix)}/tasks/${segment(taskId)}`, options);
  }

  resource(prefix, resourceId, options = {}) {
    return this.request(`/api/v1/projects/${segment(prefix)}/resources/${segment(resourceId)}`, options);
  }

  decision(prefix, decisionId, options = {}) {
    return this.request(`/api/v1/projects/${segment(prefix)}/decisions/${segment(decisionId)}`, options);
  }

  projectBrief(prefix, options = {}) {
    return this.request(`/api/v1/projects/${segment(prefix)}/project-brief`, options);
  }

  brief(prefix, taskId, maxBytes, format = "markdown", options = {}) {
    const query = new URLSearchParams();
    if (taskId) query.set("task_id", taskId);
    if (maxBytes) query.set("max_bytes", String(maxBytes));
    if (format) query.set("format", format);
    return this.request(`/api/v1/projects/${segment(prefix)}/brief?${query}`, options);
  }

  events(prefix, offset = 0, limit = 100, options = {}) {
    const query = new URLSearchParams({ offset: String(offset), limit: String(limit) });
    return this.request(`/api/v1/projects/${segment(prefix)}/events?${query}`, options);
  }

  mutate(prefix, mutation, options = {}) {
    return this.request(`/api/v1/projects/${segment(prefix)}/mutations`, {
      ...options,
      method: "POST",
      token: true,
      body: mutation,
    });
  }

  watchChanges(onChange, onStatus = () => {}) {
    this.stopChanges();
    let stopped = false;
    let retryDelay = 1000;
    let generation = Number(this.#session?.change_generation) || 0;

    const connect = async () => {
      if (stopped) return;
      const controller = new AbortController();
      this.#changeController = controller;
      try {
        const response = await fetch(`/api/v1/changes?since=${encodeURIComponent(generation)}`, {
          headers: {
            Accept: "text/event-stream",
            "X-Tasker-Token": this.#token,
          },
          cache: "no-store",
          credentials: "same-origin",
          signal: controller.signal,
        });
        if (!response.ok || !response.body) throw new Error(`SSE ${response.status}`);
        retryDelay = 1000;
        onStatus("connected");
        await consumeEventStream(response.body, (change) => {
          const nextGeneration = Number(change?.generation);
          if (!Number.isFinite(nextGeneration) || nextGeneration <= generation) return;
          generation = nextGeneration;
          onChange(change);
        }, controller.signal);
        if (!stopped) this.#changeTimer = window.setTimeout(connect, 50);
      } catch (error) {
        if (stopped || error?.name === "AbortError") return;
        onStatus("disconnected");
        this.#changeTimer = window.setTimeout(connect, retryDelay);
        retryDelay = Math.min(retryDelay * 2, 15000);
      }
    };

    connect();
    return () => {
      stopped = true;
      this.stopChanges();
    };
  }

  stopChanges() {
    this.#changeController?.abort();
    this.#changeController = null;
    if (this.#changeTimer !== null) window.clearTimeout(this.#changeTimer);
    this.#changeTimer = null;
  }
}

async function consumeEventStream(stream, onChange, signal) {
  const reader = stream.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    while (!signal.aborted) {
      const { value, done } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true }).replaceAll("\r\n", "\n");
      let boundary;
      while ((boundary = buffer.indexOf("\n\n")) >= 0) {
        const block = buffer.slice(0, boundary);
        buffer = buffer.slice(boundary + 2);
        const data = block.split("\n")
          .filter((line) => line.startsWith("data:"))
          .map((line) => line.slice(5).trimStart())
          .join("\n");
        if (!data) continue;
        try { onChange(JSON.parse(data)); }
        catch { onChange({ reason: "invalid_event" }); }
      }
    }
  } finally {
    reader.releaseLock();
  }
}

function segment(value) {
  return encodeURIComponent(String(value));
}
