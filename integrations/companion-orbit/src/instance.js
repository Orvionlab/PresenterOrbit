const { InstanceBase, InstanceStatus } = require("@companion-module/base");
const { OrbitClient } = require("./client");
const { actions, feedbacks, presets } = require("./definitions");

class OrbitInstance extends InstanceBase {
  async init(config, _firstInit, secrets) {
    this.setVariableDefinitions(
      Object.fromEntries(
        [
          ["connected", "Connected"],
          ["workspace", "Workspace path"],
          ["presentation", "Live presentation"],
          ["cue", "Live cue text"],
          ["recording", "Recording active"],
          ["last_result", "Last command result"],
        ].map(([id, name]) => [id, { name }]),
      ),
    );
    this.setFeedbackDefinitions(feedbacks(this));
    await this.configUpdated(config, secrets);
  }

  getConfigFields() {
    return [
      { type: "textinput", id: "host", label: "Orbit host", width: 8, default: "127.0.0.1" },
      { type: "number", id: "port", label: "Remote port", width: 4, default: 49321, min: 1, max: 65535 },
      { type: "secret-text", id: "password", label: "Orbit Remote password", width: 12 },
      { type: "number", id: "pollInterval", label: "Poll interval (ms)", width: 6, default: 500, min: 250, max: 10000 },
    ];
  }

  async configUpdated(config, secrets) {
    this.client?.stop();
    this.connected = false;
    this.snapshot = null;
    this.catalogKey = undefined;
    this.publish();
    this.updateStatus(InstanceStatus.Connecting);
    try {
      const client = new OrbitClient(
        config,
        secrets?.password,
        (state) => {
          if (this.client !== client) return;
          this.snapshot = state;
          this.connected = true;
          this.updateStatus(
            state.controlsError ? InstanceStatus.UnknownWarning : InstanceStatus.Ok,
            state.controlsError || undefined,
          );
          this.publish();
        },
        (error) => {
          if (this.client !== client) return;
          this.connected = false;
          this.snapshot = null;
          this.updateStatus(InstanceStatus.ConnectionFailure, error.message);
          this.publish();
        },
      );
      this.client = client;
      client.start();
    } catch (error) {
      this.updateStatus(InstanceStatus.BadConfig, error.message);
    }
  }

  publish() {
    const state = this.snapshot;
    const { targets } = require("./targets");
    const key = JSON.stringify([targets(state), state?.timers]);
    if (key !== this.catalogKey) {
      this.catalogKey = key;
      this.setActionDefinitions(actions(this));
      this.setPresetDefinitions(...presets(state));
    }
    this.setVariableValues({
      connected: Boolean(this.connected),
      workspace: state?.workspacePath ?? "",
      presentation: state?.presentation?.title ?? "",
      cue: state?.currentSlide?.text ?? "",
      recording: state?.captureStatus?.active === true,
    });
    this.checkFeedbacks("connected", "presentation", "cue", "prop", "timer", "message", "recording", "layer");
  }

  async execute(payload) {
    const client = this.client;
    if (!this.connected || !client) throw new Error("Orbit is disconnected.");
    try {
      const result = await client.command(payload);
      if (this.client !== client) return;
      this.setVariableValues({ last_result: result.status });
      await client.refresh();
    } catch (error) {
      if (this.client === client) {
        this.setVariableValues({ last_result: error.message });
        this.log("error", error.message);
      }
      throw error;
    }
  }

  async destroy() {
    this.client?.stop();
    this.client = null;
    this.connected = false;
    this.snapshot = null;
  }
}

module.exports = { OrbitInstance };
