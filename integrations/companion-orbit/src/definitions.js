const { selectedPayload, targets, mediaPayload } = require("./targets");
const layers = ["slide", "media", "music", "props", "messages", "announcements", "videoInput"];
const dropdown = (id, label, values) => ({
  type: "dropdown",
  id,
  label,
  default: values[0]?.id ?? "",
  choices: values,
});
const choices = (values) => values.map((id) => ({ id, label: id }));

function actions(instance) {
  const run = (payload) => instance.execute(payload);
  return {
    navigate: {
      name: "Navigate / clear all",
      options: [dropdown("command", "Command", choices(["nextSlide", "previousSlide", "clearAll"]))],
      callback: ({ options }) => run({ command: options.command }),
    },
    target: {
      name: "Trigger workspace target",
      options: [dropdown("target", "Target (workspace and occurrence bound)", targets(instance.snapshot))],
      callback: ({ options }) => run(selectedPayload(instance.snapshot, options.target)),
    },
    clearLayer: {
      name: "Clear layer",
      options: [dropdown("layer", "Layer", choices(layers))],
      callback: ({ options }) => run({ command: "clearLayer", layer: options.layer }),
    },
    timerIncrement: {
      name: "Adjust timer",
      options: [
        dropdown(
          "id",
          "Timer",
          (instance.snapshot?.timers || []).map((timer) => ({ id: timer.id, label: timer.name })),
        ),
        { type: "number", id: "seconds", label: "Seconds (signed)", default: 30, min: -86400, max: 86400 },
      ],
      callback: ({ options }) => {
        if (!instance.snapshot?.timers?.some((timer) => timer.id === options.id) || !Number.isFinite(options.seconds))
          throw new Error("Timer or adjustment is invalid.");
        return run({
          command: "timer",
          id: options.id,
          action: "increment",
          seconds: options.seconds,
          workspacePath: instance.snapshot.workspacePath,
        });
      },
    },
    stageMessage: {
      name: "Stage message",
      options: [
        dropdown("action", "Action", choices(["edit", "show", "hide", "clear"])),
        { type: "textinput", id: "text", label: "Text", default: "", useVariables: true },
      ],
      callback: async ({ options }) => {
        if (!instance.snapshot?.workspacePath) throw new Error("An active workspace is required.");
        const payload = {
          command: "stageMessage",
          action: options.action,
          workspacePath: instance.snapshot.workspacePath,
        };
        if (options.action === "edit") {
          if (typeof options.text !== "string") throw new Error("Stage message text is required.");
          payload.text = options.text;
        }
        await run(payload);
      },
    },
    media: {
      name: "Current media transport",
      options: [
        dropdown("layer", "Layer", choices(["media", "audio"])),
        dropdown("action", "Action", choices(["play", "pause", "restart", "inPoint", "outPoint", "seek"])),
        { type: "number", id: "position", label: "Seek position (0–1)", default: 0, min: 0, max: 1, step: 0.01 },
      ],
      callback: ({ options }) => run(mediaPayload(instance.snapshot, options.layer, options.action, options.position)),
    },
  };
}

function feedbacks(instance) {
  const boolean = (name, options, predicate) => ({
    type: "boolean",
    name,
    options,
    defaultStyle: { color: 0xffffff, bgcolor: 0x187038 },
    callback: ({ options: values }) => Boolean(instance.connected && predicate(instance.snapshot, values)),
  });
  const id = [{ type: "textinput", id: "id", label: "Exact ID", default: "" }];
  const workspace = { type: "textinput", id: "workspacePath", label: "Exact workspace path", default: "" };
  return {
    connected: boolean("Connected to Orbit", [], () => true),
    presentation: boolean("Presentation is live", id, (state, values) => state?.presentation?.id === values.id),
    cue: boolean(
      "Cue occurrence is live",
      [
        ...id,
        workspace,
        { type: "textinput", id: "presentationId", label: "Presentation ID", default: "" },
        { type: "textinput", id: "playlistEntryId", label: "Playlist entry ID (empty = library cue)", default: "" },
        { type: "textinput", id: "occurrence", label: "Occurrence ID (empty = unscoped)", default: "" },
      ],
      (state, values) =>
        state?.workspacePath === values.workspacePath &&
        state?.currentSlide?.id === values.id &&
        (state.currentSlide.cueInstanceId ?? "") === values.occurrence &&
        state.presentation?.id === values.presentationId &&
        (state.presentation.playlistEntryId ?? "") === values.playlistEntryId,
    ),
    prop: boolean("Prop is live", id, (state, values) => state?.livePropIds?.includes(values.id)),
    timer: boolean(
      "Timer is running",
      [...id, workspace],
      (state, values) =>
        state?.workspacePath === values.workspacePath && state?.timerRuns?.[values.id]?.paused === false,
    ),
    message: boolean("Audience message is live", id, (state, values) => state?.activeMessage?.id === values.id),
    recording: boolean("Recording is active", [], (state) => state?.captureStatus?.active === true),
    layer: boolean(
      "Output includes layer",
      [
        { type: "textinput", id: "screenId", label: "Screen ID", default: "" },
        dropdown("layer", "Layer", choices(layers)),
      ],
      (state, values) => state?.layers?.[values.screenId]?.[values.layer] === true,
    ),
  };
}

function presets(state) {
  const definitions = Object.fromEntries(
    ["nextSlide", "previousSlide", "clearAll"].map((command) => [
      command,
      {
        type: "simple",
        name: command,
        style: { text: command, size: "auto", color: 0xffffff, bgcolor: 0x203040 },
        steps: [{ down: [{ actionId: "navigate", options: { command } }], up: [] }],
        feedbacks: [{ feedbackId: "connected", options: {}, style: { bgcolor: 0x187038 } }],
      },
    ]),
  );
  const common = Object.keys(definitions);
  const dynamic = [];
  for (const target of targets(state)) {
    const id = `target-${Buffer.from(target.id).toString("base64url")}`;
    const payload = JSON.parse(target.id);
    const feedbacks = [];
    if (["cue", "libraryCue", "playlistCue"].includes(payload.command))
      feedbacks.push({
        feedbackId: "cue",
        options: {
          id: payload.slideId,
          workspacePath: payload.workspacePath,
          presentationId: payload.presentationId,
          playlistEntryId: payload.playlistEntryId ?? "",
          occurrence: payload.cueInstanceId ?? "",
        },
        style: { bgcolor: 0x187038 },
      });
    if (payload.command === "timer")
      feedbacks.push({
        feedbackId: "timer",
        options: { id: payload.id, workspacePath: payload.workspacePath },
        style: { bgcolor: 0x187038 },
      });
    definitions[id] = {
      type: "simple",
      name: target.label,
      style: { text: target.label, size: "auto", color: 0xffffff, bgcolor: 0x203040 },
      steps: [{ down: [{ actionId: "target", options: { target: target.id } }], up: [] }],
      feedbacks,
    };
    dynamic.push(id);
  }
  return [
    [
      { id: "orbit", name: "Presenter Orbit", definitions: common },
      { id: "targets", name: "Workspace targets", definitions: dynamic },
    ],
    definitions,
  ];
}

module.exports = { actions, feedbacks, presets };
