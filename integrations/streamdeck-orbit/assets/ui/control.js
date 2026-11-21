/* global WebSocket */
(() => {
  const field = (id) => document.getElementById(id);
  let socket,
    context,
    action,
    settings = {},
    globalSettings = {},
    catalog = [];
  const send = (event, payload, target = context) => {
    if (socket?.readyState === WebSocket.OPEN)
      socket.send(
        JSON.stringify({
          event,
          context: target,
          ...(payload === undefined ? {} : { payload }),
          ...(event === "sendToPlugin" ? { action } : {}),
        }),
      );
  };
  function render() {
    for (const id of ["mode", "command", "layer"]) field(id).value = settings[id] || "";
    field("commandRow").hidden = settings.mode !== "navigate";
    field("targetRow").hidden = settings.mode !== "target";
    field("layerRow").hidden = settings.mode !== "clearLayer";
    const select = field("target");
    select.replaceChildren();
    const add = (label, value) => {
      const option = document.createElement("option");
      option.textContent = label;
      option.value = value;
      select.append(option);
    };
    add("Choose exact target…", "");
    if (settings.target && !catalog.some((item) => item.id === settings.target))
      add("Saved target unavailable — select again", settings.target);
    for (const target of catalog) add(target.label, target.id);
    select.value = settings.target || "";
  }
  for (const id of ["mode", "command", "layer", "target", "label"])
    field(id).addEventListener("change", () => {
      settings = { ...settings, [id]: field(id).value };
      if (id === "mode" && settings.mode === "navigate" && !settings.command) settings.command = "nextSlide";
      if (id === "mode" && settings.mode === "clearLayer" && !settings.layer) settings.layer = "slide";
      send("setSettings", settings);
      render();
    });
  field("connection").addEventListener("submit", (event) => {
    event.preventDefault();
    if (!field("connection").reportValidity()) return;
    globalSettings = {
      ...globalSettings,
      host: field("host").value.trim(),
      port: Number(field("port").value),
      password: field("password").value,
      pollInterval: Number(field("pollInterval").value),
    };
    send("setGlobalSettings", globalSettings);
    field("status").textContent = "Connecting to Orbit…";
  });
  window.connectElgatoStreamDeckSocket = (port, uuid, registerEvent, info, actionInfo) => {
    const details = JSON.parse(actionInfo);
    context = uuid;
    action = details.action;
    settings = details.payload.settings;
    for (const id of ["mode", "command", "layer", "label"]) field(id).value = settings[id] || "";
    render();
    socket = new WebSocket(`ws://127.0.0.1:${port}`);
    socket.onopen = () => {
      socket.send(JSON.stringify({ event: registerEvent, uuid }));
      send("getGlobalSettings");
      send("sendToPlugin", { type: "catalog" });
    };
    socket.onmessage = ({ data }) => {
      const message = JSON.parse(data);
      if (message.event === "didReceiveGlobalSettings") {
        globalSettings = message.payload.settings;
        for (const id of ["host", "port", "password", "pollInterval"]) field(id).value = globalSettings[id] ?? "";
      }
      if (message.event === "sendToPropertyInspector") {
        const payload = message.payload;
        if (payload.type === "catalog") {
          catalog = payload.targets;
          field("status").textContent = payload.connected
            ? `Connected · ${payload.workspace || "No workspace"}`
            : payload.error;
          render();
        }
        if (payload.type === "result") field("result").textContent = payload.error || "Command completed.";
      }
    };
    socket.onclose = () => {
      field("status").textContent = "Stream Deck disconnected.";
    };
  };
})();
