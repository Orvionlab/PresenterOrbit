const fs = require("node:fs");
const path = require("node:path");
if (!fs.existsSync("src-tauri/tauri.conf.json"))
  throw new Error("Run this fixture generator from the Orbit repository root.");
const root = path.resolve(".tmp/ctl14/workspace");
function write(file, data) {
  const target = path.join(root, file);
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.writeFileSync(target, JSON.stringify(data));
}
const timers = [
  { id: "ctl14-timer", name: "Companion Timer", durationSeconds: 60, allowsOverrun: false, type: "countdown" },
];
write("workspace.json", {
  schema: "presenter-orbit.workspace",
  version: 1,
  id: "ctl14-workspace",
  title: "Companion Acceptance",
  storage: { layout: "folders" },
  clearGroups: [],
  communicationDevices: [],
  mediaEffectPresets: [],
  transitionPresets: [],
  triggerBindings: [],
  importWarnings: [],
});
write("Libraries/index.json", { libraries: [{ id: "ctl14-library", title: "Companion Test", folder: "Test" }] });
write("Libraries/Test/library.json", { id: "ctl14-library", title: "Companion Test", items: ["test.json"] });
write("Libraries/Test/test.json", {
  id: "ctl14-presentation",
  title: "Companion Presentation",
  slides: [
    { id: "ctl14-slide-1", label: "First", content: "Companion first cue" },
    { id: "ctl14-slide-2", label: "Second", content: "Companion second cue" },
  ],
});
write("Playlists/index.json", { playlists: [] });
write("Media/index.json", { libraries: [], media: [] });
write("Themes/index.json", { themes: [] });
write("Masks/index.json", { masks: [] });
write("Configuration/Props/Props", {
  schemaVersion: 1,
  props: [],
  propFolders: [],
  countdownTimers: timers,
  macros: [
    {
      id: "ctl14-macro",
      title: "Companion Clear",
      actions: [{ id: "clear-props", type: "clear", targetLayer: "prop", enabled: true }],
    },
  ],
  macroCollections: [],
  messages: [],
  plans: [],
  scheduledEvents: [],
});
write("Configuration/Stage/Stage.json", {
  schemaVersion: 1,
  stageLayouts: [],
  stageScreenAssignments: [],
  stageMessage: { text: "", state: "cleared" },
  audienceLooks: [],
  videoInputs: [],
  audioInputs: [],
  recordingPresets: [],
  tickers: [],
});
write("Configuration/Settings.json", {
  schemaVersion: 1,
  screenConfig: {},
  countdownTimers: timers,
  groupSettings: {},
  groups: [],
  keyMappings: {},
});
const workspace = path.join(root, "workspace.json");
const expression = `(async () => {
  const bridge=window.presenterOrbit;
  await bridge.loadWorkspace(${JSON.stringify(workspace)});
  const settings=await bridge.getSettings();
  await bridge.saveSettings({...settings,presenterOrbitWorkspacePath:${JSON.stringify(workspace)},remoteSettings:{enabled:true,port:5194,password:'ctl14-synthetic-test-only'}});
  return {ready:true,workspace:${JSON.stringify(workspace)}};
})()`;
fs.writeFileSync(".tmp/ctl14/setup-webview.js", expression);
fs.writeFileSync(".tmp/ctl14/password.txt", "ctl14-synthetic-test-only");
