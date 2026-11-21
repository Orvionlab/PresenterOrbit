# Phase 15 React parity inventory

Started 2026-11-07. Done means the named implementation exists and, where it can run without a display, a test exercises it; what only a person or hardware can check is listed as hand acceptance. Implementation and hand acceptance are recorded separately. Rows are not marked ahead of the code.

**Status: implemented, acceptance pending.** All four groups now have GPUI counterparts. Group 4 adds timeline authoring/transport/recording, timecode input and chase, device trigger runtimes and remote command execution, and closes the operator deviations listed below. Part (a) has no unresolved method: every Used entry has a production call/event receiver; Unused entries explain the native or semantic replacement. Hardware and visual acceptance remain explicitly unverified.

Ownership decisions (which process runs what) are in the README decision log.

## (a) Bridge walk

Walked from `src/shared/presenter-orbit.ts` and the bridge types it composes (`src/shared/*.ts`). Method names were extracted from the type declarations by pattern, then checked by searching `crates/orbit-app/src` for each name, plus a manual pass for the event listeners (`on...`), which GPUI receives as host event names. A method counts as Used when GPUI source calls it, or receives the event; a name found only in a comment or test would be miscounted, and the extraction can miss a method declared in an unusual form. Counts below are from the final run of the walk script.

| Bridge method | Declared in | State | Where or why |
| --- | --- | --- | --- |
| `acceptCommunicationInput` | communication-input.ts | Used | `automation/inputs.rs`: host acceptance precedes one controller dispatch. |
| `applyAudioSettings` | output-scene-bridge.ts | Used | `settings/audio.rs`, `settings/persist.rs` |
| `applyScreenConfig` | output-scene-bridge.ts | Unused | GPUI sends the semantic `mutateScreenConfig` intents (`output/windows.rs`, `output/screen_canvas.rs`), never the whole config. |
| `applyWorkspaceSync` | workspace-sync.ts | Used | `settings/sync.rs` |
| `auxiliaryWindowReady` | auxiliary-windows.ts | Unused | Tauri webview plumbing; the GPUI windows are native (`auxiliary.rs`). |
| `Boolean` | slide-composition.ts | Unused | false positive, a TypeScript type name. |
| `checkAppUpdate` | app-updates.ts | Used | `settings/services.rs`, `settings/tests.rs` |
| `chooseChordChart` | chord-chart.ts | Used | `charts.rs`; existing PDF/image host chooser. |
| `cleanUnusedMedia` | media-workspace-bridge.ts | Used | `media/maintenance.rs`, `media/view.rs` |
| `clearPlanningCenterCredentials` | planning-center.ts | Used | `settings/services.rs` |
| `closeAuxiliaryWindow` | auxiliary-windows.ts | Unused | Native auxiliary closing hides the window through `AuxiliaryWindows`; the old match was test-only. |
| `closeDisplayOutput` | output-scene-bridge.ts | Used | `native_outputs/linux.rs`, `native_outputs/macos.rs` |
| `connectCommunicationInput` | communication-input.ts | Used | `settings/devices.rs` |
| `controlExternalPresentation` | external-presentation.ts | Used | `external.rs`, controller session and strip. |
| `controlPlayback` | output-scene-bridge.ts | Used | `automation/transport.rs`, sent to the output process (clock commands only, see the allow-list). Art-Net transport uses the same output route. |
| `controlTicker` | ticker.ts | Used | `tickers.rs`, `services.rs`. |
| `controlTimer` | output-scene-bridge.ts | Used | automation/execute.rs (timer action) |
| `createBiblePresentation` | bible.ts | Used | `bibles.rs` (controller-side, background calls). |
| `createWindowedOutput` | output-window.ts | Used | `output/windows.rs`, `output_process/protocol.rs` |
| `deleteCapturePreset` | capture.ts | Used | `output_process/server_tests.rs`, `settings/capture.rs` |
| `deleteTheme` | workspace-content-bridge.ts | Used | `canvas/inspector.rs` |
| `disconnectCommunicationInput` | communication-input.ts | Used | `settings/devices.rs` |
| `discoverNetworkLinkPeers` | network-link-membership.ts | Used | `settings/membership.rs` |
| `downloadPlanningCenterAttachments` | planning-center.ts | Used | `planning.rs`, `services.rs`; credentials remain in host. |
| `exportDiagnostics` | presenter-orbit.ts | Used | `settings/general.rs` |
| `exportHandout` | handout.ts | Used | `transfers.rs`, controller-only reviewed host path. |
| `exportPresentation` | presenter-orbit.ts | Used | `transfers.rs`, host reviewed destination and export. |
| `exportPresentationBundle` | portable-bundle.ts | Used | `imports.rs`; original validation and host review/report dialogs retained. |
| `exportProPresenterWorkspace` | presenter-orbit.ts | Used | `settings/services.rs` |
| `fetchRssFeed` | presenter-orbit.ts | Used | `canvas/inspector.rs` |
| `fetchRssFeeds` | presenter-orbit.ts | Unused | Ticker RSS is resolved by the host cache through `getTickerStatus`; no separate renderer feed request is needed. |
| `focusWindowedOutput` | output-window.ts | Used | `output_process/protocol.rs` |
| `forgetMissingMedia` | media-workspace-bridge.ts | Used | `media/maintenance.rs`, `media/view.rs` |
| `getAppUpdateStatus` | app-updates.ts | Used | `settings/services.rs` |
| `getAudioInputStatus` | audio-input.ts | Used | `settings/audio_inputs.rs`: read-only output-link status and peaks. |
| `getBibleBooks` | bible.ts | Used | `bibles.rs` (controller-side, background calls). |
| `getBiblePassage` | bible.ts | Used | `bibles.rs` (controller-side, background calls). |
| `getCaptureStatus` | capture.ts | Used | `output/capture.rs`, `output_process/protocol.rs` |
| `getCommunicationInputStatus` | communication-input.ts | Used | `automation/inputs.rs`: initial status snapshot. |
| `getExternalPresentationStatus` | external-presentation.ts | Used | `external.rs`, controller session and strip. |
| `getLibraryItemLocation` | presenter-orbit.ts | Unused | `showItemInFolder` is used for the reveal action; the item path is resolved by the host. |
| `getMediaEffectCatalog` | media-effects.ts | Used | `canvas/inspector.rs`, `media/view.rs` |
| `getMidiInputStatus` | midi-input.ts | Used | `settings/triggers.rs` |
| `getNativeMediaPlayback` | media-workspace-bridge.ts | Used | `output_process/protocol.rs` |
| `getNetworkLinkSettings` | network-link.ts | Used | `settings/network_link.rs` |
| `getNetworkLinkStatus` | network-link.ts | Used | `settings/network_link.rs` |
| `getOutputOptions` | output-scene-bridge.ts | Used | `output/windows.rs`, `output_process/protocol.rs` |
| `getPlanningCenterLive` | planning-center.ts | Used | `planning.rs`, `services.rs`; credentials remain in host. |
| `getPlanningCenterStatus` | planning-center.ts | Used | `settings/services.rs` |
| `getRemoteNetworkAddress` | presenter-orbit.ts | Used | `settings/remote.rs` |
| `getSettings` | presenter-orbit.ts | Used | `appearance.rs`, `auxiliary.rs` |
| `getTickerStatus` | ticker.ts | Used | `tickers.rs`, `services.rs`. |
| `getTimecodeInputConfig` | timeline-audio.ts | Used | `settings/timecode.rs`. |
| `getTimecodeStatus` | timeline-audio.ts | Used | `settings/timecode.rs`. |
| `getTimelineWaveform` | timeline-audio.ts | Used | `media/view.rs` |
| `getTransitionCatalog` | transitions.ts | Used | `canvas/inspector.rs` |
| `renderTransitionPreview` | transitions.ts | Unused | `orbit_host::native_transitions::render_preview` is called directly (`transitions/picker.rs`). |
| `getVersion` | app-updates.ts | Unused | About reads `env!("CARGO_PKG_VERSION")` (`shell.rs`), no host call. |
| `importBiblePackage` | bible.ts | Used | `bibles.rs` (controller-side, background calls). |
| `importKeynotePresentation` | keynote-import.ts | Used | `imports.rs`; platform-gated native import, workspace lane and canonical selection. |
| `importPowerPointImages` | powerpoint-import.ts | Used | `imports.rs`; platform-gated native import, workspace lane and canonical selection. |
| `importPresentationBundle` | portable-bundle.ts | Used | `imports.rs`; original validation and host review/report dialogs retained. |
| `installAppUpdate` | app-updates.ts | Used | `settings/services.rs` |
| `installWebBible` | bible.ts | Used | `bibles.rs` (controller-side, background calls). |
| `joinNetworkLinkGroup` | network-link-membership.ts | Used | `settings/membership.rs` |
| `listAudioInputDevices` | audio-input.ts | Used | `settings/audio_inputs.rs` |
| `listBibles` | bible.ts | Used | `bibles.rs` (controller-side, background calls). |
| `listCameraDevices` | domain-contracts.ts | Used | `media/video_inputs.rs` |
| `listInstalledFonts` | presenter-orbit.ts | Used | `canvas/inspector.rs` |
| `listMasks` | presenter-orbit.ts | Unused | masks arrive in the workspace snapshot. |
| `listMidiOutputPorts` | communication.ts | Used | `settings/devices.rs` |
| `listMissingMedia` | media-workspace-bridge.ts | Used | `media/maintenance.rs` |
| `listNdiSources` | domain-contracts.ts | Used | `canvas/inspector.rs`, `media/video_inputs.rs` |
| `listPlanningCenterAttachments` | planning-center.ts | Used | `planning.rs`, `services.rs`; credentials remain in host. |
| `listPlanningCenterPlans` | planning-center.ts | Used | `planning.rs`, `services.rs`; credentials remain in host. |
| `listPlanningCenterServiceTypes` | planning-center.ts | Used | `planning.rs`, `services.rs`; credentials remain in host. |
| `listProps` | presenter-orbit.ts | Unused | props arrive in the workspace snapshot. |
| `listSerialPorts` | communication.ts | Used | `settings/devices.rs` |
| `listStage` | presenter-orbit.ts | Unused | stage layouts arrive in the workspace snapshot. |
| `listThemes` | presenter-orbit.ts | Unused | themes arrive in the workspace snapshot (`readWorkspaceSnapshot`). |
| `listTimecodeDevices` | timeline-audio.ts | Used | `settings/timecode.rs`. |
| `listWindowedOutputs` | output-window.ts | Used | `output_process/protocol.rs` |
| `loadWorkspace` | presenter-orbit.ts | Used | `startup.rs`, `collections/scale_tests.rs` |
| `mutateCountdownTimers` | presenter-orbit.ts | Used | automation/execute.rs (timer configuration) |
| `mutateGroupSettings` | presenter-orbit.ts | Used | `settings/groups.rs`, `settings/persist.rs` |
| `mutateKeyMappings` | key-mapping-mutations.ts | Used | `settings/persist.rs`, `settings/shortcuts.rs` |
| `mutateMasks` | mask-mutations.ts | Used | `canvas/content_document.rs`, `canvas/documents_ui.rs` |
| `mutateMediaLibrary` | media-workspace-bridge.ts | Used | `media/menu.rs`, `media/tree.rs` |
| `mutatePresentationLibrary` | presenter-orbit.ts | Used | `canvas/content_document.rs`, `canvas/documents_ui.rs` |
| `mutatePresentationPlaylist` | presenter-orbit.ts | Used | `collections/model.rs`, `collections/view_tests.rs` |
| `mutateProps` | presenter-orbit.ts | Used | `canvas/content_document.rs`, `canvas/documents_ui.rs` |
| `mutateScreenConfig` | output-scene-bridge.ts | Used | `output/screen_canvas.rs`, `output/windows.rs` |
| `mutateStageContent` | stage-content-mutations.ts | Used | `canvas/content_document.rs`, `canvas/documents_ui.rs` |
| `onAudioInputsChanged` | audio-input.ts | Used | Used as the `audio-inputs-changed` event, `view.rs`. |
| `onAutomationDue` | automation-bridge.ts | Used | the `automation-due` host event, `view.rs` `on_host_event` -> `automation::on_due`. |
| `onCommunicationDevicesChanged` | communication-input.ts | Used | `communication-devices-changed` receiver in `automation/inputs.rs`. |
| `onCommunicationInput` | communication-input.ts | Used | `communication-input` receiver in `automation/inputs.rs`. |
| `onCommunicationInputStatus` | communication-input.ts | Used | `communication-input-status` receiver in `automation/inputs.rs`. |
| `onMidiInputStatus` | midi-input.ts | Used | `midi-input-status` receiver in `automation/inputs.rs`. |
| `onMidiNote` | midi-input.ts | Used | `midi-note` receiver in `automation/inputs.rs`. |
| `onMusicPlaybackEnded` | presenter-orbit.ts | Used | Used as `presenter-orbit:music-playback-ended`, `view.rs` -> `media.ended`. |
| `onNativeOutputStatePatch` | output-scene-bridge.ts | Used | Used as the `native-output-state-patch` event, `view.rs`. |
| `onOutputClosed` | output-scene-bridge.ts | Used | Used as the `output-closed` event, `view.rs` -> `output::windows::closed`. |
| `onOutputSceneCommitted` | output-scene-bridge.ts | Used | Used as the `output:scene-committed` event, `view.rs`. |
| `onRemoteCommand` | presenter-orbit.ts | Used | `remote-command` receiver in `view.rs`, `automation/remote.rs`. |
| `onScreenConfigChanged` | output-scene-bridge.ts | Unused | screen config edits come back in the `mutateScreenConfig` answer and the settings revision; GPUI has one window family per process. Not traced for a second GPUI process (the network link covers peers). |
| `onSettingsChanged` | presenter-orbit.ts | Used | `view.rs`, `state/revisions.rs` |
| `onSettingsPanelRequested` | presenter-orbit.ts | Used | `settings-panel-requested` receiver in `view.rs`. |
| `onSpellContext` | spellcheck.ts | Unused | WebView-only event, replaced by rich-text selection and corpus suggestions. |
| `onTimecodeReading` | timeline-audio.ts | Used | `timecode-reading` receiver in `automation/timecode.rs`. |
| `onTimecodeStatus` | timeline-audio.ts | Used | `timecode-status` receiver in `automation/timecode.rs`. |
| `onTimelineSoundtrackStatus` | timeline-audio.ts | Used | `timeline-soundtrack-status` receiver in `view.rs`. |
| `onTriggerBindingsChanged` | automation-bridge.ts | Used | Used as the `trigger-bindings-changed` event, `view.rs` (revision bookkeeping only). |
| `onTriggerBindingTest` | automation-bridge.ts | Used | `trigger-binding-test` receiver in `automation/inputs.rs`. |
| `onWindowedOutputsChanged` | output-scene-bridge.ts | Unused | output windows are listed after each `createWindowedOutput` and `closeDisplayOutput` answer (`output/windows.rs`), not by event. |
| `onWorkspaceSyncChanged` | workspace-sync.ts | Used | Used as the `workspace-sync-changed` event, `view.rs`. |
| `openAuxiliaryWindow` | auxiliary-windows.ts | Used | `shell_tests.rs`, `view.rs` |
| `openExternalUrl` | presenter-orbit.ts | Used | `bibles.rs`, translation license links. |
| `openLogFiles` | presenter-orbit.ts | Used | `auxiliary.rs` |
| `openSettingsWindow` | auxiliary-windows.ts | Unused | windows open natively (`windows.rs`, `AuxiliaryWindows`); the host method `openAuxiliaryWindow` is used. |
| `prepareAuxiliaryWindows` | auxiliary-windows.ts | Unused | Tauri webview plumbing; `AuxiliaryWindows::prepare` is native. |
| `preservePresentationTextStyleParagraphOverrides` | presentation-text-paragraph-preservation.ts | Unused | a TypeScript helper in `src/shared`, not a bridge method. |
| `previewHandout` | handout.ts | Used | `transfers.rs`, controller-only reviewed host path. |
| `previewWorkspaceSync` | workspace-sync.ts | Used | `settings/sync.rs` |
| `printHandout` | handout.ts | Used | `transfers.rs`, controller-only reviewed host path. |
| `probeMediaMetadata` | workspace-content-bridge.ts | Used | `canvas/inspector.rs`, `media/assets.rs` |
| `readClipboardText` | spellcheck.ts | Used | `imports.rs`: File → Import → Text from Clipboard; rich-text clipboard actions remain native. |
| `readLocalTextWatch` | presenter-orbit.ts | Used | `state/text_files.rs` |
| `readWorkspaceSnapshot` | workspace-content-bridge.ts | Used | `canvas/content_document.rs`, `canvas/document.rs` |
| `releaseMidiOutputNotes` | communication.ts | Used | `settings/devices.rs` |
| `relinkMissingMedia` | media-workspace-bridge.ts | Used | `media/maintenance.rs`, `media/view.rs` |
| `remove` | presentation-text-paragraph-preservation.ts | Unused | Extraction false positive: a TypeScript collection method, not a bridge call. |
| `removeBible` | bible.ts | Used | `bibles.rs` (controller-side, background calls). |
| `renderCameraInputFrame` | domain-contracts.ts | Unused | `host.render_camera_input_frame` is called directly (`media/video_inputs_view.rs`). |
| `renderMediaEffectPreview` | media-effects.ts | Unused | `host.render_media_effect_preview` is called directly (`media/assets.rs`). |
| `renderNdiInputFrame` | domain-contracts.ts | Unused | `host.render_ndi_input_frame` is called directly (`media/video_inputs_view.rs`). |
| `renderSlideStill` | workspace-content-bridge.ts | Unused | `orbit_host::native_compositor::render_slide_still` is called directly (`output/preview.rs`). |
| `replaceLibrarySearchIndex` | library-search.ts | Used | `collections/model.rs`, `collections/search.rs` |
| `replaceMisspelling` | spellcheck.ts | Unused | GPUI edits rich-text input directly; shared host returns false for WebView replacement. |
| `reportRemoteCommandResult` | presenter-orbit.ts | Used | `automation/remote.rs`: dispatched/completed/error after runner settlement. |
| `resetVideoThumbnail` | media-workspace-bridge.ts | Used | `media/inspector.rs`, `media/view.rs` |
| `resolveBibleReference` | bible.ts | Used | `bibles.rs` (controller-side, background calls). |
| `reviewMediaCleanup` | media-workspace-bridge.ts | Used | `startup.rs`, `media/maintenance.rs` |
| `saveCapturePreset` | capture.ts | Used | `output/capture.rs`, `output_process/server_tests.rs` |
| `saveLibraryItem` | presenter-orbit.ts | Used | `collections/song_metadata.rs` |
| `saveLibraryItems` | presenter-orbit.ts | Unused | GPUI saves one item (`saveLibraryItem`) or sends semantic library mutations. |
| `saveMask` | presenter-orbit.ts | Unused | Canvas saves semantic `mutateMasks/updateDocument` (`canvas/content_document.rs`), with full document metadata preserved. |
| `saveMediaLibraries` | media-workspace-bridge.ts | Used | `media/gesture.rs`, `media/view.rs` |
| `saveMediaLibrary` | media-workspace-bridge.ts | Used | `media/view.rs` |
| `saveMediaSettings` | media-workspace-bridge.ts | Used | `media/effects.rs`, `media/gesture.rs` |
| `saveMediaState` | media-workspace-bridge.ts | Unused | media edits go through `mutateMediaLibraries`/`saveWorkspaceContent` (`media/model.rs`, `media/effects.rs`). |
| `saveNetworkLinkSettings` | network-link.ts | Used | `settings/network_link.rs` |
| `saveProps` | presenter-orbit.ts | Unused | Canvas and prop panels use semantic `mutateProps` (`canvas/content_document.rs`, `automation/props.rs`). |
| `saveSettings` | presenter-orbit.ts | Used | `shell_tests.rs`, `output_process/client_tests.rs` |
| `saveStage` | presenter-orbit.ts | Unused | Canvas and stage panel use semantic `mutateStageContent/replaceLayout` (`canvas/content_document.rs`, `automation/stage.rs`). |
| `saveTheme` | presenter-orbit.ts | Used | `canvas/document.rs`, `canvas/inspector/tests.rs` |
| `saveTickers` | ticker.ts | Used | `tickers.rs`, `services.rs`. |
| `saveWorkspaceContent` | presenter-orbit.ts | Used | `canvas/presets.rs`, `collections/scale_tests.rs` |
| `scheduleAutomationActions` | automation-bridge.ts | Used | automation/execute.rs (delays and markers) |
| `searchBible` | bible.ts | Used | `bibles.rs` (controller-side, background calls). |
| `searchLibraryIndex` | library-search.ts | Used | `collections/search.rs` |
| `selectCaptureDirectory` | capture.ts | Used | `output_process/server_tests.rs`, `settings/capture.rs` |
| `selectLogoImage` | presenter-orbit.ts | Used | `settings/general.rs`, `settings/patterns.rs` |
| `selectMediaSmartPlaylistFolder` | presenter-orbit.ts | Used | `media/view.rs` |
| `selectOrbitWorkspaceFolder` | presenter-orbit.ts | Used | `settings/general.rs` |
| `selectPresentationTextFile` | presenter-orbit.ts | Used | `canvas/inspector.rs`, `imports.rs`: File → Import → Text File. |
| `scanProPresenterWorkspace` | (new) | Used | `settings/services.rs`: picks the folder and lists what an import would create; read-only. |
| `selectProPresenterWorkspace` | presenter-orbit.ts | Used | `settings/services.rs`: takes the folder path and the import selection. |
| `selectSyncRepository` | workspace-sync.ts | Used | `settings/sync.rs` |
| `selectThemeAssetFile` | presenter-orbit.ts | Used | `canvas/inspector.rs` |
| `sendCommunication` | communication.ts | Used | `automation/execute.rs`, `settings/devices.rs` |
| `sendNetworkLinkCue` | network-link.ts | Used | `cues/actions.rs`, `automation/link.rs`: authored library/playlist occurrence indexes. |
| `setDisplayOutputVisibility` | output-scene-bridge.ts | Used | `shell.rs`, `output/scene.rs` |
| `setMidiInputEnabled` | midi-input.ts | Used | `automation/inputs.rs`: enabled while a binding requires MIDI. |
| `setPlanningCenterCredentials` | planning-center.ts | Used | `settings/services.rs`, `settings/tests.rs` |
| `setPlaylistPlanningCenterLink` | planning-center.ts | Used | `planning.rs`, `services.rs`; credentials remain in host. |
| `setTimecodeInputConfig` | timeline-audio.ts | Used | `settings/timecode.rs`; controller-owned settings and devices. |
| `setVideoThumbnail` | media-workspace-bridge.ts | Used | `media/inspector.rs`, `media/view.rs` |
| `showItemInFolder` | output-window.ts | Used | `media/menu.rs` |
| `startCapture` | capture.ts | Unused | `output/capture.rs` resolves the preset in the controller then calls `startCaptureResolved` on the output link. The old match was test-only. |
| `startExternalPresentation` | external-presentation.ts | Used | `automation/execute.rs` |
| `startLocalTextWatch` | presenter-orbit.ts | Used | `state/text_files.rs` |
| `startPlanningCenterLive` | planning-center.ts | Used | `planning.rs`, `services.rs`; credentials remain in host. |
| `stopCapture` | capture.ts | Used | `output/capture.rs`, `output_process/protocol.rs` |
| `stopLocalTextWatch` | presenter-orbit.ts | Used | `state/text_files.rs` |
| `stopPlanningCenterLive` | planning-center.ts | Used | `planning.rs`, `services.rs`; credentials remain in host. |
| `syncPlaylistFromPlanningCenter` | planning-center.ts | Used | `planning.rs`, `services.rs`; credentials remain in host. |
| `testPlanningCenterConnection` | planning-center.ts | Used | `settings/services.rs` |
| `testTriggerBinding` | automation-bridge.ts | Used | `settings/triggers.rs` |
| `updateAutomationSchedule` | automation-bridge.ts | Used | automation/mod.rs (session start, marker cancellation) |
| `updateOutputScene` | output-scene-bridge.ts | Used | `output/protocol.rs`, `output/scene.rs` |
| `while` | content-runs.ts | Unused | Extraction false positive: a TypeScript keyword, not a bridge call. |
| `windowCommand` | output-window.ts | Unused | window commands (minimize, maximize, close) are native window calls in `main_window.rs`. |
| `writeClipboardText` | spellcheck.ts | Unused | GPUI editor native clipboard actions. |

Bridge audit: 192 extracted entries, including the documented extraction false positives. Counts below were recomputed after excluding comments and test modules; every Used method has a production call literal or the documented host event receiver. `getNativeMusicPlayback` is an additional GPUI-only read call, outside the unchanged TypeScript bridge contract. The totals are 160 Used, 32 Unused with a stated reason, and 0 not done.

## (b) File-by-file checklist

Code is in `crates/orbit-app/src/automation/` (`runner.rs`, `execute.rs`, `due.rs`, `markers.rs`, `panels.rs`, `mod.rs`; tests in `tests.rs` and each module) unless stated. Host additions: `getMarkerPositions` and `acceptMarkerPositions` in `crates/orbit-host/src/native_render.rs` and the dispatcher.

### Action runner and automation runtime (`features/app`)

| React file | User-reachable behavior | State in GPUI |
| --- | --- | --- |
| `automation-actions.ts`, `automation-run.ts` | Run actions in order, skip disabled, nesting depth limit 8, marker identity guard, per-action failure | Done: `runner::run_batch` (tests `a_disabled_action_is_skipped_*`, `macros_nest_in_order_and_stop_at_the_depth_limit`, `a_cue_runs_before_the_actions_that_follow_it`). The first failure of a run is shown on the status line; Remote macro completion is observed by `remote.rs`, including nested delays and native failures. |
| `automation-action-presentation.ts` | clear, clear-group, macro, presentation, presentation-slide, slide-destination, playlist, mask, audience-look, video-input | Done: a test for each handler. A presentation or playlist action runs the host cue command (`libraryCue`, `playlistCue`) as its own step, in order. `contentDestination == 1` starts the announcements player manual on that slide without touching the main presentation (`runner::announce`, test `the_announcements_destination_starts_the_independent_player_on_that_slide`). A playlist action read `workspace["playlists"]`, a key the workspace does not have (it is `playlist`); that is fixed and its test fixture corrected. |
| `automation-action-media.ts` | media (video, audio), prop with auto clear, stage assignments, message with tokens, audio-input override, external presentation, capture, communication, transport | Done at the encoding or fake boundary: live fields, steps, and failures are tested. Communication encodes through `settings::devices::encode_payload` and sends through `sendCommunication`, never exercised against a device. Capture start and plain stop call `output::capture`; stop with an operator alert asks the operator first (`automation/confirm.rs`, `execute::confirmed_stop`) and stops only the recording that was asked about. Clock and timer tokens of a message are resolved by the host message ticker, as for React. |
| `automation-action-timers.ts` | timer control and configuration | Done: `Step::Timer` runs `mutateCountdownTimers` (configuration), then `controlTimer`, then stores the run in `countdownTimerRuns`. Tests cover the steps; the host calls were not run. The configuration step read the timer from `result["countdownTimers"]`, but the host answers with `result["workspace"]["countdownTimers"]`, so a Set Configuration action always failed; fixed (`execute::configured_timer`, tested). |
| `automation-action-clear-group.ts` | clear group | Done through `cues::actions::clear_group_patch` (phase 9). |
| `automation-action-values.ts` (`scheduleMediaMarkers`, `interpolateMessageTokens`) | Marker scheduling, token values | Done: `markers.rs`, `runner::message`. Tests cover defaults, guard fields, clamping, and token overrides; clock and timer tokens are host-side. |
| `automation-delay.ts` | Native delay scheduler | Done: `Step::Delay` -> `scheduleAutomationActions`. |
| `automation-runtime.ts` | Session, due events, build and cue events, startup actions, marker cancellation | Done: `automation::start`, `on_due`, `synchronize` (test `due_events_clear_what_is_on_air_but_stale_overdue_and_foreign_ones_do_not`, `due.rs` tests). Startup actions run once per workspace load through the same runner; no test drives them. Not ported: `synchronizeLiveBuildClock`. |
| `output-content-sync.ts` (`acceptNativeOutputStatePatch`) | Merge host-applied due patches | Done: a `statePatch` due event goes through `Output::accept_native_patch`, which drops older revisions. |
| `communication-input-runtime.ts`, `midi-trigger-input.ts`, `trigger-binding-runtime.ts`, `trigger-binding-controller.ts` | Input devices and MIDI fire trigger bindings, test button | Done: `automation/inputs.rs`, `artnet.rs`; MIDI, command/cue/group bindings, keyboard shortcuts, host test events and accepted communication packets reach the shared runner. Fake-boundary dispatch tests open no devices. |
| `timecode-chase*.ts`, `timecode-playlist.ts`, `TimecodeInputBridge.tsx`, `TimelineTimecodeFields.tsx` | Timecode input configuration and chase | Done: `automation/timecode.rs`, `settings/timecode.rs`; chase jumps/nudges/reverse/dropout, playlist range validation, input settings and devices. Synthetic-reading tests; hardware acceptance pending. |
| `timeline-*.ts`, `Timeline*.tsx` | Timeline editor, transport, lanes, soundtrack, waveform | Done: `automation/timeline.rs`; slides/actions lanes, waveform, zoom, marker dragging and time/target/action editing, record/pause/resume/stop/seek and semantic `setTimeline` save. Tests cover transport targeting and invalid authoring; hand acceptance pending. |
| `schedule-controller.ts`, `ScheduledEventsPanelBridge.tsx` | Calendar events: authoring | Done: `automation/calendar.rs`, a dialog from View > Calendar events (calendar switch through `saveSettings`, event list, editor with local date and time, weekdays, exception dates, end date, and the shared action-list editor), validation in `validate.rs` (`event_error`, local-time conversion with `chrono`), tests in `calendar/tests.rs` and `validate/tests.rs`. Due scheduled events run as before (not tested with a calendar fixture). Differences: weekdays are check boxes that only add or remove the canonical names (imported `DAY_OF_WEEK_` names are kept); exception dates are one comma separated field. |
| `message-controller.ts`, `MessagesPanelBridge.tsx`, `MessageEditorFields.tsx`, `MessagesInspectorPanel.tsx` | Message list, show, clear, editor, tokens, clear policy | Done: list, show, clear (with the instance guard), and the editor in `panels.rs` and `editors.rs` (name, multi-line text, theme slide, text, clock, and timer tokens with rename following, clear policy and duration, stale-save refusal, delete). Validation is `validate::message_error`; tests in `validate/tests.rs`, `editors.rs`, and `panels/tests.rs` (save, refusal, delete against a real host). The editor replaces the list in the panel instead of opening a dialog; there is no separate inspector pane; show-with-token-overrides and the network link publish are not ported. |
| `countdown-timer-controller.ts`, `native-timer-controller.ts`, `CountdownTimer*` | Timer list, control, editor | Done: list, start, stop, reset, and the editor (name, type, duration, target time, start and end time as `HH:MM:SS`, allow overrun), create, reorder, delete, all as the host's `mutateCountdownTimers` intents with one replay after a revision conflict (`save::mutation`); a deleted timer's run leaves the live state. Test `a_timer_is_created_edited_reordered_and_deleted_through_the_hosts_mutations`. No running countdown is drawn in the list (React redraws it every 100 ms); the timer format (`format`) is edited in the theme editor only. |
| `macro-controller.ts`, `macro-collection-controller.ts`, `MacrosPanelBridge.tsx`, `MacroPalettePanel.tsx`, `ActionListEditor.tsx`, `MacroTriggerFieldsBridge.tsx`, `Macro*Fields.tsx` | Macro palette, editor, collections, action lists | Done: the palette runs macros; the editor has name, hotkey capture, startup flag, collection membership, a built-in icon or a picked image (`selectMacroImage`), and the action list (`action_list.rs`, every authored action type with the fields React shows, move, enable, delay, remove). Collections: create, rename, duplicate, delete with a confirmation, reorder, an order per collection, cut, copy, paste, duplicate and reorder of macros, and the default collection (`macros.rs`). Validation is `validate::macro_error` with the hotkey conflict check against key mappings, group keys, trigger bindings, and other macros. A macro hotkey runs the macro (`automation::run_hotkey`, after the command shortcuts and never while a text field has focus). Tests: `macros/tests.rs`, `validate/tests.rs`, `action_list.rs`, `panels/tests.rs`. Not ported: the palette's Add Action tree on the row context menu (the editor's add action replaces it), assigning a macro to a collection from a menu (the editor's collection check boxes do it), the macro palette image rendering in rows, the palette row image painting (images remain editable in its editor). The playlist action's jump target is not resolved when validating (only that the playlist exists). |

### Authoring and utility panels

| React file | User-reachable behavior | State in GPUI |
| --- | --- | --- |
| `RightInspectorPropsPanel.tsx`, `prop-collection-controller.ts` | Prop collections, single mode, create, show and clear with auto-clear, rename, duplicate, copy, cut, paste, delete, add to and move to a collection, auto-clear time, edit | Done: `automation/props.rs`. A click is the semantic `prop` action (`toggle_action`), so single mode and the auto-clear delay run in the runner; every edit is one `mutateProps` intent (`save::mutation`). Edit opens the prop in the canvas editor through the shell. Test `props_and_collections_are_created_renamed_moved_and_deleted_through_the_hosts_mutations` and the pure tests. Differences: a row of buttons and selects replaces the card grid and the context menu; the running auto-clear countdown on a card and its pause are not drawn (the runner clears the prop after its delay); custom auto-clear seconds are not offered; the new prop and collection names are the English `New prop` and `New Collection`, as React stores them. |
| `StageOutputPanel.tsx`, `StageLayoutPicker.tsx`, `stage-content-controller.ts` | Layout per stage screen, create a layout, rename or delete a stage screen, configure a screen, the stage message | Done: `automation/stage.rs`. Screen edits are `mutateScreenConfig` intents through `output::windows::mutate_config`; layouts and the message are `mutateStageContent`, and the message also becomes `live["stageMessage"]` so the stage screens show it. Test `the_stage_message_and_layouts_are_edited_through_the_hosts_mutations`. Differences: a select replaces the layout picker with previews; Configure opens the stage editor on the screen's layout. |
| `TransportControls.tsx`, `handleTransportAction` | Play and pause, 10 seconds back and forward, jump to a marker, progress and time of the media layer | Done: `automation/transport.rs`. A button sends `controlPlayback` to the output process, which owns the playback clock; the answer's state patch is applied to the live state so the next scene keeps the pause. A marker jump is a `seek` to its fraction of the measured duration, computed here because the output process may not read the workspace. The media position is polled from the output process with `getNativeMediaPlayback` every 250 ms while media is on air. Tests: `transport/tests.rs`, and `playback_clock_commands_run_but_marker_jumps_and_workspace_reads_do_not` for the allow-list. Group 4 made the progress bar seekable and added the music position (`getNativeMusicPlayback`, read-only); see the seekable transport row below. |
| `AnnouncementsPanel.tsx`, `AnnouncementsDialogBridge.tsx`, `announcements-controller.ts` | Start a presentation as announcements with an interval and loop, previous, pause, next, clear, route screens | Done: `automation/announcements.rs`, a dialog from View > Announcements (it preselects the presentation chosen in the library). The player is `live["announcements"]`, which the scene already reads; select, navigate, toggle, and the interval tick are pure and tested (`announcements/tests.rs`). Routing is a `contentDestination` update with the announcements look layer. Workspace loads reset the player, including reload of the same path. Entry is View > Announcements. |
| `SlideActionsBridge.tsx`, `SlideCompletionBridge.tsx`, `slide-actions-controller.ts`, `slide-completion-controller.ts` | Slide action list and completion (off, after time, at media end, with target) | Done: `automation/slide_authoring.rs`, a dialog from Edit > Slide actions on the slide selected in the cue editor. Both are saved in one `setSlideDocument` through the library mutation (`Runtime::persist`), keeping every other slide field; an imported completion mode the editor has no fields for is kept unless changed; a dialog opened before the slide changed refuses to save. Tests: `slide_authoring/tests.rs` (validation, request building, clearing, and a save against a real host). Differences: one dialog and one Save for the two sections, and it is not inside the canvas inspector, which the phase 12 follow-up named. |
| `executeCaptureAction` | Stop a recording with an operator alert | Done: `automation/confirm.rs`. The question waits outside the workspace lane (`execute_queued`); a modal answers; an accepted stop is `stopCapture` with the recording file the question named, and only that recording is stopped. An unanswered question ends with an error after 5 minutes. Tests: `confirm/tests.rs` (answers, order, timeout, closed dialog, lane stays free) and `execute::only_an_accepted_question_about_a_running_recording_stops_it`. |

### Marker execution (owed by phase 13)

| Behavior | State |
| --- | --- |
| Markers of media on air become delayed actions bound to the selection | Done: `automation::synchronize` watches the live layers; a new selection cancels the layer's pending markers (`cancelPendingMarkerLayer`) and schedules the authored markers (`markers.rs` tests: guard fields, repeat defaults, clamping, missing path, layer key). Markers carried by a media action are scheduled after the batch set the selection (`Step::Markers`). |
| Position reaches the guarded runner | Done by design: the output process decodes the media and publishes marker positions in a process-global table; the UI scheduler reads its own table. `getMarkerPositions` (read-only, allow-listed) returns the output's table and the UI polls it every 40 ms while a layer is on air, then `acceptMarkerPositions` re-publishes the samples locally, aged by their transit. Host test `marker_positions_cross_a_process_boundary_aged_and_validated`. The poll runs only while a layer that is on air has markers armed (`Automation::armed`, set after the scheduler accepted at least one marker; test `the_position_poll_runs_only_while_a_marker_is_armed_on_air`), so media without markers costs nothing. The cross-process poll itself (`follow_positions`) was not run in a test. |
| A stale marker never fires | Done: `marker_guard_holds` drops actions whose workspace, asset, layer, or selection no longer matches, including the nested actions of a marker macro (tests `a_marker_action_runs_only_*`, `nested_actions_of_a_marker_macro_keep_the_marker_guard`). |
| TS tests of `media-markers.test.ts` | Ported by behavior: defaults (41), missing path (170), nested guard (182), retired asset and workspace (241). Not ported to the GPUI crate: the measured-position request queue with transport changes and the reselect-rearm sequence (76, 127, 312). The scheduler half of those cases is covered by the host tests in `native_automation.rs`; the GPUI half (key change, cancel, rearm) is tested only through `layer_key`, because the scheduler's pending delays cannot be inspected from `orbit-app`. |

### Restart and overdue actions

| Question | Answer |
| --- | --- |
| What survives a UI restart? | Nothing of the scheduler: delayed actions, marker delays, and timeline timers live in the Controller host of the UI process and are gone. A restarted UI starts a new session; the host retires the old one. Nothing scheduled before the restart can fire after it. |
| What does a fresh scheduler do with the slide that is still on air? | It sees a presentation with a start time, queues the slide's cue actions, and finds timeline cues already past due, so it would fire all of them at once. `due::Adoption` ignores `cue` and `timeline` events of the adopted live key for 3 seconds (`ADOPTION_WINDOW`), including a host-applied stale clear (a `statePatch` of source `cue`). Test `a_slide_adopted_after_a_restart_does_not_replay_its_overdue_cue_actions`. Detection: the output process answered a scene sync with a slide on air while this process had none (`output/scene.rs`); the condition is `output::scene::is_takeover`, tested by `a_slide_the_output_kept_is_a_takeover_only_when_this_process_has_none`. |
| What does React do? | It reloads with nothing on air, so nothing is overdue. GPUI differs because the output process keeps the slide. A cue action that was never run because the UI was down is skipped by design; a timeline that was playing is not resumed. |

### Features of the scope with no GPUI counterpart yet

None remain. Group 4 counterparts are recorded in the group 4 rows below.

### Shell reachability repair

| Feature | State | Evidence / deviation |
| --- | --- | --- |
| Toolbar Show and Edit | Done | Correct close/save and slide-canvas commands; headless shell test |
| Reflow | Done with native interaction differences | Text form, split and guarded merge mutations; real-host split/selection test, text rule tests; explicit buttons/commit instead of autosave and boundary gestures |
| More and its Stage/Theme/CCLI/Props/Mask entries | Done | Real menu, real editor commands, headless shell regression test |
| Audience / Stage / per-screen context actions | Done | Semantic group or screen mutation through existing routing; live token; exact intent test; hardware acceptance pending |
| File text imports and missing Screens menu entries | Done | Reviewed text source calls, guarded createPresentation draft, group toggles, capture/patterns in Screens; text mutation validation test |
| All menu registrations | Done | Recursive check of actual menus rejects missing registrations |

## Allow-list and script changes

- Invoke allow-list (output process): `getMarkerPositions` (read-only), `getNativeMusicPlayback` and `getAudioInputStatus` (read-only, group 4), `acceptLinkedServiceState` (group 2) and `controlPlayback`. `controlPlayback` is allowed for clock commands only: a `marker` action or a command that names a workspace is refused by `transport_command_allowed`, because both read the workspace (test `playback_clock_commands_run_but_marker_jumps_and_workspace_reads_do_not`). `acceptMarkerPositions` is not allowed on the output process (test). No settings or workspace writer was added.
- Event allow-list: unchanged. `automation-due` is deliberately not forwarded: automation runs only in the UI process, so an action cannot run in both.
- No script, baseline, or Tauri change. New ARB keys: `automation.macros.empty`, `automation.targets.destinationAudience`, `automation.targets.destinationAnnouncements`, `props.autoClear.title`, `props.autoClear.never`, `props.autoClear.after` (English, Dutch).
- New dependency: `chrono` 0.4 in `orbit-app` with the `clock` feature only, for the local time zone of calendar event times (`validate::timestamp`). It is already in the lockfile through `orbit-host`, with the same features.

## Hand acceptance (needs a person or hardware)

All five utility tabs and the calendar, announcements, and slide authoring dialogs against a real workspace (typing in the editors, the hotkey capture field, the icon and image pickers, the stage message typed live); the transport controls against playing media and music; the recording stop question with a real recording; announcements on a second screen; marker actions on real playing video and music through the link; a UI kill and restart with a slide on air (the adoption guard is tested with a fake clock); timer control, communication sends, and recording actions against real devices; the Tauri-versus-GPUI comparison of the action runner on a real service file. macOS and Linux were not compiled.

### Group 2: Bible browser

| React file | User-reachable behavior | State in GPUI |
| --- | --- | --- |
| `react/bibles/*`, `features/bibles/*` | Catalog, import, search, reference, parallel passage, create presentation | Done in `bibles.rs`, View > Bible Browser. Range and distinct layer mapping tests ported. Human acceptance: catalog pickers, import, search, output creation. Differences: explicit book-load button; creation refreshes canonical collections and selects the created item without changing live output; layer selection is manual. Preview selection uses search results; chapter verses are display only. |

### Group 2: Tickers

| React file | User-reachable behavior | State in GPUI |
| --- | --- | --- |
| `Ticker*`, `features/output/ticker-controller.ts` | Collection authoring, duplicate, text snapshot/file/RSS sources, play/pause/resume/seek/clear | Done in `tickers.rs`, View > Tickers. Host validates every saved definition. Replacement/duplicate tests ported. `services.rs` mirrors controller status to output every 250 ms, where the elapsed clock advances locally; no extra file watcher or RSS fetch. Human acceptance: ticker theme text and file/RSS refresh, pause and seek. Differences: optional values use empty number fields; native position refresh is explicit in the dialog. |

Output invoke allow-list addition: `acceptLinkedServiceState`, resolved ticker and Planning Center text only; no workspace/settings access or side effect runner. Host default/Tauri paths are unchanged unless this explicit GPUI command was received. `fetchRssFeeds` remains unused: ticker RSS resolution is owned by the host cache and `getTickerStatus`, so the authoring dialog need not fetch feeds itself.

### Group 2: Spelling

| React file | User-reachable behavior | State in GPUI |
| --- | --- | --- |
| `cue-spelling-utils.ts`, `cue-text-context-controller.ts` | Corpus suggestions, word range, guarded replacement, clipboard | Done in `orbit-ui/controls/spelling.rs` and `rich_text.rs`; the slide rich-text toolbar offers suggestions for the word at the caret/selection using the presentation library corpus. Uses editor replacement, preserving rich runs and undo. Tests port Unicode, punctuation, selection boundaries, ranking/deduplication, styled replacement, undo, and stale-word refusal. Corpus indexing and suggestion ranking run in background tasks. No dictionary dependency; shared host has no native spell engine (returns false). Differences: suggestions appear in toolbar, no WebView context event; uses Unicode lowercasing, not locale-specific collation; combining-mark classification covers the common combining blocks. Hand acceptance: suggestion click, undo, styled runs and IME. |

### Group 2: Planning Center

| React file | User-reachable behavior | State in GPUI |
| --- | --- | --- |
| `PlanningCenterDialogBridge`, `planning-center-controller.ts` | Browse services/plans, link/unlink playlist, sync with replacement confirmation, attachment imports, Live | Done in `planning.rs`, View > Planning Center. Host boundary fixtures test link mapping, attachment occurrence identity, unsupported media exclusion and replacement confirmation. All remote requests stay in the existing host; tests do not contact Planning Center. Workspace writers run on the revision lane and refresh canonical collections. Live is polled by the host and mirrored to program output; changing/removing the watched playlist link stops Live. Differences: explicit plan-load and Live status-refresh buttons in the dialog; sync confirmation is inline with a second Sync click; browse entry is in View rather than a playlist context menu. Human acceptance: credentialed plan browsing, sync, per-file errors and stage live text. |

Group 2 notes: every host call is off the UI thread. Request responses are retired after a workspace leave/return; form-dependent results are rejected if the draft changed. No new dependency, script/baseline change, workspace format change, or Tauri edit. New English/Dutch shared control labels were generated through `i18n:build`. Additional host-side command `acceptLinkedServiceState` only accepts the two resolved service snapshots and stores them per host. It advances a running ticker clock locally without starting a second controller runtime. UI crash freezes source updates; ticker scrolling continues from the last snapshot in program output. Reconnecting sends the new controller's status.

### Group 2 verification (2026-11-07)

- `cargo build -p orbit-app`: passed, zero compiler warnings (only the known path-canonicalization warning).
- `cargo test -p orbit-app -- --skip ten_thousand`: passed, 481 unit tests and 11 process integration tests; one unit and one integration test ignored; two scale tests filtered by the requested skip. The existing test-only deprecation warning in `automation/confirm/tests.rs` remains.
- `cargo test -p orbit-host`: passed, 1,567 tests, 51 ignored. The first sandboxed run could not execute FFmpeg's metadata probe; the permitted rerun passed unchanged. Existing host test-only warnings remain.
- `cargo test -p orbit-ui`: passed, 45 unit tests, 16 control tests, and five theme fixture tests.
- `cargo fmt --all --check`: passed.
- `npm run maintenance:check` and `npm run check`: passed (the latter includes renderer and test typechecks, lint, formatting, Vite build, and Cargo workspace check).

The app suite's stand-in process lifecycle tests required a permitted rerun to observe their child processes; no real output window was opened. Its existing concurrent group-edit test exposed older parsed canonical settings responses replacing a newer one. `settings/persist.rs` now rejects workspace responses older than the accepted revision, and that unchanged test passes both alone and in the full suite. The spelling replacement test also found inheritance from the preceding run at a selected-run boundary; replacements now inherit the selected run's style while insertions retain their prior behavior.

New headless fixtures cover Bible reference/search/parallel passages and presentation creation into a playlist without changing output content, Planning Center link/unlink revisions and failure preservation without credentials/network, Live link invalidation, ticker replacement and independent duplication, spelling ranges/ranking/styled replacement/undo/stale-word refusal, and mirrored ticker clock advancement. Successful remote plan fetches and attachment downloads continue to use the existing host implementation and its fixtures; the GPUI credentialed workflow remains human acceptance.

No group 2 feature has an open implementation blocker. Human acceptance remains for every new dialog in English/Dutch and dark/light themes, OS Bible/text import pickers, ticker/RSS/file behavior on actual output, rich-text suggestion interaction and IME, credentialed Planning Center browsing/imports and Live stage text, and the controller/output link during a real UI restart. No real workspace/settings, credentials, network service, output window, or device was used. macOS and Linux were not compiled; no platform-specific file was changed.

### Group 3: Imports, exports and external presentations

| React file | Behavior | State in GPUI |
| --- | --- | --- |
| `HandoutBridge`, `HandoutDialog` | Targets, arrangements, paper, outline/thumbnails, inclusion options; preview, print, PDF | Done in `transfers.rs`, View > Handouts. Validation ports target/arrangement/selection tests. Host retains preview, printing and destination review. Hand acceptance: native pickers, PDF preview and printer. Headless checks passed; native acceptance remains. |
| `PresentationExportBridge`, `PresentationExportDialog` | Text, images, PDF, arrangement, disabled slides and text notes | Done in `transfers.rs`, View > Export Presentation; validation tests ported. Hand acceptance: native destination picker and rendered files. Headless checks passed; native acceptance remains. |
| `PortableBundleDialogBridge` | Export/import presentation bundles and dependencies | Done in `imports.rs`, View > Portable bundles. Imports serialize on workspace revision lane and refresh canonical content; live state is untouched. Original host checks and conversion reports retained. Selection tests ported; native archive/picker workflow needs hand acceptance. |
| Keynote and PowerPoint import controllers | Native import with canonical selection and cancellation | Done in `imports.rs`, alongside bundles. Keynote requires a regular playlist and macOS; PowerPoint image import requires Windows/macOS. Fixtures port missing collection, library membership and cue validation; existing host image and Keynote insertion fixtures exercise files without launching either application. Busy suppression and workspace retirement use `services::call`. Live output is untouched. Hand acceptance: installed applications and OS pickers. |
| Chord chart controller | Choose PDF/image, page selection, load saved chart, save and clear | Done in `charts.rs`, View > Chord chart. Page validation and action-preserving save/clear tests ported. Uses `setSlideDocument` with original document guard on revision lane. Difference: standalone dialog, zero-based page field, explicit load/save; page preview and a single chart undo are local to this dialog. Existing canvas history is not extended. Hand acceptance: PDF conversion, page choice and stage rendering. |
| External presentation session and strip | Application/file/position, previous/next/stop, errors and dismissal; operator next/previous routes to external session | Done in `external.rs`. Status polling stays controller-only; controls serialize on a command lane. Stale poll responses cannot replace newer command results. Pure session fixtures cover status preservation, dismissal identity, stale results and ended sessions without launching Office. Starts and failures are published directly from the automation report, then status polls once per second only while running. Hand acceptance: real Office applications and hotkeys. |

### Group 3 verification (2026-11-07)

All six feature areas have implementations; no group 3 implementation row remains open. These are implementation and headless-verification results, not hardware acceptance.

- `cargo build -p orbit-app`: passed, zero compiler warnings.
- `cargo test -p orbit-app -- --skip ten_thousand`: passed with process permissions, 489 unit tests and 11 process integration tests; one unit and one integration test ignored, two scale tests filtered. The initial sandboxed run passed 488 unit tests but failed the two existing child-process lifecycle integration tests. The permitted full rerun passed unchanged. Existing test-only deprecation warning in `automation/confirm/tests.rs` remains.
- The subsequently added `external::backend_tests::native_service_fixture_drives_the_session_without_launching_office`: passed separately. It uses `ExternalPresentationService` with a fake `Backend`, never Office.
- `cargo test -p orbit-host powerpoint_import`: three fixture tests passed. `cargo test -p orbit-host keynote_import`: two fixture tests passed. Host sources were not changed; full host suite was not rerun. Existing host test-only unused-method warning remains.
- `cargo fmt --all --check`, `git diff --check`, `npm run maintenance:check`, and `npm run check`: passed. The npm check includes renderer/test typechecks, lint, formatting, renderer production build, and Cargo workspace check. Maintenance initially rejected missing descriptions for five new English translation keys; descriptions were added without changing a baseline.
- New app tests cover handout target/arrangement/selection validation; presentation export targets, arrangements and canvas-note refusal; bundle source/destination selection; imported collection membership and Keynote playlist cue validation; regular-playlist restriction; chart page bounds, action preservation and reference clearing; external error/dismissal/retirement semantics; external fake backend lifecycle; and opening/drawing all four dialogs in English and Dutch on GPUI's test platform.

Newly used bridge methods: `exportHandout`, `previewHandout`, `printHandout`, `exportPresentation`, `exportPresentationBundle`, `importPresentationBundle`, `importKeynotePresentation`, `importPowerPointImages`, `chooseChordChart`, `controlExternalPresentation`, `getExternalPresentationStatus`. Every group 3 method in part (a) is now Used. The table totals are 142 Used, 30 Unused with explanations, before group 4; the final bridge audit above supersedes these historical totals.

No host, Tauri, script, dependency, baseline, workspace-format, invoke allow-list or event allow-list changes. English/Dutch messages were generated with `npm run i18n:build`. A final integration commit adds workspace retirement, native File import/export menu entry points, a direct automation-result handoff to the external session, chart preview/local undo, and headless dialog coverage.

Differences from React: imports share the portable-bundle dialog (the File import entries open that form); handouts/export also have View entries. Chord charts use a standalone form with explicit load/save, one-based page choices over the stored zero-based index (group 4) and a local single-step undo rather than canvas history. Export/print destinations and bundle validation/reports stay entirely in the existing reviewed host commands; GPUI never writes their files directly.

Hand acceptance still required: OS file/save pickers and print submission, PDF preview, text/image/PDF export fidelity, bundle round trips and conversion reports, installed Keynote/PowerPoint import and slideshow control, chart PDF conversion and stage display, keyboard routing and external errors during a real session, and dialog layout across themes/densities. No app was launched against real data, and no real Office application, printer, output window or device was opened. macOS/Linux were not compiled; no platform-specific source was edited.

| Chord chart operator pages (group 4) | Done | `charts.rs`: page choices display 1..N and keep stored 0..N-1. Existing mutation test preserves the stored index. Hand acceptance: chooser and preview. |

| Recording confirmation lane (group 4) | Done | `execute_queued`: serial batches and effects rejoin the workspace lane; the recording question waits on the background orchestrator. File identity is still checked by `stop_if_current`. Tests cover question/lane independence and existing recording identity guards. |

| Live timer list values (group 4) | Done | `countdown.rs` ports running, paused, elapsed, target-time, overrun and display rules. A tick notifies only the utility panel while the timer tab is selected. Headless tests cover clock values. Hand acceptance: visible countdown. |

| Prop running auto-clear value (group 4) | Done | Runner publishes runtime deadlines and native remote patch metadata updates them. Props tab ticks independently and draws remaining time while live. Headless runner tests cover scheduling; hand acceptance covers the visible countdown. |

| Live build clock synchronization (group 4) | Done | `build_clock.rs` retains a valid occurrence clock, creates one for a new cue, clears it off air and prunes on rewind. Ported clock test exercises these transitions. |
| Startup actions (group 4) | Done | Existing runner now keys startup actions on workspace load generation as well as path, so reloading the same workspace runs its startup macros again. |

| Workspace load resets announcements (group 4) | Done | `output/scene.rs` resets transient output state on every workspace load, including the same path, matching React workspace-loader behavior. Canonical mutations do not count as loads. Hand acceptance: reload while announcements play. |

| Seekable transport and music position (group 4) | Done | Shared `RangeInput` sends clock-only seek commands; both layers poll native clocks. New read-only `getNativeMusicPlayback` joins the existing output allow-list; no workspace/settings writer added. Hand acceptance: playback on hardware. |

### Group 4 runtime acceptance

| Feature | State | Evidence and acceptance |
| --- | --- | --- |
| Timeline panel | Implemented | `timeline.rs`: authoring validation and playback target/clock tests, waveform and marker tracks; record operator actions/media and cue occurrences; save through `setTimeline`. Hand acceptance: drag, keyboard, long tracks, recording through real output. UI difference: lane markers have full field editors below them in a dialog opened from View. |
| Timecode | Implemented | `timecode.rs` synthetic-reading tests cover start, drift band, jumps, reverse and loss; settings use all four host methods. No test opens an audio input. Hand acceptance: LTC/MTC devices, playlist arrangements and dropouts. |
| Trigger runtimes | Implemented | `inputs.rs`, `artnet.rs`: matching MIDI dispatches the first binding exactly once at a fake boundary; packet validation and action-target tests. Hardware acceptance: RossTalk/deck/Art-Net, MIDI hotplug/reconnect. |
| Remote commands | Implemented | `remote.rs`: existing-target validation, nested delay flattening, disabled action omission and recursion guard. Deferred actions use the runner, workspace/load guards and final completed/error reports. Server remains in the controller host. Hand acceptance: browser control and cancellation during a long delayed macro. |
| Network Link cue publishing | Implemented | `link.rs`: authored index mapping test, controller `sendNetworkLinkCue` call; remote input suppresses rebroadcast. Peer hardware/network acceptance remains pending. |
| Audio input status | Implemented | `settings/audio_inputs.rs`: read-only output-link status/peak polling while the panel is visible; no test opens capture. |

Output allow-list additions in group 4: read-only `getNativeMusicPlayback` and `getAudioInputStatus`. No settings/workspace writer, dependency, script allow-list or workspace format change. No macOS/Linux-specific source was edited. Existing language keys were reused; no ARB regeneration was needed.

The source search for literal "not available yet" found no GPUI text. `shell.region.unavailable` remains a defensive unknown-tab/panel empty state; configured utility tabs and settings panels have implementations. Updater availability belongs to phase 16. Visual/hardware acceptance has not been performed, per the shared brief's no-real-windows/devices rule.

Final bridge inventory: **160 Used, 32 Unused with explanations, 0 unresolved entries**. The extracted table contains 192 entries, including its documented TypeScript false positives.

### Group 4 verification (2026-11-07)

Interrupted-work repair: the snapshot built and its tests passed, but two runtime errors in `automation/inputs.rs` showed raw translation keys (`automation.bindings.invalid`, `automation.timeline.invalid`) on the status line; they now carry the English sentence. Reloading the same workspace resetting the announcements player had no test; `output::scene::tests::reloading_the_same_workspace_resets_the_announcements_player` covers it. The host change is one read-only `getNativeMusicPlayback` dispatcher entry sharing `get_native_media_playback`; `src-tauri` is untouched and the default role is unchanged. The output allow-list additions are read-only (`getNativeMusicPlayback`, `getAudioInputStatus`).

- `cargo build -p orbit-app`: 0 warnings.
- `cargo test -p orbit-app -- --skip ten_thousand`: 504 unit tests passed, 1 ignored, 2 filtered; 11 process tests passed, 1 ignored.
- `cargo test -p orbit-host`: 1,567 passed, 51 ignored.
- `cargo fmt --all --check`, `npm run maintenance:check`, `npm run check`: passed.

User-visible "Not available yet." (`shell.region.unavailable`) remains only as the fallback for an unknown settings panel id (`settings/view.rs`) and an unknown utility tab (`view.rs`); every configured panel and tab now has an implementation. The toolbar buttons Reflow, More, Audience and Stage are disabled placeholders with no message; they are outside the phase 15 bridge scope.

## (c) Shell reachability

Audited 2026-11-07 from the mounted `AppShell.tsx`, `AppMenuBar.tsx`, `Toolbar.tsx`, `MoreMenu.tsx`, `AppMenuImport.tsx`, `ScreensMenu.tsx`, `AppWindowMenus.tsx`, `OutputGroupButton.tsx`, and `features/shell/app-interaction-controller.ts`. The GPUI evidence below is the real `AppView` registration, render composition, and child control callback, not a declaration of an editor type. “Reachable” records implementation; visual and hardware acceptance is still pending. A bridge inventory alone does not establish UI reachability.

| React entry | GPUI UI route and working handler | Change in this pass |
| --- | --- | --- |
| Brand/settings | Toolbar brand → `open-settings` → auxiliary Settings | Existing |
| Search | Toolbar / Edit → `open-search` → mounted `SearchView` | Existing |
| Show | Toolbar → `show-workspace` → suspend canvas and save the active text form | Fixed: previously took a cue, unlike React |
| Edit | Toolbar / Edit selected slide → `open-slide-canvas` → selected slide canvas | Fixed: toolbar previously opened the text form |
| Reflow | Toolbar → `open-reflow` → suspend canvas → `CueView::command(editCue)` → mounted text form | Enabled; split-at-cursor and merge-previous send `splitSlide` / `mergeSlides` on the workspace lane |
| More | Toolbar → `open-more` → rendered `ContextMenu` | Enabled |
| More → Stage | `open-stage-canvas` → `SlideCanvas::open_editor(Stage)` | Enabled in More; existing Edit menu route retained |
| More → Theme | `open-theme-canvas` → `SlideCanvas::open_selected` | Enabled in More |
| More → CCLI | `open-ccli` → selected document's `FormKind::Song` | New command; existing cue header metadata control retained |
| More → Props | `open-prop-canvas` → `SlideCanvas::open_editor(Prop)` | Enabled in More |
| More → Mask | `open-mask-canvas` → `SlideCanvas::open_editor(Mask)` | Enabled in More |
| Audience; Stage | Toolbar / Screens → `toggle-audience` / `toggle-stage` → `setGroupEnabled` → `mutate_config` → canonical screen configuration and existing output routing/scene | Enabled; partial groups turn fully on, fully enabled groups turn off; any enabled member gives the toolbar its exclusive live color |
| Audience/Stage context menu | Right-click toolbar output → group submenu → per-screen `set_enabled` | Added; checked per-screen state, Escape/outside dismissal via shared ContextMenu |
| File → New presentation / library / playlist | `newPresentation` / `new-library` / `new-playlist` → existing creation dialog or semantic tree mutation | Existing |
| File → Import → Text file / Clipboard | `import-text-file` / `import-clipboard` → mounted Imports form → reviewed picker / clipboard call → text draft → `createPresentation` | Added missing entries and text draft surface; author, notes, split mode, marker, units and destination preserved |
| File → Import → PowerPoint (.pptx) | `open-bundles` → Imports | Added label; React also routes this label to its portable-bundle surface |
| File → Import → PowerPoint images / Keynote | `open-bundles` → Imports → `importPowerPointImages` / `importKeynotePresentation` | Existing reachable handlers; platform restrictions remain in the form |
| File → Export presentation / Presentation bundles | `open-presentation-export` / `open-bundles` → mounted Transfers / Imports | Existing |
| Edit → Search / Edit selected slide / Theme editor / Key mappings | Search, slide canvas, theme canvas, auxiliary Settings keys | Existing; every menu command is registered |
| Presentation → New slide; Next/Previous slide; Next/Previous background | Registered `newSlide`, `nextSlide`, `previousSlide`, `nextBackground`, `previousBackground` → CueView or MediaBin | Existing |
| Presentation → Clear all / slide / media / props / audio / messages / announcements / to logo | Registered `clearAll`, `clearSlide`, `clearMedia`, `clearProps`, `clearAudio`, `clearMessages`, `clearAnnouncements`, `clearToLogo` → shared clear intents | Existing |
| View → Calendar / Bibles | `open-calendar` / `open-bibles` → mounted Calendar / Bibles | Existing |
| Screens → Audience / Stage | Registered group-toggle commands | Corrected: previously only offered individual-screen submenus |
| Screens → Configure / Recording / Test patterns | `open-screens` → auxiliary Screens; `SettingsView::render` mounts screen controls, capture panel and patterns panel together | Added missing Recording and Test patterns entries; shared native window instead of separate React dialogs |
| Window → Restore / Minimize / Maximize | Registered window commands → WindowControl | Existing |
| Window → New output → each source | Dynamic source entries → `create_windowed` through output call | Existing; hardware acceptance pending |
| Help → Settings / Key mappings / About | Registered auxiliary-window commands | Existing |
| Source context → Rename / Delete | Screens → screen title rename / delete button → `begin_screen_rename` / `delete_screen`, semantic config mutations | Reachable through Screens controls rather than duplicating the source context popover; last-screen and confirmation guards retained |
| AppShell → library, playlist, cue workspace, inspector, media bin, layout dividers | `AppView::shell` mounts TreesView, DocumentsView, cue thumbnails, output preview, utility tabs, MediaBin and persisted dividers | Existing |
| AppShell → placeholder linking, text import, new presentation, confirmation, auxiliary windows | Documents placeholder controls; Imports text draft; Documents new-presentation form; mounted Confirm; auxiliary registry | Text import route added; other routes existing |

### Global keyboard entry points

| React shortcut | GPUI route | Result |
| --- | --- | --- |
| Ctrl/Meta+N | `dispatch_key_down` → `newPresentation` | Reachable |
| Configurable New slide, next/previous slide/background, all eight clear commands | `KeyMappings` → registered command handlers listed above | Reachable, repeat policy retained |
| Macro hotkeys; imported trigger bindings | root key handler → `automation::run_hotkey` | Reachable, non-repeating; invalid bindings report through runtime status |
| Up/Down navigation; Shift range; Ctrl/Meta+A/C/X/V; Delete; Enter | focused cue workspace → `CueView::key_down` → selection, clipboard, delete or take | Reachable |
| Group shortcut letters | cue workspace key handler → effective group lookup and take | Reachable |
| Escape / menu keyboard navigation | shared ContextMenu/Search/Dialog and canvas handlers | Reachable; output context menus use the same menu primitive |
| Canvas undo/redo, delete, tool and shape shortcuts | focused SlideCanvas handler and theme shape mappings | Reachable when the editor is open |

Text-entry focus suppresses shell presentation/macro shortcuts. Canvas and form controls retain their own input handling.

### Editor, dialog and panel entry-point trace (phases 10–15)

| Surface | Entry from the running shell |
| --- | --- |
| Theme / slide / prop / mask / stage canvas and inspector | Edit menu registrations; toolbar Edit; More entries above; each sets `canvas_open`, and `AppView::shell` mounts the canvas |
| Screen canvas | Edit → Screen canvas → `open-screen-canvas`; render mounts `screen_canvas` when open |
| Reflow; operator notes; arrangements; CCLI | Toolbar Reflow; cue header notes / arrangements / CCLI; forms and arrangement editor mounted in CueView |
| Slide actions / completion / transitions | Edit → Slide actions → selected item/slide → mounted SlideAuthoring form |
| Graphics/music libraries; asset inspector; effects; presets; media markers; audio audition; routing | bottom MediaBin roots, selected-asset inspector and its controls; `MediaBin::render` mounts the active inspector and preview |
| Video input create/edit/discover | MediaBin input section → VideoInputs entity; input dialog rendered by its panel |
| General / Appearance / Audio / Remote / Groups / Keys / Sync / Updates / Import / Cleanup / Planning Center settings | Brand or Help → auxiliary Settings → panel navigation → SettingsView panel match |
| Video, screens, audience looks, NDI sizing, key/fill, correction, clear groups | Screens → Configure / Settings Video → screen controls and detail sections |
| Recording / stop confirmation / test patterns | Screens → Recording / Test patterns → mounted capture and patterns panels; capture stop uses mounted Confirm |
| Communication inputs/outputs / network links / membership / audio inputs / trigger bindings | Settings device/audio sections and their field callbacks |
| Macros / action lists / messages / timers | right utility tabs → Panels enum → list add/edit controls → mounted shared form/action list |
| Props / Stage message / countdowns | right utility tabs → mounted PropsPanel / StagePanel; edit callbacks open corresponding canvas |
| Calendar / timeline / announcements | View menu → `open-calendar` / `open-timeline` / `open-announcements` → option entities included in AppView render |
| Bible / Tickers / Planning Center | View → `open-bibles` / `open-tickers` / `open-planning` → mounted option entity |
| Export / handouts / bundles / Office imports / chord charts | File or View menu → Transfers / Imports / Charts option entities, included in AppView render |
| External presentation controls | selected Office cue → external session; status and controls rendered in the shell strip |
| Spelling | selected canvas rich-text input → corpus suggestion controls |
| About / licenses / notices | Help → About → auxiliary About controls |
| Startup update dialog | Still blocked: Phase 16 updater adapter never reports an available update (existing phase 14 blocker). Settings Updates is reachable. |

The shell regression test recursively builds every real menu (including dynamic and More/context menus) and rejects an unregistered-command disablement, then dispatches toolbar/editor commands. The output-group test verifies the exact semantic intents without opening outputs. Reflow adds tests for blank-line removal and merge joins, using React's rules. Existing canvas, cue, settings, media and automation suites exercise the child controls.

Native layout deviations: CCLI and Reflow use cue dialogs, recording/patterns share Screens, and rename/delete live in Screens controls. Reflow provides explicit split/merge buttons and Done/Show commit rather than React's multi-textarea autosave and boundary-key gestures; structural undo/redo remains in the canvas editor. These differences need operator acceptance and are not claimed as exact React interaction parity. No new dependencies, output invoke/event allow-list entries or script exceptions. No host or Tauri changes.

### Shell repair verification (2026-11-07)

- `cargo build -p orbit-app`: passed, zero compiler warnings.
- `cargo test -p orbit-app -- --skip ten_thousand`: 509 unit tests passed, 1 ignored, 2 filtered; 11 integration tests passed, 1 ignored. Includes the new real-host Reflow split/canonical-selection test, shell command/menu reachability test, output-group intent test, text-import mutation test, and Reflow text-rule test.
- `cargo clippy -p orbit-app -p orbit-ui --all-targets`: passed; **0 orbit-app warnings, 0 orbit-ui warnings** (25 orbit-app findings removed). The dependency's 344 older orbit-host warnings were left untouched.
- `cargo fmt --all --check`: passed.
- `npm run typecheck`: passed.
- `npm run maintenance:check`: passed, including design/architecture/import/reachability/unused-export checks and strict-v1 compatibility.
- `npm run check`: passed, including formatting, lint, both TypeScript configurations, production renderer build, and Rust workspace check.

The first sandboxed process-lifecycle test run failed its Windows process-liveness assertions; the same full suite passed with process inspection permitted. The first sandboxed build could not replace the generated executable; the final build passed with the required file permissions. No tests or checks were weakened. Manual launch was not performed: the shared brief explicitly prohibits real windows, outputs and devices. Visual interaction, physical/NDI output, hardware, macOS and Linux acceptance remain unverified. No macOS/Linux-specific sources were edited.
