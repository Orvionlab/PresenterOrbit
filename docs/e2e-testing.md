# End-to-end testing

Run `npm test` or `npm run test:e2e` for the small operator-workflow suite. `npm run verify` also runs the existing lint, formatting, architecture, localization, and compatibility checks. CI uses these same commands. Removing the internal suites also reduced the host library-test clippy baseline from 536 to 364; the lint ratchet remains active.

The media and recording workflows require working `ffmpeg` and `ffprobe` executables on `PATH`. The test script checks both before compiling. Verify them with `ffmpeg -version` and `ffprobe -version` in the same shell you use to run the tests.

The internal Rust unit/component/timing suites and Node unit suites were removed by product request. Do not replace them with one test per helper or control. Extend an existing operator workflow when possible. New scenarios must verify an observable result across the application boundary, such as saved workspace files, state after reopening, or content accepted by the real output process.

The GPUI scenarios live in `crates/orbit-app/src/e2e/`, mounted under `view` to exercise the real application without adding a public test API. They cover presentation authoring and reopening, taking/advancing/clearing slides through a real output child, audience message creation with title focus, autosave, drag reordering, reopening, showing, clearing and deletion, props view/size controls and saved transition defaults reaching real output, macro/prop collection context actions and drag order after reopening, managed media import and filtering after restart, and panel navigation across themes, densities, and device scales. `crates/orbit-app/tests/output_process.rs` additionally covers controller crash recovery and exclusive output ownership. The old process-tree demonstration and manual window driver were removed because they were not assertive automated scenarios.

The suite runs sequentially to avoid global desktop/service contention. Every scenario creates private data and workspace directories. It shuts down its host and child output process before removing those directories. Wait for observable state with a deadline; do not use arbitrary sleeps or frame-time assertions as success criteria. First-time compilation can still take time; reducing the suite does not eliminate Rust compilation.

The wait deadline also bounds draining GPUI tasks. The harness handles one host event per wait iteration, so continuous output events cannot prevent checking the expected state or the deadline. Presentation assertions read the real output child, including after rapid slide changes, rather than only checking the controller's local copy.

The Looks workflow covers the seeded Default preset, context-menu actions, focused rename, and automatic persistence. It holds workspace writes while changing layers and transition duration, checks live edits separately from the saved preset, exercises save, revert, save-as and double-click rename, and reopens the saved presets after restart. Delayed screen acknowledgements must preserve newer preset names.

The video-output workflow changes logical screens through the settings UI while holding storage, checks grouped and mirrored crops in the real output child, and configures edge overlap, color, corner pin and fill/key. Native BGRA samples verify attenuation on both blend edges. It rejects malformed layout mutations through the host, exports and imports the actual ProPresenter workspace file, and verifies the layout after reopening. Native device windows and NDI remain inert in this test.

Screen activation checks deliberately hold the workspace write lane while switching an individual output on and off and rapidly toggling the Audience group. A rejected scene-protocol request reads the output child's accepted configuration without committing another scene. The child must accept the final activation state while the stored state remains unchanged; releasing storage must preserve the last intent. This checks that output activation is independent of persistence, without a frame-time assertion.

The activation fixture saves an enabled screen but disables automatic activation at launch. Its first group activation must reach the output child before storage is released, even though the requested enabled value already matches the saved value.

Collection and prop creation flows hold the workspace write lane while naming and reordering items. The UI must show the new item, focus its name and apply the dragged order before that lane is released. After release and reopening, the stored result must match the latest edit rather than an older save acknowledgement.
The authoring workflow also exercises graphical stroke choices and the feather amount slider, then exports real ProPresenter files and imports them into a separate workspace. It checks composed slide layers so imported theme layouts participate, and verifies inside, center, and outside feather settings, including a disabled effect with a nonzero amount.

GPUI's test platform supplies mouse/keyboard input and layout without an OS desktop. The real host, persistence lanes, and workspace format are used. Tray/login integration and native windows are inert in this harness. Output checks use a real child process in inert mode and validate delivered scene content. Physical displays, GPU pixels, device audio, NDI hardware, installers, and operating-system chrome require separate acceptance checks. Prefer these internal E2E scenarios for routine work and leave routine visual verification to the user.

The Companion and Stream Deck packages retain their build/check commands; their removed unit suites are no longer advertised as test commands. Packaged-app smoke checks remain separate from this development suite.

The media readiness workflow imports a still, SVG, and video into managed storage, reopens an asset whose saved thumbnail is missing, and verifies generated previews. It takes media through a real output child and renders its accepted scenes at a fixed resolution. A configured video delay must preserve the outgoing picture and leave the incoming fade unconsumed until decoding starts.

The same media workflow waits for the active library to be prepared in both processes, checks the cache memory bound and malformed preparation requests, and verifies native image/video cache hits. The prepared video uses a nonzero in point; preparing it must neither activate it nor consume its delay or transition.

Media thumbnail layout is checked at small, medium and large zoom settings, including portrait artwork. The image element must fill its thumbnail viewport and resize with the media zoom control.

Startup waits for the visible first-page media thumbnails and native previews before dismissing its loading overlay. The harness delivers GPUI next-frame callbacks while awaiting that page. The media workflow checks its thumbnail elements immediately after reopening, without an additional thumbnail wait. Background library preparation is independent of this initial page barrier.

The operator navigation workflow also opens Help → Licenses, reads the bundled application license, filters a dependency with declared licensing but no standalone text, opens its detail page, and returns through the list to About. Both license declarations and original texts come from the build-generated catalog; absence of an original text must not be presented as absence of licensing.

The media workflow imports a transparent PNG alongside opaque media, checks that thumbnail decoding preserves alpha, and verifies that only transparent thumbnails receive a checkerboard in both grid and list views. The bin footer must no longer expose the missing-media button.

Prepared video activation must reuse metadata in the real output process, verified with metadata-load and cache-hit counters rather than a timing threshold. Repeated probes of an unchanged copied video reuse the result; changing its file timestamp forces a new probe. The media workflow also exercises delayed playback, fades and retakes while decoder process startup runs in the background.

The prepared-image fixture is 4K so the media flow exercises reduced cached images rather than only tiny source files. The presentation workflow holds workspace writes while changing the default from fade to cut and verifies the real output already cuts. It also checks presentation and slide overrides and removing those overrides to resume inheritance.

The media workflow uses a full logical canvas viewport and checks that a consumer with a different aspect ratio retains the canvas fit and transparent letterboxing. The video-output workflow continues to cover partial canvas crops, device calibration and edge blends.

The presentation workflow assigns a theme and edits it through the GPUI canvas while it is live, then verifies the changed theme reaches the output process. The media workflow switches between opaque and transparent stills and back, checking replacement and cached-pixel reuse before proceeding to video playback.

The preview Freeze workflow holds program delivery while changing slides and clearing to a configured logo. A real output child records the sequence through FFmpeg: decoded frames must retain the old picture during Freeze and show the current preview after resuming. It also checks the equal-width toolbar, NDI freeze state, invalid freeze payloads, and that Freeze is runtime state rather than workspace content. Existing program audio continues during Freeze; preview selections and transport changes do not alter its players. New recordings and streams require resuming output first.

The output-link workflow starts a real output child, overlaps a controller reconnect with its previous socket, and verifies that the existing output process and scene survive. It then mixes 64 scene updates with concurrent output/status requests and checks that the final content arrives without a link error. The foreign-controller rejection uses a genuinely different controller process. The presentation workflow also rapidly alternates slides through the UI and checks the final program content and connection errors. Connection teardown closes both socket directions; Welcome and pending events precede live events. Link state and original transport failures are recorded in diagnostics.

System display discovery runs through the controller host independently of output-process ownership or connectivity. The headless backend reports its actual empty monitor inventory. The display workflow shuts down its real output child, refreshes local display discovery and topology notifications, and opens the screen settings without clearing a separate connection error.

The presentation workflow also interleaves real native edits with scene submissions at the revision boundary. Repeated conflicts yield to the latest UI state, preserve native changes, and resume delivery without treating normal contention as a synchronization failure.

The playlist item menu workflow changes destination and presentation dimensions, opens the selected presentation in both editors, and reveals its library document. It hides an occurrence before storage completes, checks keyboard navigation and persistence across restart, restores individual or all hidden entries, and exercises copy, paste, duplicate and removal without deleting library content. Invalid visibility payloads are rejected by the host.

The continuous playlist workflow loads the saved preference, shows multiple presentations in one scrolling playlist, and toggles the same setting through General settings, the toolbar and the View command. It verifies persistence after restart and normal single-presentation browsing when a library is selected.
