import clientModule from "../../companion-orbit/src/client.js";
import targetModule from "../../companion-orbit/src/targets.js";

const { OrbitClient } = clientModule;
const { targets, selectedPayload } = targetModule;
export const commands = {
  nextSlide: "Next cue",
  previousSlide: "Previous cue",
  clearAll: "Clear all",
};
export const layers = ["slide", "media", "music", "props", "messages", "announcements", "videoInput"];

export function resolveControl(status, settings) {
  if (!status?.workspacePath) throw new Error("An active Orbit workspace is required.");
  if (settings.mode === "target") return selectedPayload(status, settings.target);
  if (settings.mode === "navigate" && Object.hasOwn(commands, settings.command))
    return { command: settings.command, workspacePath: status.workspacePath };
  if (settings.mode === "clearLayer" && layers.includes(settings.layer))
    return { command: "clearLayer", layer: settings.layer, workspacePath: status.workspacePath };
  throw new Error("Choose an Orbit command or workspace target.");
}

export function targetActive(status, payload) {
  if (!status || !payload || status.workspacePath !== payload.workspacePath) return false;
  switch (payload.command) {
    case "cue":
    case "libraryCue":
    case "playlistCue":
      return (
        status.currentSlide?.id === payload.slideId &&
        (status.currentSlide.cueInstanceId ?? null) === (payload.cueInstanceId ?? null) &&
        status.presentation?.id === payload.presentationId &&
        (status.presentation.playlistEntryId ?? null) === (payload.playlistEntryId ?? null)
      );
    case "timer":
      return status.timerRuns?.[payload.id]?.paused === false;
    case "prop":
      return status.livePropIds?.includes(payload.id) === true;
    case "message":
      return payload.action === "show" && status.activeMessage?.id === payload.id;
    case "media":
      return status.background?.id === payload.id || status.audio?.id === payload.id;
    case "capture":
      return status.captureStatus?.active === true;
    default:
      return false;
  }
}

export function buttonView(status, settings, error, busy) {
  if (error || !status) return { title: "Orbit\nOffline", color: "#803232" };
  try {
    const payload = resolveControl(status, settings);
    const label =
      settings.mode === "target"
        ? targets(status).find((target) => target.id === settings.target)?.label
        : settings.mode === "clearLayer"
          ? `Clear ${settings.layer}`
          : commands[settings.command];
    return {
      title: busy ? "Working…" : (settings.label?.trim() || label).split(" / ").slice(-2).join("\n").slice(0, 70),
      color: busy ? "#805b20" : targetActive(status, payload) ? "#187038" : "#203040",
    };
  } catch {
    return { title: "Select\nOrbit target", color: "#805b20" };
  }
}

export function buttonImage(color) {
  return `<svg xmlns="http://www.w3.org/2000/svg" width="144" height="144"><rect width="144" height="144" rx="14" fill="${color}"/><path d="M22 12h100" stroke="#8fc9ff" stroke-width="4"/></svg>`;
}

export class OrbitControls {
  constructor(inspector, factory = (...args) => new OrbitClient(...args)) {
    this.inspector = inspector;
    this.factory = factory;
    this.buttons = new Map();
    this.status = null;
    this.error = "Configure the Orbit connection.";
  }

  configure(config) {
    const key = JSON.stringify([config.host, config.port, config.password, config.pollInterval]);
    if (key === this.configKey) return;
    this.configKey = key;
    this.config = config;
    this.disconnect();
    this.connect();
  }

  disconnect() {
    const old = this.client;
    this.client = null;
    old?.stop();
    this.status = null;
    this.error = "Orbit is disconnected.";
    this.refresh();
  }

  connect() {
    if (this.client || !this.config || !this.buttons.size) return;
    try {
      const client = this.factory(
        this.config,
        this.config.password,
        (status) => {
          if (this.client !== client) return;
          this.status = status;
          this.error = null;
          this.refresh();
        },
        (error) => {
          if (this.client !== client) return;
          this.status = null;
          this.error = error.message;
          this.refresh();
        },
      );
      this.client = client;
      client.start();
    } catch (error) {
      this.error = error.message;
      this.refresh();
    }
  }

  appear(action, settings) {
    this.buttons.set(action.id, { action, settings, busy: false });
    this.connect();
    this.refresh();
  }
  update(action, settings) {
    const entry = this.buttons.get(action.id);
    if (entry) this.buttons.set(action.id, { action: entry.action, settings, busy: false });
    this.refresh();
  }
  disappear(action) {
    this.buttons.delete(action.id);
    if (!this.buttons.size) this.disconnect();
  }
  refresh() {
    for (const entry of this.buttons.values()) {
      const view = buttonView(this.status, entry.settings, this.error, entry.busy);
      const key = JSON.stringify(view);
      if (entry.view === key) continue;
      entry.view = key;
      void Promise.all([entry.action.setTitle(view.title), entry.action.setImage(buttonImage(view.color))]).catch(
        () => {
          if (this.buttons.get(entry.action.id) === entry) entry.view = null;
        },
      );
    }
    this.publish(false);
  }
  publish(force = true) {
    const payload = {
      type: "catalog",
      connected: Boolean(this.status),
      error: this.error,
      workspace: this.status?.workspacePath ?? null,
      targets: targets(this.status),
    };
    const key = JSON.stringify(payload);
    if (!force && this.catalogKey === key) return;
    this.catalogKey = key;
    void Promise.resolve(this.inspector(payload)).catch(() => {});
  }
  async press(action) {
    const entry = this.buttons.get(action.id);
    if (!entry || entry.busy) return;
    const client = this.client;
    try {
      if (!client || !this.status) throw new Error(this.error || "Orbit is disconnected.");
      const payload = resolveControl(this.status, entry.settings);
      entry.busy = true;
      this.refresh();
      await client.command(payload);
      if (client !== this.client || this.buttons.get(action.id) !== entry) return;
      await action.showOk();
      await this.inspector({ type: "result", error: null });
      try {
        await client.refresh();
      } catch (error) {
        if (client === this.client) {
          this.status = null;
          this.error = error.message;
          this.refresh();
        }
      }
    } catch (error) {
      if (client !== this.client || this.buttons.get(action.id) !== entry) return;
      await action.showAlert();
      await this.inspector({ type: "result", error: error.message });
    } finally {
      entry.busy = false;
      this.refresh();
    }
  }
}
