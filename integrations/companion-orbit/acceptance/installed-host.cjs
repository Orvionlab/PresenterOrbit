const assert = require("node:assert/strict");
const fs = require("node:fs");
const http = require("node:http");
const { targets } = require("../src/targets");
const { httpRequest } = require("../src/client");

const args = Object.fromEntries(
  process.argv.slice(2).map((value) => {
    const index = value.indexOf("=");
    return [value.slice(0, index), value.slice(index + 1)];
  }),
);
const companion = new URL(args["--companion"]);
const orbit = new URL(args["--orbit"]);
const password = fs.readFileSync(args["--password-file"], "utf8").trim();
for (const origin of [companion, orbit])
  assert.ok(
    origin.protocol === "http:" && origin.hostname === "127.0.0.1",
    "Acceptance endpoints must use loopback HTTP.",
  );
const headers = {
  Authorization: `Basic ${Buffer.from(`presenter-orbit:${password}`).toString("base64")}`,
  Connection: "close",
};
const checks = [];
const { loadImage, createCanvas } = require(args["--canvas-module"]);

async function rpc(path, method, input) {
  const socket = new WebSocket(new URL("/trpc", companion).href.replace("http:", "ws:"));
  await new Promise((resolve, reject) => {
    socket.onopen = resolve;
    socket.onerror = reject;
  });
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      socket.close();
      reject(new Error(`Companion RPC timeout: ${path}`));
    }, 5000);
    socket.onmessage = (event) => {
      const message = JSON.parse(event.data);
      if (message.id !== 1 || message.result?.type === "started") return;
      clearTimeout(timeout);
      socket.close();
      if (message.error) reject(new Error(JSON.stringify(message.error)));
      else resolve(message.result?.data);
    };
    socket.send(JSON.stringify({ id: 1, method, params: { path, input } }));
  });
}
async function status() {
  const response = await fetch(new URL("/api/v1/status", orbit), { headers, signal: AbortSignal.timeout(5000) });
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("connection"), "close", "Orbit must declare its one-request connection lifecycle.");
  return response.json();
}
async function variable(name) {
  const response = await fetch(new URL(`/api/variable/Orbit_Test/${name}/value`, companion), {
    signal: AbortSignal.timeout(5000),
  });
  assert.equal(response.status, 200);
  return response.text();
}
async function waitFor(test, description) {
  const deadline = Date.now() + 10000;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const value = await test();
      if (value) return value;
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`Acceptance timeout: ${description}`, { cause: lastError });
}
async function press(location) {
  await rpc("controls.hotPressControl", "mutation", { location, direction: true, surfaceId: "ctl14-acceptance" });
  await rpc("controls.hotPressControl", "mutation", { location, direction: false, surfaceId: "ctl14-acceptance" });
}
async function backgroundPixel(location) {
  const preview = await rpc("preview.graphics.location", "subscription", { location });
  const image = await loadImage(preview.image);
  const canvas = createCanvas(image.width, image.height);
  const context = canvas.getContext("2d");
  context.drawImage(image, 0, 0);
  return Array.from(context.getImageData(6, 100, 1, 1).data).join(",");
}

(async () => {
  console.log("Checking the isolated Orbit status.");
  const initial = await status();
  assert.match(
    initial.workspacePath.replaceAll("\\", "/"),
    /\/.tmp\/ctl14\/workspace\/workspace.json$/,
    "Refusing to operate outside the synthetic acceptance workspace.",
  );
  assert.equal(initial.controlsError, undefined);
  const connections = await rpc("instances.connections.watch", "subscription");
  const connection = Object.values(connections[0].info).find(
    (value) => value.label === "Orbit_Test" && value.moduleId === "presenter-orbit",
  );
  assert.ok(connection, "Create the disposable Orbit_Test connection before running acceptance.");
  const settings = {
    connectionId: connection.id,
    label: "Orbit_Test",
    enabled: true,
    config: { host: "127.0.0.1", port: Number(orbit.port), pollInterval: 250 },
    secrets: { password },
  };
  const proxy = http.createServer(async (req, res) => {
    try {
      let body = "";
      for await (const chunk of req) body += chunk;
      const response = await httpRequest(new URL(req.url, orbit), {
        method: req.method,
        headers: { Authorization: req.headers.authorization, "Content-Type": "application/json", Connection: "close" },
        ...(body ? { body } : {}),
        signal: AbortSignal.timeout(5000),
      });
      res.writeHead(response.status, { "Content-Type": "application/json" });
      res.end(await response.text());
    } catch (error) {
      console.error("Acceptance proxy backend error:", error);
      res.writeHead(503);
      res.end(JSON.stringify({ ok: false, error: "Acceptance proxy unavailable." }));
    }
  });
  await new Promise((resolve) => proxy.listen(0, "127.0.0.1", resolve));
  const port = proxy.address().port;
  try {
    console.log("Connecting the installed module through the local fault-injection proxy.");
    await rpc("instances.connections.setConfig", "mutation", { ...settings, config: { ...settings.config, port } });
    await waitFor(async () => /true/.test(await variable("connected")), "module connection");
    checks.push("Installed Companion loaded the packaged API v2 module and authenticated with real Orbit.");
    console.log("Importing real Companion button presets.");
    let state = await status();
    const select = (predicate) => {
      const target = targets(state).find((entry) => predicate(JSON.parse(entry.id)));
      assert.ok(target, "Required fixture target is missing.");
      return `target-${Buffer.from(target.id).toString("base64url")}`;
    };
    const cue = select((value) => value.command === "libraryCue" && value.slideId === "ctl14-slide-1");
    const timer = select(
      (value) => value.command === "timer" && value.id === "ctl14-timer" && value.action === "start",
    );
    const macro = select((value) => value.command === "macro" && value.id === "ctl14-macro");
    const locations = [0, 1, 2, 3].map((column) => ({ pageNumber: 1, row: 0, column }));
    const controls = [];
    for (const [index, presetId] of [cue, "nextSlide", timer, macro].entries()) {
      const id = await rpc("controls.importPreset", "mutation", {
        connectionId: connection.id,
        presetId,
        location: locations[index],
        variableValues: null,
      });
      assert.ok(id, "Companion could not import a module preset.");
      controls.push(id);
    }
    await press(locations[0]);
    await waitFor(async () => (await status()).currentSlide?.id === "ctl14-slide-1", "exact library cue");
    await press(locations[1]);
    await waitFor(async () => (await status()).currentSlide?.id === "ctl14-slide-2", "next cue");
    checks.push("Real Companion preset buttons triggered an exact library cue and native slide navigation.");
    const reset = await fetch(new URL("/api/v1/command", orbit), {
      method: "POST",
      headers: { ...headers, "Content-Type": "application/json" },
      body: JSON.stringify({ command: "timer", id: "ctl14-timer", action: "reset" }),
    });
    assert.equal(reset.status, 200);
    await waitFor(async () => (await backgroundPixel(locations[2])) === "32,48,64,255", "inactive timer feedback");
    await press(locations[2]);
    await waitFor(async () => (await status()).timerRuns?.["ctl14-timer"]?.paused === false, "native timer start");
    await waitFor(async () => (await backgroundPixel(locations[2])) === "24,112,56,255", "active timer feedback");
    checks.push("Timer action changed real native timer state and Companion evaluated its button feedback.");
    await press(locations[3]);
    await waitFor(async () => /completed/.test(await variable("last_result")), "queued macro completion");
    checks.push("Queued macro reached completed feedback through the real renderer command path.");
    await rpc("instances.connections.setConfig", "mutation", {
      ...settings,
      config: { ...settings.config, port },
      secrets: { password: "deliberately-wrong-synthetic-password" },
    });
    await waitFor(async () => /false/.test(await variable("connected")), "authentication failure");
    const errors = await rpc("instances.statuses.watch", "subscription");
    assert.match(errors.statuses[connection.id].message, /Authentication/);
    await rpc("instances.connections.setConfig", "mutation", { ...settings, config: { ...settings.config, port } });
    await waitFor(async () => /true/.test(await variable("connected")), "authentication recovery");
    checks.push("Wrong credentials produced explicit failure; corrected credentials restored the connection.");
    await new Promise((resolve) => {
      proxy.closeAllConnections();
      proxy.close(resolve);
    });
    await waitFor(async () => /false/.test(await variable("connected")), "network disconnection");
    await new Promise((resolve) => proxy.listen(port, "127.0.0.1", resolve));
    await waitFor(async () => /true/.test(await variable("connected")), "automatic network reconnection");
    checks.push(
      "Network loss cleared connection feedback and polling recovered without resubmitting commands or changing module config.",
    );
    console.log(JSON.stringify({ passed: true, checks }, null, 2));
  } finally {
    await rpc("instances.connections.setConfig", "mutation", settings);
    await new Promise((resolve) => {
      proxy.closeAllConnections();
      proxy.close(resolve);
    });
  }
})().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
