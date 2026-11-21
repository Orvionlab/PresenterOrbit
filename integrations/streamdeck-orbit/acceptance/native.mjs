import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { launchPlugin } from "../test/host.mjs";
import clientModule from "../../companion-orbit/src/client.js";
import targetModule from "../../companion-orbit/src/targets.js";

const args = Object.fromEntries(
  process.argv.slice(2).map((arg) => {
    const split = arg.indexOf("=");
    if (!arg.startsWith("--") || split < 0) throw new Error("Use --orbit=URL --password-file=PATH.");
    return [arg.slice(2, split), arg.slice(split + 1)];
  }),
);
const url = new URL(args.orbit);
if (url.protocol !== "http:" || url.hostname !== "127.0.0.1" || url.username || url.password)
  throw new Error("Only an isolated loopback Orbit test endpoint is allowed.");
const password = (await readFile(args["password-file"], "utf8")).trim();
const config = { host: url.hostname, port: Number(url.port), pollInterval: 250 };
const client = new clientModule.OrbitClient(
  config,
  password,
  () => {},
  () => {},
);
const initial = await client.refresh();
if (!/[\\/]\.tmp[\\/]ctl14[\\/]workspace[\\/]workspace\.json$/i.test(initial.workspacePath))
  throw new Error("This runner requires the synthetic CTL-14 fixture workspace.");
const host = await launchPlugin(args.node);
const checks = [];
try {
  host.globals({ ...config, password });
  let settings = { mode: "navigate", command: "nextSlide" };
  host.send("willAppear", host.appearance(settings));
  host.send("propertyInspectorDidAppear", {});
  await host.wait((message) => message.event === "sendToPropertyInspector" && message.payload.connected);
  checks.push("Packaged Elgato SDK registered and authenticated against native Orbit.");
  async function press(next) {
    settings = next;
    host.messages.length = 0;
    host.send("didReceiveSettings", host.appearance(settings));
    host.send("keyDown", host.appearance(settings));
    await host.wait((message) => message.event === "showOk", 35000);
    return client.refresh();
  }
  const choices = targetModule.targets(initial);
  const find = (predicate) => {
    const target = choices.find((item) => predicate(JSON.parse(item.id)));
    assert.ok(target, "Required native fixture target is missing.");
    return { mode: "target", target: target.id };
  };
  let state = await press(find((item) => item.command === "libraryCue" && item.slideId === "ctl14-slide-1"));
  assert.equal(state.currentSlide.id, "ctl14-slide-1");
  await host.wait((message) => message.event === "setImage" && message.payload.image.includes("#187038"));
  state = await press({ mode: "navigate", command: "nextSlide" });
  assert.equal(state.currentSlide.id, "ctl14-slide-2");
  checks.push("Exact library cue and next-cue key events changed native cue state and live feedback.");
  await press(find((item) => item.command === "timer" && item.id === "ctl14-timer" && item.action === "reset"));
  state = await press(find((item) => item.command === "timer" && item.id === "ctl14-timer" && item.action === "start"));
  assert.equal(state.timerRuns["ctl14-timer"].paused, false);
  await host.wait((message) => message.event === "setImage" && message.payload.image.includes("#187038"));
  checks.push("Timer start used native state and produced green key feedback.");
  await press(find((item) => item.command === "macro" && item.id === "ctl14-macro"));
  checks.push("Queued native macro completed before the SDK emitted showOk.");
  host.messages.length = 0;
  host.globals({ ...config, password: "incorrect-synthetic-password" });
  await host.wait(
    (message) => message.event === "sendToPropertyInspector" && /Authentication/.test(message.payload.error),
  );
  await host.wait((message) => message.event === "setImage" && message.payload.image.includes("#803232"));
  host.messages.length = 0;
  host.globals({ ...config, password });
  await host.wait((message) => message.event === "sendToPropertyInspector" && message.payload.connected);
  checks.push("Incorrect native credentials cleared feedback; corrected configuration recovered.");
  host.send("willDisappear", host.appearance(settings));
  console.log(JSON.stringify({ passed: true, checks }, null, 2));
} finally {
  client.stop();
  await host.close();
}
