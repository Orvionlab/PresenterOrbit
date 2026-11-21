const http = require("node:http");

function httpRequest(url, options) {
  return new Promise((resolve, reject) => {
    const body = options.body;
    const request = http.request(
      url,
      {
        method: options.method,
        headers: {
          ...options.headers,
          Connection: "close",
          ...(body ? { "Content-Length": Buffer.byteLength(body) } : {}),
        },
        signal: options.signal,
        agent: false,
      },
      (response) => {
        const chunks = [];
        response.on("data", (chunk) => chunks.push(chunk));
        response.on("error", reject);
        response.on("end", () =>
          resolve({
            status: response.statusCode,
            ok: response.statusCode >= 200 && response.statusCode < 300,
            text: async () => Buffer.concat(chunks).toString("utf8"),
          }),
        );
      },
    );
    request.on("error", reject);
    request.end(body);
  });
}

class OrbitClient {
  constructor(config, password, onStatus, onError, fetcher = httpRequest) {
    const host = String(config.host || "").trim();
    const port = Number(config.port);
    const interval = Number(config.pollInterval);
    if (
      !host ||
      /[\s/@?#]/.test(host) ||
      !Number.isInteger(port) ||
      port < 1 ||
      port > 65535 ||
      !Number.isInteger(interval) ||
      interval < 250 ||
      interval > 10000 ||
      !password
    )
      throw new Error("Configure a host, port, Remote password and polling interval (250–10000 ms).");
    this.origin = new URL(`http://${host.includes(":") && !host.startsWith("[") ? `[${host}]` : host}:${port}`);
    this.auth = `Basic ${Buffer.from(`presenter-orbit:${password}`).toString("base64")}`;
    this.interval = interval;
    this.onStatus = onStatus;
    this.onError = onError;
    this.fetcher = fetcher;
    this.controller = new AbortController();
    this.status = null;
  }

  async request(path, payload) {
    this.controller.signal.throwIfAborted();
    const response = await this.fetcher(new URL(path, this.origin), {
      method: payload ? "POST" : "GET",
      redirect: "error",
      headers: { Authorization: this.auth, ...(payload ? { "Content-Type": "application/json" } : {}) },
      body: payload ? JSON.stringify(payload) : undefined,
      signal: AbortSignal.any([this.controller.signal, AbortSignal.timeout(5000)]),
    });
    this.controller.signal.throwIfAborted();
    if (response.status === 401) throw new Error("Authentication failed. Check the Orbit Remote password.");
    const text = await response.text();
    let result;
    try {
      result = JSON.parse(text);
    } catch {
      throw new Error("Orbit returned an invalid JSON response.");
    }
    if (!response.ok || result.ok === false) throw new Error(result.error || `Orbit HTTP ${response.status}`);
    return { code: response.status, result };
  }

  async refresh() {
    const { result } = await this.request("/api/v1/status");
    if (result?.controlsError) throw new Error(`Orbit catalog: ${result.controlsError}`);
    if (!result || typeof result !== "object" || !Object.hasOwn(result, "workspacePath"))
      throw new Error("Orbit status does not contain a workspace identity.");
    this.controller.signal.throwIfAborted();
    this.status = result;
    this.onStatus(result);
    return result;
  }

  start() {
    void this.poll();
  }
  async poll() {
    try {
      await this.refresh();
    } catch (error) {
      if (this.controller.signal.aborted) return;
      this.status = null;
      this.onError(error);
    }
    if (!this.controller.signal.aborted) this.timer = setTimeout(() => void this.poll(), this.interval);
  }

  stop() {
    clearTimeout(this.timer);
    this.controller.abort(new Error("Orbit connection was retired."));
    this.status = null;
  }

  async command(payload) {
    if (!this.status) throw new Error("Orbit is disconnected.");
    if (payload.workspacePath !== undefined && payload.workspacePath !== this.status.workspacePath)
      throw new Error("The action belongs to a different workspace. Select its target again.");
    const { code, result } = await this.request("/api/v1/command", payload);
    if (code !== 202) {
      if (!["applied", "unchanged"].includes(result.status))
        throw new Error("Orbit did not confirm command acceptance.");
      return result;
    }
    if (!result.requestId)
      throw new Error("Orbit queued this command without execution feedback. Its result is unconfirmed.");
    const deadline = Date.now() + 30000;
    while (Date.now() < deadline) {
      const { result: progress } = await this.request(`/api/v1/commands/${encodeURIComponent(result.requestId)}`);
      if (progress.workspacePath !== payload.workspacePath || progress.command !== payload.command)
        throw new Error("Orbit command feedback has a different identity.");
      if (progress.status === "unconfirmed") throw new Error("Orbit could not confirm command execution.");
      const terminals = payload.command === "macro" ? ["completed"] : ["applied", "dispatched"];
      if (terminals.includes(progress.status)) return progress;
      if (!["queued", "dispatched"].includes(progress.status))
        throw new Error("Orbit returned an invalid command result.");
      await new Promise((resolve, reject) => {
        const signal = this.controller.signal;
        const abort = () => {
          clearTimeout(timer);
          reject(signal.reason);
        };
        const timer = setTimeout(() => {
          signal.removeEventListener("abort", abort);
          resolve();
        }, 100);
        signal.addEventListener("abort", abort, { once: true });
        if (signal.aborted) abort();
      });
    }
    throw new Error("Orbit command execution was not confirmed within 30 seconds. It was not resubmitted.");
  }
}

module.exports = { OrbitClient, httpRequest };
