import streamDeck, { action, SingletonAction } from "@elgato/streamdeck";
import { OrbitControls } from "./control.mjs";

const controls = new OrbitControls((payload) => streamDeck.ui.sendToPropertyInspector(payload));
class Control extends SingletonAction {
  onWillAppear(ev) {
    controls.appear(ev.action, ev.payload.settings);
  }
  onWillDisappear(ev) {
    controls.disappear(ev.action);
  }
  onDidReceiveSettings(ev) {
    controls.update(ev.action, ev.payload.settings);
  }
  onKeyDown(ev) {
    return controls.press(ev.action);
  }
  onPropertyInspectorDidAppear() {
    controls.publish();
  }
  onSendToPlugin(ev) {
    if (ev.payload?.type === "catalog") controls.publish();
  }
}
const ControlAction = action({ UUID: "com.presenterorbit.control.command" })(Control);
streamDeck.actions.registerAction(new ControlAction());
streamDeck.settings.onDidReceiveGlobalSettings((ev) => controls.configure(ev.settings));
await streamDeck.connect();
controls.configure(await streamDeck.settings.getGlobalSettings());
