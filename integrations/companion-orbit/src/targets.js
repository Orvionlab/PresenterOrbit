const choice = (label, payload) => ({ id: JSON.stringify(payload), label });
const rows = (value) => (Array.isArray(value) ? value : []);

function targets(status) {
  if (!status?.workspacePath || status.controlsError) return [];
  const workspacePath = status.workspacePath;
  const result = [];
  const add = (label, payload) => result.push(choice(label, { ...payload, workspacePath }));
  const cues = (prefix, slides, payload) => {
    for (const slide of rows(slides))
      if (!slide.disabled)
        add(`${prefix} / ${slide.index || ""} ${slide.title || slide.text || slide.id}`, {
          ...payload,
          slideId: slide.id,
          cueInstanceId: slide.cueInstanceId ?? null,
        });
  };
  if (status.presentation)
    cues("Live cue", status.presentation.slides, {
      command: "cue",
      presentationId: status.presentation.id,
      playlistEntryId: status.presentation.playlistEntryId ?? null,
    });
  for (const library of rows(status.libraries))
    for (const presentation of rows(library.presentations)) {
      const payload = {
        command: "libraryCue",
        libraryId: library.id,
        presentationId: presentation.id,
        playlistEntryId: null,
      };
      cues(`${library.title} / ${presentation.title}`, presentation.slides, payload);
      for (const arrangement of rows(presentation.arrangements))
        cues(`${library.title} / ${presentation.title} / ${arrangement.title}`, arrangement.slides, {
          ...payload,
          arrangementId: arrangement.id,
        });
    }
  for (const playlist of rows(status.playlists))
    for (const entry of rows(playlist.entries))
      cues(`${playlist.title} / ${entry.title} (${entry.id})`, entry.slides, {
        command: "playlistCue",
        playlistId: playlist.id,
        playlistEntryId: entry.id,
        presentationId: entry.presentationId,
      });
  for (const [field, command] of [
    ["audienceLooks", "audienceLook"],
    ["macros", "macro"],
    ["clearGroups", "clearGroup"],
  ])
    for (const entry of rows(status[field]))
      if (entry.enabled !== false) add(`${command} / ${entry.title}`, { command, id: entry.id });
  for (const timer of rows(status.timers))
    for (const action of ["start", "stop", "reset"])
      add(`Timer / ${timer.name} / ${action}`, { command: "timer", id: timer.id, action });
  for (const prop of rows(status.props))
    for (const action of ["show", "clear"])
      add(`Prop / ${prop.title} / ${action}`, { command: "prop", id: prop.id, action });
  for (const message of rows(status.messages))
    if (message.enabled !== false)
      add(`Message / ${message.title}`, { command: "message", id: message.id, action: "show" });
  add("Message / Clear", { command: "message", action: "clear" });
  for (const screen of rows(status.outputs))
    if (screen.type === "stage")
      for (const layout of rows(status.stageLayouts))
        add(`Stage layout / ${screen.label} / ${layout.title}`, {
          command: "stageLayout",
          screenId: screen.id,
          id: layout.id,
        });
  for (const bin of rows(status.mediaBins))
    for (const item of rows(bin.items))
      add(`Media / ${bin.title} / ${item.title}`, { command: "media", id: item.id, action: "trigger" });
  for (const preset of rows(status.recordingPresets))
    if (preset.enabled) add(`Record / ${preset.title}`, { command: "capture", id: preset.id, action: "start" });
  add("Record / Stop", { command: "capture", action: "stop" });
  return [...new Map(result.map((entry) => [entry.id, entry])).values()];
}

function selectedPayload(status, value) {
  const target = targets(status).find((entry) => entry.id === value);
  if (!target) throw new Error("This target is unavailable or its workspace/occurrence changed. Select it again.");
  return JSON.parse(target.id);
}

function mediaPayload(status, layer, action, position) {
  if (!status?.workspacePath) throw new Error("An active Orbit workspace is required.");
  const selection = layer === "audio" ? status.audio : status.background;
  if (!selection?.id || selection.selectionId == null) throw new Error("No current media occurrence is available.");
  const payload = {
    command: "media",
    workspacePath: status.workspacePath,
    id: selection.id,
    selectionId: selection.selectionId,
    action,
  };
  if (action === "seek") {
    if (!Number.isFinite(position) || position < 0 || position > 1)
      throw new Error("Seek position must be between 0 and 1.");
    payload.position = position;
  }
  return payload;
}

module.exports = { targets, selectedPayload, mediaPayload };
