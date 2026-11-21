# ProPresenter / Presenter Orbit feature parity

> This is a historical compatibility assessment. It includes legacy source paths and test commands. See the [end-to-end testing guide](e2e-testing.md) for the current operator workflows.

**Assessment date:** 2026-10-14, Europe/Berlin\
**Summary and development priorities refreshed:** 2026-10-30, Europe/Berlin. Remote phone findings, the bounded timer-authoring follow-up and the 2026-10-30 evening round (Remote fixes, macOS PowerPoint image import, INT-21 gradients/timer display rules, INT-02 exclusion) are recorded below; this is not a new whole-product audit.\
**Presenter Orbit:** `0.0.1-alpha.3`; original source baseline `61f638ae6a7ef9766381960db0463677cf320dcd`; the 2026-10-09 round starts at `c6b82316bbf9be65ca77a5aa23c4d8fd19392f11`, with its twelve feature commits listed in the integration review.\
**Purpose:** A reusable feature inventory, gap analysis, and acceptance-test backlog.
**Assessment method:** Current source inspection, repository tests, and official ProPresenter product/support documentation.
**Document language:** English, following the repository documentation policy.

**Inventory size:** 211 bounded capabilities across 11 feature families: 194 implemented, 6 partial and 11 not found. Counts describe this inventory, not a percentage of ProPresenter completion.

## 1. Findings

Presenter Orbit implements the core presentation workflow: libraries, nested playlists, arrangements, rich slide/theme editing, media, props, messages, stage layouts, timers, masks, native audio/video and display/NDI routing. The current matrix also records macro/action and timeline authoring, external timecode chase, independent announcements, Bible browsing and scripture generation, native editable PowerPoint import, recording with audio, RTMP streaming, folder synchronization and Orbit Network Link as implemented within their documented boundaries.

Seventeen inventory rows remain open. Six are partial: current Remote mobile acceptance (CTL-12), paid Bible acquisition/licensing (INT-02, excluded by the operator on 2026-10-30), Mac acceptance of PowerPoint image import (INT-18), real Keynote acceptance (INT-19), source-verified gradient/timer variants and text gradients (INT-21), and qualification of the latest Mac package payload (OPS-11). Eleven are not found: DeckLink input/output (NDI-09, OUT-23), Syphon input/output (NDI-11, OUT-26), SongSelect (INT-04), automatic CCLI reporting (INT-05), MultiTracks (INT-08), ProContent (INT-09), Resi (OPS-03), cloud backup/sync (OPS-09), and hosted multi-user collaboration (OPS-10).

Implemented means the bounded workflow exists, not that every source-application variant or target platform is qualified. Layered export now preserves supported presentation structure, actions, media and styling, with explicit loss reporting; the latest exported presentations have not been opened in ProPresenter. Planning Center has browse/sync and attachment download paths tested against mocks, not a real account. Native PowerPoint import has explicit unsupported content and only small generated fixtures. RTMP was tested locally; the bundled FFmpeg lacks TLS for RTMPS. Hardware, endurance and platform limits remain recorded in each matrix row. A field in a type, an import decoder, or an item in a dropdown is **not** sufficient evidence of working parity.

There is **no defensible overall percentage of ProPresenter completion** yet. Counting rows would give equal weight to a small text option and an entire streaming or Bible subsystem. This document instead records bounded capabilities, gaps, and verification requirements. None of the statuses means certified interchangeability or proven production reliability.

### Comparison baseline

The official [download page](https://renewedvision.com/propresenter/download) displayed **21.4/21.4.2**, dated July 1, 2026, at the original 2026-10-07 assessment. The page also exposed a `BETA` label in its extracted content; release-channel interpretation was not independently established. Therefore, the baseline is the **publicly documented current feature set**, with the version-specific editable PowerPoint importer explicitly included, rather than a claim that one exact stable binary was tested.

Orbit's importer is based on the vendored **ProPresenter 7.16.2** schemas. Newer ProPresenter documentation and older import schemas are separate baselines. Successful decoding of a document does not establish fidelity with every current ProPresenter file.

### Practical readiness by workflow

| Workflow | Assessment | Main reason |
| --- | --- | --- |
| Prepare a local lyrics presentation and ordered service | Substantial implementation | Library, text splitting, arrangements, playlists, notes, and themes have real editing/persistence paths. |
| Present lyrics with background media and stage text/timers | Implemented within measured limits | Native scenes, playback, stage assignment, timers and NDI exist; a full real-show and physical-output rehearsal remains unmeasured. |
| Use imported ProPresenter content as a starting point | Implemented supported subset | Authentic 21.4.2 layered/media fixtures have acceptance evidence; other versions/document families remain limited by available fixtures and explicit unsupported variants. |
| Recreate the whole ProPresenter workspace in Orbit | Broad supported subset | Workspace structure and authorable control features exist; external services, unsupported variants and hardware-specific integrations prevent complete equivalence. |
| Round-trip Orbit changes back into ProPresenter | Layered export implemented; acceptance incomplete | Supported layers, media, presentation structure and actions export with loss review; latest exported presentations still need reopening and comparison in ProPresenter. |
| Run independently scheduled lobby announcements | Implemented | Independent playback, scheduling and routing are recorded in the matrix; target-hardware rehearsal remains necessary. |
| Replace a ProPresenter broadcast workstation | Recording and RTMP implemented; integration gaps | MP4 includes audio; RTMP has local-listener evidence only, bundled RTMPS lacks TLS, and Resi/DeckLink remain absent. |
| Prepare worship content through SongSelect, Bibles, Planning Center, or MultiTracks | Mixed | Bible search/generation and local/WEB installation work. Planning Center paths have mock-only evidence. Paid Bible licensing, SongSelect and MultiTracks remain open. |
| Synchronize several presentation computers | Manual folder synchronization and Orbit Network Link implemented | Selected content and portable assets can be reviewed and pushed/pulled through a dedicated repository folder. Authenticated Orbit groups synchronize indexed cues and show controls. Cloud-account synchronization remains a separate gap. |

## 2. How to interpret the matrix

| Status | Meaning | Evidence required |
| --- | --- | --- |
| **I — Implemented** | A bounded capability has an authoring/control path and substantive implementation. | Connected UI/controller/native path, as applicable. Still subject to the verification limits below. |
| **P — Partial** | Useful implementation exists, but the ProPresenter workflow, fidelity, or breadth has a known gap. | The notes identify the implemented portion and the missing portion. |
| **D — Data/runtime only** | Import, storage, or execution exists without a complete operator workflow, or a status stub exists without the real subsystem. | Do not count as completed product parity. |
| **N — Not found** | No substantive corresponding implementation was found in the inspected source. | Absence is scoped to this commit and the inspected production paths. |
| **U — Unverified** | Names/fields exist, but a functioning end-to-end capability could not be established. | Requires focused tracing or manual comparison before upgrading status. |

Each row has a permanent ID. **Orbit evidence** refers to the linked source/test groups in section 5. The ProPresenter baseline is linked immediately above each feature group. Tests establish specific semantics, not complete equality with ProPresenter. Existing tests listed in evidence groups were inspected by topic; they are not all individually certified as meaningful acceptance tests.

Separate three questions when updating this document:

1. Can Orbit import or retain the data?
2. Can an operator create, edit, trigger, and persist the feature in Orbit?
3. Does its output and interaction match the required ProPresenter behavior on the target hardware?

Only the bounded capability in a row is rated. An `I` for countdown timers does not imply an `I` for Planning Center timers, for example.

Operator exclusions and unavailable provider accounts, hardware and Mac acceptance environments are recorded in the remaining-scope ledger below. These decisions do not upgrade inventory statuses.

#### Remaining-scope ledger

| ID | Decision or blocker | Recorded |
| --- | --- | --- |
| INT-02 | Excluded by the operator: paid Bible publisher purchase, activation and licensing are not wanted. The row stays `P` because the capability is absent, not delivered. | 2026-10-30 |
| OPS-11, INT-18, INT-19 | Blocked by environment: no Mac or funded hosted macOS runner is available for package, PowerPoint and Keynote acceptance. | 2026-10-30 |

### Inventory counts

| Classification | Capabilities |
| --- | -----------: |
| Implemented bounded capability (`I`) | 194 |
| Partial (`P`) | 6 |
| Data/runtime only (`D`) | 0 |
| Not found (`N`) | 11 |
| Unverified (`U`) | 0 |
| **Total** | **211** |

These are source-audit classifications. They do not certify every implementation for production or establish an overall parity score.

## 3. Detailed feature matrix

### A. Libraries, presentations, playlists, and search

ProPresenter baseline: [Interface](https://learn.renewedvision.com/propresenter/interface), [Arrangements and other editing workflows](https://learn.renewedvision.com/propresenter/working-with-slides), and [current release notes](https://renewedvision.com/propresenter/download). Some older `learn` URLs redirected to the knowledge-base landing page when opened; indexed guide excerpts were available, but exact historical UI details should be rechecked in the application.

| ID | Capability | Orbit | Existing behavior / remaining gap | Orbit evidence |
| --- | --- | --- | --- | --- |
| LIB-01 | Multiple presentation libraries | I | Create, rename, reorder, and delete libraries; native IDs and transactional persistence. | E01 |
| LIB-02 | Create and manage presentations | I | New presentations, rename, duplicate/copy/paste, move, and deletion have real mutation paths. | E01 |
| LIB-03 | Presentation search | I | Native indexed search includes metadata and slide content; selection uses renderer controllers. Exact ProPresenter ranking is not established. | E01 |
| LIB-04 | Playlist creation and editing | I | Create, rename, duplicate, reorder, and delete; canonical entries persist natively. | E02 |
| LIB-05 | Playlist folders / nested organization | I | Tree hierarchy, reparenting, collapsed state, and descendant deletion. | E02 |
| LIB-06 | Reorder items and repeated occurrences | I | Occurrence-aware anchors prevent editing the wrong repeated entry; drag interactions exist. | E02 |
| LIB-07 | Playlist headers | I | Header entries are distinct from presentation entries and survive playlist mutations. | E02 |
| LIB-08 | Placeholders and linked presentations | I | Placeholder creation, rename, and linking exist; not Planning Center synchronization. | E02 |
| LIB-09 | Continuous playlist view | I | Combined cue view, navigation, selection, notes, and linked placeholders. | E02, E03 |
| LIB-10 | Grid / table / simplified cue views | I | `grid`, `table`, and `easy` are implemented view choices. They are Orbit's views, not a pixel-identical ProPresenter UI. | E03, E16 |
| LIB-11 | Groups, labels, colors, and group keys | I | Group assignment and settings are editable/persistent; imported group identities also feed automation. | E03, E16 |
| LIB-12 | Song arrangements | I | Create, duplicate, rename, reorder, choose defaults, and edit section order. | E04 |
| LIB-13 | Repeated sections in an arrangement | I | Runtime uses occurrence identities; current/next resolution accounts for repeated sections. | E04, E10 |
| LIB-14 | Arrangement override per playlist entry | I | Entry override is persisted separately; deleting an arrangement clears matching references. | E02, E04 |
| LIB-15 | Operator notes | I | Presentation notes are stored and exposed in the cue/linked-text workflow. | E03, E07 |
| LIB-16 | CCLI / song metadata management | I | Existing presentation song metadata can be viewed, edited and saved with imported metadata preservation and supported export. Account connections and automatic reporting are outside this bounded editor. | E13, E17, |
| LIB-17 | Presentation bundles and sharing | I | Orbit bundles carry a presentation with its managed media, charts and themes, and validate chart fingerprints. Import also opens authentic ProPresenter `.proBundle`/`.proPlaylist` archives into the selected library: presentations, extracted and validated media (actions keep their ProPresenter identity) and playlists. Header actions are not imported; exporting to ProPresenter archives is bounded by INT-15/16. | |
| LIB-18 | Planning Center playlist linking | I | Built 2026-10-23 on operator request; tested only against a mock server, no real Planning Center account. Personal Access Token in Settings, service type/plan browsing, per-playlist plan link, explicit sync of songs (CCLI, then exact title), headers and a report of unmatched songs and skipped items. No OAuth, attachments or write-back. | E13, E17 |

### B. Cue editing, slide editing, and live operation

ProPresenter baseline: [Working with slides](https://learn.renewedvision.com/propresenter/working-with-slides) and [Interface](https://learn.renewedvision.com/propresenter/interface).

| ID | Capability | Orbit | Existing behavior / remaining gap | Orbit evidence |
| --- | --- | --- | --- | --- |
| CUE-01 | Text import and automatic slide splitting | I | Text import supports paragraph/line/marker-based split configuration and native presentation creation. | E01, E03 |
| CUE-02 | Quick cue text editing | I | Textarea editing, cursor preservation, context actions, and persistence. | E03 |
| CUE-03 | Insert, delete, split, merge, paste slides | I | Structural mutations allocate canonical slide IDs and guard stale anchors. | E03 |
| CUE-04 | Reorder slides | I | Drag/reorder controllers and native validation. | E03 |
| CUE-05 | Disable / re-enable slides | I | Authored enabled state persists; runtime navigation filters disabled cues. | E03, E10 |
| CUE-06 | Slide notes | I | Notes and note runs are stored; current/next notes can feed stage linked text. | E03, E07 |
| CUE-07 | Media drop onto a cue | I | Cue media-drop controller and native media-slide insertion exist. | E03, E08 |
| CUE-08 | Object-based slide editor | I | Uses the shared editor surface with composed slide documents, primary text, theme overrides, and persistence. | E05 |
| CUE-09 | Undo / redo in slide editing | I | Per-document histories and undo operations exist. No claim of global undo across every subsystem. | E05 |
| CUE-10 | Edit while preserving live content | I | `editorProgramSnapshot` freezes the live item, slide, and themes while authoring proceeds. Hardware/output rehearsal remains necessary. | E05, E10 |
| CUE-11 | Send edited slide to program | I | Slide editor has show/update controls and native output synchronization. | E05, E10 |
| CUE-12 | Next / previous slide | I | Navigation supports arrangements, enabled slides, and native remote execution. | E04, E10, E12 |
| CUE-13 | Next / previous presentation | I | Document navigation exists, including remote item navigation through the renderer. | E01, E12 |
| CUE-14 | Clear all / selected output layers | I | A dedicated Clear control sits beside the layer rail. Its context menu runs saved Clear Groups and opens their dedicated configuration dialog; scene updates coordinate native consumers. Exact ProPresenter clear-group semantics differ. | E10, E11 |
| CUE-15 | Global clear-group configuration | I | ProPresenter's `Configuration/ClearGroups` imports and exports with its identities. All eight ProPresenter layers, Stop Timeline, Clear Next, hide-in-preview, built-in or imported icons and tint can be authored, along with unique names, reordering and duplication. The icons are shown on the preview clear rail. Audio Effects and Video Input have no Orbit content source; exact icon artwork is not claimed. | E11, E13 |
| CUE-16 | Go-to-next / completion actions | I | ProPresenter Slide Timer semantics: After-time with Next/Loop-to-first targets, and a slide's own non-looping foreground video holds its last frame and starts the delay at its actual decoded end. Authentic Slide Timer encoding imports cleanly; the bare no-timer encoding reads as Off. Media-end and exact-occurrence targets remain Orbit extensions; targeted FIRST/LAST/AFTER_ACTION stay explicitly unsupported. | |
| CUE-17 | Object builds | I | On-click and after-previous steps are editable; native due events drive delayed builds. | E05, E11 |
| CUE-18 | Full transition library and presets | I | All 37 ProPresenter library effects are catalogued by UUID and animate natively as Orbit reimplementations, not frame-matched to ProPresenter's shaders. Slide, presentation and workspace-default slide transitions, effect variables and `Presets/Transition` favorites are imported, exported, authored and played. Media (background and foreground), props and messages play library transitions: a media selection's or prop's own transition, then the workspace layer default. ProPresenter's global layer transitions import from its Windows preferences and the props document; announcements use the slide hierarchy. | E08, E10, E11,; , |
| CUE-19 | Spelling tools | I | Windows native dictionary correction and autosave were rehearsed; Orbit proposals replace the whole Unicode word with cursor/selection, text and workspace guards. Dictionary availability is platform-owned; grammar and macOS/Linux operator equivalence are not claimed. | E03, E16 |
| CUE-20 | Print presentations / playlists / outlines | I | Canonical A4/Letter thumbnail/outline handouts for presentations, ordered selections and saved playlist arrangements; real PDF preview/export and direct Windows printer dialog with page ranges and driver-supported copies/properties. Installed ProPresenter comparison, native cancellation and actual Windows printer-to-PDF submission/page selection passed. Physical delivery and other-platform printer qualification are not certified. | |

### C. Themes, geometry, typography, and object appearance

ProPresenter baseline: [Slide/theme editor](https://learn.renewedvision.com/propresenter/working-with-slides). Transition breadth is documented separately under [Other features](https://learn.renewedvision.com/propresenter/other-features).

| ID | Capability | Orbit | Existing behavior / remaining gap | Orbit evidence |
| --- | --- | --- | --- | --- |
| EDT-01 | Theme creation / duplication / saving | I | Theme lifecycle, document factories, and native theme assets/persistence. | E05, E14 |
| EDT-02 | Multiple template slides per theme | I | Theme slides have identities and can be selected independently for output. | E05, E10 |
| EDT-03 | Theme assignment and inheritance | I | Library, playlist, presentation, slide, and output resolution paths; explicit no-theme and inheritance are distinct. | E05, E10 |
| EDT-04 | Per-slide theme overrides | I | Composed slide storage keeps layer overrides/order and primary text identity. | E05 |
| EDT-05 | Position, size, rotation, opacity | I | Numeric inspector fields, drag interactions, geometry, and native rendering exist. | E05, E06, E10 |
| EDT-06 | Zoom, rulers, guides, and canvas navigation | I | Viewport memory, zoom controls, rulers, and canvas interactions. Does not assert every ProPresenter snapping rule. | E05 |
| EDT-07 | Layer order, locking, and hiding | I | Object reorder/selection/visibility paths and shared scene ordering. | E05, E10 |
| EDT-08 | Text boxes and basic font styles | I | Font family/size, bold, italic, underline, strike, and alignment are authored. | E06 |
| EDT-09 | Enumerate all installed fonts / missing-font handling | I | Native cosmic-text/fontdb inventory replaces fixed font choices. Requested missing names are preserved with an explicit warning; deliberate installed replacement uses canonical persistence. Stale inventory and storage are tested; no whole-workspace audit or visual equivalence claim. | E06, E10, |
| EDT-10 | Automatic text scaling | I | Multiple text/container scaling choices and native text layout. Complex script/font equivalence needs comparison fixtures. | E06, E10 |
| EDT-11 | Capitalization and line transformations | I | Caps/title/start case and line-return/word/character transformations. | E06 |
| EDT-12 | Rich paragraph/ruler/tab-stop parity | I | Per-paragraph alignment, spacing, indents, line-height multiple, additive spacing and minimum/maximum line height; an editable ruler; left/right/center/decimal tab stops, automatic intervals and wrapping columns; all seven ProPresenter bullet types, numbered lists, affixes, zero decimal starts and levels 0–8. The inspector edits all paragraphs or a retained inclusive range. Strict persistence, WebView/native rendering and paragraph/list RTF import/export are connected. Plain edits, splits and merges preserve paragraph patches and character runs, including inherited theme patches after reload. An authentic ProPresenter 21.4.2 presentation passes workspace import/reload, native rendering and portable export; real keyboard/drag edits remain restricted to the selected range after reopening. Orbit's additional justified tab cells export as left-aligned stops with an explicit loss report. Unrepresented RTF extensions and other portable text-export limits remain reported separately. See the; . | E06, E13 |
| EDT-13 | Solid and gradient fills | I | Text/shape gradient controls and native rendering. | E06, E10 |
| EDT-14 | Shapes and custom paths | I | Shape tools, pen/path objects, geometry, and native SVG/path rendering. | E05, E06, E10 |
| EDT-15 | Stroke / shadow / feathering | I | Editor controls and substantive compositor effects with tests. This does not imply the complete ProPresenter effects library. | E06, E10 |
| EDT-16 | Image and video objects | I | Media picker, media fills, crop/flip/rotation/scale and local native decoding. | E06, E08, E10 |
| EDT-17 | Text media fill / cutout / background effects | I | Text media fills, gradients, Background Blur/Invert and object background effects import from ProPresenter and render natively. Text Cut Out cuts the glyphs through the box fill to the backdrop on flat and layered output, including scrolling text. Slide fill videos restart with each slide occurrence and follow Loop/Stop/Hold and Ramp In/Out. The editor shares one blur amount per slide and shows a native output still for backdrop effects. Effects are Orbit implementations; frame-exact ProPresenter comparison and the 100 % blur radius are not claimed. | E06, E10, |
| EDT-18 | Scrolling text / ticker appearance | I | Direction, speed, repeat, and feather controls; native scrolling retains text effects. Imported ticker-management breadth is separate. | E06, E07, E10 |
| EDT-19 | Web page object / live web fill | I | URL-authored and imported web objects supply live native raster pixels for display/NDI. Real WebView2, WKWebView and WebKitGTK probes verify page scripts, transparent overlays and view lifecycle. Native-authored URL/geometry/UUID import and alpha composition are covered. Browser input forwarding/audio routing, video-rate throughput and physical target qualification are outside this bounded raster acceptance. | E05, E06, E10, |
| EDT-20 | Object-to-object fill / text linking | I | Exact-ID fill/text authoring, persistence and authentic ProPresenter import. Linked fill inherits source paint while preserving destination geometry/style; editor and native pixels agree. Text transformations, dependency validation and source-change invalidation are covered. Desktop comparison and author/save/reload accepted. | |
| EDT-21 | Group-color and label-color data fills | I | All five ProPresenter data-color selectors import, author, persist and render, including exact repeated/scoped arrangement group colors, per-cue editor miniatures, alpha and explicit invalid-data handling. Native pixels, workspace reload and Windows operator save/reload are verified; physical color calibration remains a general output limit. | |
| EDT-22 | Capture-status color object | I | Complete editable four-state palette, native-driven editor/thumbnails, linked fills and native stage-window delivery. ProPresenter fill placement and genuine green recording behavior compared. Real stage recording start/stop, final MP4 pixels/audio, authored idle return and custom-color reload accepted; recording/error regressions include finalization after source failure. Original workspace and disabled outputs restored. Exact RGB matching is not claimed. | E11, E17 |

### D. Linked text, live data, and confidence content

ProPresenter baseline: [Linked text and fills](https://learn.renewedvision.com/propresenter/working-with-slides) and [Stage screen configuration](https://learn.renewedvision.com/propresenter/screen-configuration).

| ID | Capability | Orbit | Existing behavior / remaining gap | Orbit evidence |
| --- | --- | --- | --- | --- |
| DAT-01 | Current / next slide text | I | Renderer and native linked-text resolvers; next respects arrangement occurrences. | E04, E07, E10 |
| DAT-02 | Current / next slide notes | I | Note fields are resolved into stage/theme text. | E07, E10 |
| DAT-03 | System time / date | I | Format controls and native dynamic clock refresh. | E07, E09 |
| DAT-04 | Countdown timer linked text | I | Timer IDs/formats and native dynamic linked-text resolution. | E07, E09 |
| DAT-05 | Group, slide label, and slide count | I | Current/next group and label plus current/remaining/total counts. | E07, E10 |
| DAT-06 | Presentation / playlist / operator context | I | Presentation title, operator notes, playlist current/next/header/parent context. | E07, E10 |
| DAT-07 | CCLI-linked text | I | Edited canonical metadata refreshes renderer/native linked text, including copyright and song number; empty fields and supported binary export are tested. Manual output verification remains pending. | E07, E13, |
| DAT-08 | Text from a local file | I | Typed native content polling refreshes linked text and handles missing/invalid sources, replacement and recovery explicitly. Source/workspace/shutdown cleanup and stale read/picker guards are tested. Refresh can take about one second; short intermediate states can be missed. | E07, |
| DAT-09 | RSS feed text | I | Native fetch/cache/output resolution and editor preview; formatting controls exist. Recovery behavior differs by Orbit's own contract. | E07, E10 |
| DAT-10 | Ticker text source | I | Authorable text snapshots, watched .txt and RSS sources (titles/descriptions/custom delimiter), independent native transport, duplication, save/reload, inherited/Automatic/Off-Screen entry, exact inter-copy distance including zero and loop delays. Text-length-independent velocity and every visible copy now match the reference workflow; live updates preserve generation, position and pause. ProPresenter comparison and native desktop acceptance complete. | |
| DAT-11 | Stage message linked text | I | Native context and message state support a layout text object. | E07, E09 |
| DAT-12 | Current / next slide image | I | Current/Next image authoring, exact ProPresenter stage import, labelled editor placeholders, layout background isolation, native image updates and empty last-cue preview. ProPresenter visual comparison and Orbit save/reload verified; static first-frame media semantics and hardware limits remain explicit. | |
| DAT-13 | Embed a live preview of another output | I | Exact output selection is atomic and persists across restart. Authentic ProPresenter OutputScreen links resolve to canonical output IDs during import. Native stage-window acceptance follows live audience cue changes; fit/fill/stretch and independent video clocks are verified. Two-reference depth and 1024-pixel raster bounds remain deliberate limits. | |
| DAT-14 | Chord charts on stage | I | Managed image/PDF chart import, page selection, persisted assets and native stage rendering exist. PDF pages are rasterized on every platform by one pure-Rust decoder (hayro) within the existing size, page and pixel bounds. An imported presentation's local PDF/PNG/JPEG `chordChartUrl` becomes a managed chart on its slides; missing, remote or unsupported references are listed in the import warnings. Physical stage comparison with ProPresenter and macOS/Linux runs are not claimed. | , |
| DAT-15 | Planning Center LIVE stage data | I | Built 2026-10-23 on operator request; tested only against a mock server, no real Planning Center account. An explicit per-playlist toggle in the Planning Center dialog polls the linked plan's live resource every 5 s on a native thread. A "Planning Center Live" linked-text source shows the current item, next item, elapsed time or remaining time on stage and text layers, in the native compositor and the WebView alike. Errors clear the values and are reported in the dialog. Read-only: no control of Planning Center LIVE, no push, no OAuth. The fill type "Planning Center Live" remains an authoring placeholder. | E13, E17, |
| DAT-16 | External timecode display | I | Timecode linked text uses the existing native LTC/MTC input service, with shared input controls, live editor preview and authoritative native stage/slide output. Received labels retain frame/drop-frame notation; disengaged, failed and unlocked inputs show an explicit no-signal label. Native pixel tests, full renderer tests and a real WebView authoring/persistence smoke passed. Physical generator, display/NDI and Mac/Linux acceptance remain unqualified. | |

### E. Media bins, playback, media inspection, and audio

ProPresenter baseline: [Working with media](https://learn.renewedvision.com/propresenter/working-with-media), [Media Inspector](https://support.renewedvision.com/hc/en-us/articles/7200487649299-What-is-the-Media-Inspector), and [Preferences](https://learn.renewedvision.com/propresenter/preferences).

| ID | Capability | Orbit | Existing behavior / remaining gap | Orbit evidence |
| --- | --- | --- | --- | --- |
| MED-01 | Image/video media bin | I | Import, display, select, preview, and organize assets. | E08 |
| MED-02 | Audio files and audio collections | I | Separate audio assets/cards and playback selection; advanced audio inspector is separate. | E08, E15 |
| MED-03 | Nested media folders and playlists | I | Semantic native media-library mutations and hierarchy. | E08, E14 |
| MED-04 | Smart media folders | I | Native source-folder scanner derives media; metadata/settings persist separately from derived rows. Continuous watch latency unverified. | E08, E14 |
| MED-05 | Copy media into managed storage | I | Workspace central assets and theme assets; authored identity is retained independently of resolved URLs. | E08, E14 |
| MED-06 | Background / foreground selection | I | Separate behavior and end/playback settings; output clock/rendering paths exist. | E08, E10, E15 |
| MED-07 | Colors, gradients, and logo backgrounds | I | Background collections and native CSS/background resolution. | E08, E10 |
| MED-08 | Playback loop / stop / end behavior | I | Native playback and media setting paths. Exhaustive container/codec compatibility has not been tested. | E08, E15 |
| MED-09 | Pause, resume, seek, and skip | I | Rust-owned playback clock; native revision patches and decoder/audio reset on changed seeks. | E10, E15 |
| MED-10 | Media inspector and preview transport | I | Info/properties panels and video preview/transport exist, including detached-window support. | E08 |
| MED-11 | Crop, fit/fill/stretch, flip, rotation, alignment | I | Authored media properties and native draw behavior. Exact blur-scaling equivalence remains unverified. | E08, E10 |
| MED-12 | True codec and framerate reporting | I | Native ffprobe reports actual primary video/audio codec, dimensions, duration and average framerate; missing values and probe errors are explicit. Requires ffprobe; multi-stream and still-image codec inspection remain outside scope. | E08, |
| MED-13 | In/out points, rate, delay, retrigger rules | I | Video asset/action timing authors, persists, imports and drives native playback. Selection-scoped overrides, Restart/Continue/Automatic, remaining delay, trim-relative seeking and native preview status are integrated. ProPresenter inspector comparison and live Orbit save/reload/play/pause/seek/retrigger acceptance passed on Windows; physical output and other platforms retain the general verification limits. | |
| MED-14 | Video/image effects and presets | I | Image/video assets, media-filled objects and glyph fills carry desaturation, four color adjustments and ordered chains of 38 ProPresenter library effects with their variables, rendered natively (animated effects on the output clock). Media actions carry their own chain for their selection. Chains import from media bins, media actions, object fills and `Presets/Effect` using ProPresenter label naming; named presets hold adjustments plus chain and export to `Presets/Effect`. The media inspector shows a native still with effects; program previews are native frames. Effects are Orbit reimplementations; pixel-exact shader equivalence is not claimed, and media bin export belongs to INT-15. | |
| MED-15 | Playback marker actions | I | Named, timed and colored markers author ordered actions; consumed PTS drives video loop repetition and guarded manual replay. ProPresenter comparison and native Orbit save/reload/paused-seek/action replay acceptance passed. Measured native duration and accepted marker positions keep real video scenes and preview transport synchronized. | |
| MED-16 | User-set thumbnail from a frame | I | Native FFmpeg frame capture, workspace PNG persistence and reset for managed/smart media; invalid time, source and stale-revision guards verified. | E08 |
| MED-17 | Automatic audio playlist progression | I | Native EOF follows authored membership order for stop/next/repeat. Pause/resume and manual replacement preserve or retire the exact run. Policy saves preserve canonical asset references; browser preview completion cannot cancel native progression. ProPresenter comparison, native Orbit playback/save/reopen acceptance and stereo/six-channel Windows endpoint/main/NDI tap verification pass. | |
| MED-18 | Output audio device selection and gain | I | Native device enumeration, settings application, and playback. | E15, E16 |
| MED-19 | Main versus NDI audio routing | I | Separate output sections, multi-channel mapping, delay, mute/solo/test-tone, native mixer and NDI PCM transport. | E15 |
| MED-20 | Per-asset multichannel audio inspector | I | Audio and video assets route up to eight source channels to ProPresenter channels 1 and 2 as ProPresenter's AudioProperties do: per source outputs (summed), gain, Mute, Solo and Compress/Limit, plus master gain, before device and NDI routing. Audio assets loop, and audio markers can repeat on each pass of the decoded clock. Routes and audio Loop import from ProPresenter media and audio bins. Audio markers support inspector/action authoring, main-mixer or NDI consumed clocks, layer-isolated lifecycle guards and portable media closure. | , |
| MED-21 | Audio input capture and mix modes | I | Named native device/channel-pair authoring, gain/delay/latency, Off/On/Auto Off/Auto On, linked video inputs, cue/macro overrides, revisioned Settings-window synchronization and bounded CPAL capture feed the main/NDI PCM mix. Real Windows capture smoke and PCM/workspace regressions passed. Existing stereo-mixer, platform/hardware and release-size limits are explicit in the; embedded NDI input audio is the NDI-07 source kind of the same inputs. | E11, E13, E17 |
| MED-22 | Missing-media relink and cleanup tools | I | Relink and recoverable orphan cleanup use typed asset dependencies, explicit required-path diagnostics, preserved chart/action/live references and final file-byte verification. Import provenance and smart-folder roots are distinct. Smart folders remember the files they held, so files that disappear between sessions stay listed as missing until relinked or forgotten. ProPresenter smart media playlists import as smart libraries with their last synced files and folder behavior. | , |
| MED-23 | HDR / color-space pipeline | I | Per-screen SDR / HDR Rec. 2100 HLG / PQ format and a global Normal/Enhanced rendering bit depth (restart required) are strictly validated against target and platform. Display outputs compose into a 16-bit intermediate and present to a 10-bit swap chain with sRGB-to-Rec.2020 conversion, BT.2408 203-nit white and PQ or HLG encoding, with the DXGI or CAMetalLayer color space applied and the applied state reported. NDI/capture stay SDR, Windows has no HLG, and the compositor source is still 8-bit; no HDR monitor hands-on check. | E08, E10, E15, |

HDR baseline: [Using HDR in ProPresenter](https://support.renewedvision.com/hc/en-us/articles/45240760314003-Using-HDR-in-ProPresenter). ProPresenter's own output capabilities differ by platform/target; its documentation says HDR is not supported for NDI outputs. Orbit should be compared against the relevant target rather than an assumed universally available feature.

### F. Screens, stage layouts, looks, masks, and props

ProPresenter baseline: [Screen Configuration](https://learn.renewedvision.com/propresenter/screen-configuration), [Stage screens](https://renewedvision.com/propresenter/stage-display), [Using a stage screen](https://support.renewedvision.com/hc/en-us/articles/360041407794-Using-a-Stage-Screen-to-its-Full-Potential), and [Alpha keying](https://support.renewedvision.com/hc/en-us/articles/18435523006739-Alpha-Keying-in-ProPresenter).

| ID | Capability | Orbit | Existing behavior / remaining gap | Orbit evidence |
| --- | --- | --- | --- | --- |
| OUT-01 | Multiple audience / stage screen definitions | I | Screen identities, settings, assignment, and native output synchronization. | E09, E10 |
| OUT-02 | Physical monitor routing | I | Monitor discovery/names, target selection, native output windows. Multi-monitor rehearsal not performed here. | E10, E16 |
| OUT-03 | Native program rendering | I | Rust scene normalization, composition, text/image/video primitives and wgpu surfaces. | E10, E15 |
| OUT-04 | Operator preview of selected output | I | Native frame transport and preview UI; selected output is represented. | E10 |
| OUT-05 | Audience / stage enable controls | I | Screen/group enable and visibility mutations exist. Group on/off is not grouped pixel-canvas topology. | E09, E10 |
| OUT-06 | Per-screen layer visibility | I | Slide, media, props, messages, music and named extra layers can be toggled; effectiveness of extra layers is separately rated. | E10 |
| OUT-07 | Per-screen presentation theme | I | Output theme/slide choices override content theme and support different presentation styling. | E05, E10 |
| OUT-08 | Named Audience Look presets | I | Right inspector supports named Look CRUD, duplication and activation with independent per-screen layers, theme slide and mask. Per-screen native transactions expose partial failures; physical output acceptance remains open. | E10, E11, E13, |
| OUT-09 | Stage layout create / edit / save | I | Stage editor shares object surface; native layout mutations persist. | E05, E09 |
| OUT-10 | Different stage layouts per screen | I | Per-screen layout setting and stage assignment mutations. | E09, E10 |
| OUT-11 | Stage message show / hide / clear | I | Text/state controls, native persistence and output context; stage message objects import, author, export and render the box flash with its color. Comparison against ProPresenter hardware output remains open. | E07, E09 |
| OUT-12 | Stage current/next notes and clocks | I | Linked text/timer sources feed editable stage objects. | E07, E09, E10 |
| OUT-13 | Stage preview-image / chart / live multiview objects | I | Editable stage layouts combine slide images, exact live outputs and managed charts, with labeled placeholders and fit/fill/stretch controls. Acyclic nesting uses the full sixteen-output allowance; nested chart errors are explicit. Six-panel native-window, real NDI reception, independent video clocks and save/restart acceptance verified on Windows. Preview raster edge remains 1024 pixels; external hardware and other-platform certification are separate. | , |
| OUT-14 | Mask create / edit / output assignment | I | Dedicated mask editor/runtime/persistence and native mask scene composition. | E10, E18 |
| OUT-15 | Prop creation / editing / collections | I | CRUD, folders, duplicate/paste, shared editor, native IDs and persistence. | E05, E09, E14 |
| OUT-16 | Multiple simultaneous props / single collection mode | I | Live prop IDs, collection control and native ordered prop composition. | E09, E10 |
| OUT-17 | Prop timed auto-clear | I | Authoring and native delayed clear scheduling exist. | E09, E11 |
| OUT-18 | Independent announcements layer | I | Independent presentation/slide snapshot, transport, timed rotation and explicit persisted audience routing; native composition and video clocks remain independent of main transport. | E10, E17 |
| OUT-19 | Mirror / grouped tiled canvas | I | Dedicated source/follower topology, shared virtual canvas dimensions, full-source mirrors and bounded pixel-region tiles are authored and persisted in screen settings. Native composition crops every layer plane and aspect-fits the result to each output; source Looks/stage content and local hardware/delay/test-pattern controls remain distinct. No projector calibration, render-once optimization or frame-accurate hardware synchronization is certified. | |
| OUT-20 | Edge blending and projector correction | I | Dedicated per-output manual edge ramps, linear/quadratic/cubic curves, gamma, RGB gains, black lift and perspective corner pinning; validated authoring, persistence and native preview/output after canvas composition. Physical projector calibration and pixel-identical ProPresenter formulas are not certified. | |
| OUT-21 | Output test patterns and alignment tools | I | All ten current ProPresenter patterns, including animated Lines, Radar and Logo Bounce, Text, and Audio Video Sync with a clock-aligned beep through main/NDI audio and the configured delays. Pattern properties, single/all/audience/stage screen targets, Identify Screens and None/Orbit/imported logos are available, plus the earlier black/white levels. Projector calibration and pixel comparison with ProPresenter artwork remain hardware/visual limits. | E10, E16 |
| OUT-22 | Video delay | I | Saved per-output bounded native frame delay, explicit duration at display/NDI cadence, retained NDI sender/audio timestamps for delay-only edits and cleared pending frames. Actual NDI receiver timing and native save/restart acceptance passed on Windows. Physical A/V chain calibration, identical ProPresenter latency and other-platform hardware qualification are not certified. | |
| OUT-23 | SDI / DeckLink output | N | Targets exposed are displays and NDI; `sdiNdi` is an audio setting name, not proof of SDI hardware support. | E10, E15 |
| OUT-24 | Separate key/fill outputs | I | Audience outputs author and persist straight/premultiplied fill and grayscale key on paired display/display or NDI/NDI targets. One source raster, paired delay and generation drive both signals; NDI shares timecode/timestamp/cadence, and display batches submit matching planes before showing either member. Activation, closing, re-pairing and mode changes preserve paired lifecycle. Software delivery does not certify electrical genlock, monitor scanout or a physical switcher. DeckLink remains OUT-23. | |
| OUT-25 | NDI alpha handling / straight vs premultiplied | I | Selected NDI outputs author and persist None/Straight/Premultiplied. Native composition and real NDI receiver keying verify partial/transparent alpha, repeated frames and live mode changes with delay-buffer retirement and preserved sender/timestamps. ProPresenter controls and Orbit save/restart verified on Windows; third-party keyer and other-platform certification remain outside this bounded acceptance. | |
| OUT-26 | Syphon output | N | No macOS Syphon sender service found. Do not conflate this with NDI. | E10, E15 |

### G. NDI and live video inputs

ProPresenter baseline: [Video input configuration](https://support.renewedvision.com/hc/en-us/articles/360053482973-Setting-Up-A-Video-Input-in-ProPresenter), [NDI and SDI feature overview](https://renewedvision.com/propresenter/all-features), and [Alpha keying](https://support.renewedvision.com/hc/en-us/articles/18435523006739-Alpha-Keying-in-ProPresenter).

| ID | Capability | Orbit | Existing behavior / remaining gap | Orbit evidence |
| --- | --- | --- | --- | --- |
| NDI-01 | NDI output sender | I | Native runtime loading, sender lifecycle, configured dimensions/name/framerate and composed video frames. Runtime required. | E15 |
| NDI-02 | NDI output audio | I | Native PCM tapping/mixing, channel routing and timestamp accounting. Hardware/network acceptance still required. | E15 |
| NDI-03 | Multiple configured NDI outputs | I | Screen-target sender instances exist; usable output count/performance is unmeasured. | E10, E15 |
| NDI-04 | NDI source discovery | I | Typed discovery bridge and native receiver source service. | E15 |
| NDI-05 | NDI object and editor preview | I | Object fill, source inspector, native frame preview and renderer painting. | E06, E15 |
| NDI-06 | NDI input in native program composition | I | Receiver integration/loopback test exists. Installed-runtime loopback was ignored in this assessment. | E10, E15 |
| NDI-07 | Live NDI input audio routing | I | Native NDI capture supports the stereo pair or a free 1–64 source-channel → 1–16 bus matrix with many-to-many routing, gain, mute, Auto/Clear and input modes. Bus width survives capture, resampling, mixed inputs and independent main/NDI output matrices. An installed-runtime eight-channel sender → 16-bus native mix → actual 16-channel NDI output → second receiver loopback verifies every destination, gain, mute and cleanup. Physical-device, external-source and Mac certification remain separate verification limits. | |
| NDI-08 | USB camera / capture device input | I | A Video Input carries exactly one of `ndiSource` or `cameraSource` (device name as the OS lists it plus a fixed width, height and frame rate; workspace version 1, strict validation). Devices are listed through FFmpeg (DirectShow on Windows, AVFoundation on macOS) or `/dev/video*` (Linux, V4L2). One FFmpeg process per device decodes to raw RGBA frames into the same compositor path as NDI video; it starts when the input is used, is released five seconds after the last use and at app exit, and a missing, busy or unplugged device is an explicit error shown where NDI input errors show. Verified on Windows with a real USB webcam and with an FFmpeg `lavfi` stand-in; macOS and Linux paths are untested, and the bundled macOS FFmpeg build is expected to lack AVFoundation. Camera audio, camera controls and SDI are out of scope. See; . | E15, E17 |
| NDI-09 | SDI / DeckLink video input | N | Imported input descriptors exist; no native DeckLink input service found. | E13, E17 |
| NDI-10 | Independent switchable Video Input layer | I | Fixed input playlist with NDI discovery, naming, fit/fill/stretch, revision-checked persistence, independent trigger/clear, cue/macro actions, Looks routing and native frame composition. Hardware adapters remain separate; NDI audio is the independent NDI-07 audio input and is not part of the video layer. See; , including the existing renderer size-gate limitation and manual verification scope. | E10, E15, E17 |
| NDI-11 | Syphon input | N | No native Syphon receiver found. ProPresenter documents this as macOS-specific. | E15 |

### H. Timers, messages, macros, schedules, and timelines

ProPresenter baseline: [Macros](https://support.renewedvision.com/hc/en-us/articles/4402663090323-Using-Macros-in-ProPresenter), [Show Controls](https://learn.renewedvision.com/propresenter/interface), [Calendar and other features](https://learn.renewedvision.com/propresenter/other-features), [Timecode](https://support.renewedvision.com/hc/en-us/articles/7798653564563-Timecode-in-ProPresenter), and [Playback markers](https://support.renewedvision.com/hc/en-us/articles/7200487649299-What-is-the-Media-Inspector).

| ID | Capability | Orbit | Existing behavior / remaining gap | Orbit evidence |
| --- | --- | --- | --- | --- |
| AUT-01 | Countdown timer create / edit / order / delete | I | Dedicated panels/controller and native workspace mutations. | E09 |
| AUT-02 | Countdown-to-time and elapsed timers | I | Type-specific fields/formats, native time ownership and linked text. | E09 |
| AUT-03 | Timer start / stop / reset / overrun | I | Native controls and run state; additional imported increment commands exist. | E09, E11 |
| AUT-04 | Timer formatting | I | Hours/minutes/seconds/milliseconds and wall-clock options; visual edge cases need comparison. | E07, E09 |
| AUT-05 | Audience message templates and tokens | I | Messages author a theme slide as their template plus supported text/clock/timer tokens; native display instances continuously refresh tokens and use themed flat/layered composition. Imported ProPresenter templates resolve to the imported theme slide and export back by slide UUID. Visual comparison with ProPresenter output was not performed. | |
| AUT-06 | Message display / timed clear runtime | I | Show starts distinct attached timers and freezes a fresh display instance. Native monotonic deadlines and after-all-timer completion own dismissal, with pause/reset/retirement guards and independent stage state. ProPresenter workflow/semantics and native operator acceptance are verified. | |
| AUT-07 | Independent audience vs stage messages | I | Audience show/manual/automatic clear preserves stage text; stage show/hide/clear preserves audience state. Renderer/native delayed clears retain instance guards. Hardware output acceptance remains unverified. | E09, E11, |
| AUT-08 | Macro import and execution | I | Macro dialog creates, edits, deletes and runs ordered action lists through the existing nested/delay/startup runtime. Unsupported imported authoring contracts are retained read-only; delays remain independent offsets. | E11, E13, |
| AUT-09 | Macro CRUD / collections / palette | I | Macro CRUD, collection membership and independent order, grid/table/filter controls, workspace-scoped clipboard and slide drag assignment are implemented and regression-tested. Custom images, 62 built-in symbols, shortcuts and startup metadata remain supported. Native editor, save/copy/paste, distinct identities, grid/table/filter and cut/cancel passed manual acceptance. | |
| AUT-10 | Attach/edit slide actions | I | Slide action editor supports supported types, targets, ordering, delay, enable/disable and canonical persistence. Unsupported imported types remain explicit read-only diagnostics. | E05, E11, E13; task 5 |
| AUT-11 | Clear / prop / media / timer / stage actions | I | Authentic ProPresenter Add Action encodings are decoded and executed: per-layer clears, Clear Announcement limited to announcements, prop trigger/clear resolved to imported Orbit props, timer Start/Stop/Reset including Set Configuration persisted before control, and stage screen assignments mapped to Orbit screens while No Change screens keep their layout. Clear Video Input has no Orbit live-video layer to clear. | E11 |
| AUT-12 | Audience Look / mask actions | I | Existing Look actions apply stored per-screen layer/theme/mask values, including intentional clearing while preserving omitted imported assignments. Named preset authoring is available; hardware acceptance remains open. | E10, E11, |
| AUT-13 | Delayed action execution | I | Native monotonic deadlines and renderer session/revision guards prevent stale actions. This is an execution capability, not a full action editor. | E11 |
| AUT-14 | Startup macro execution | I | Editable startup flag and explicit activation timing use the existing once-per-workspace runtime; saving never creates a second execution path. Manual startup acceptance remains pending. | E11, |
| AUT-15 | Presentation timeline execution | I | Authored native independent timeline transport supports editing, scrubbing, recording and generation retirement. Timed actions can be authored with the shared action editor, and loop-boundary catch-up preserves chronological events. One workspace audio asset per timeline is the soundtrack: it is chosen in the panel, imported from the ProPresenter audio action and played natively on the main output in sync with the transport (seek, pause, loop wrap, generation retirement, drift correction). Per-timeline timecode start and an application LTC (audio input) or MTC (MIDI input) source drive the selected timeline with a lock status; the decoders are covered by generated-signal tests only and no timecode hardware was available. The panel shows slide, action and soundtrack lanes with a native waveform on one zoomable axis, with draggable cue markers. Playlist-wide timecode, extra audio or media tracks and the SDI/NDI audio mix remain absent. | |
| AUT-16 | Record timeline by clicking slides | I | Record arms at the chosen playhead; Play, pause/resume, repeated actual cue clicks, supported operator actions, editing, canonical save/reload and native replay are integrated. The live clock and compact controls stay available above the slide grid. Native recording suppresses automatic timeline replay; closing retires only the owned take. ProPresenter UI comparison and real Orbit recording/reload/replay passed. Soundtrack, multitrack lanes and timecode are covered by AUT-15. | |
| AUT-17 | External LTC/MTC timeline chase | I | Configured playlist chase preserves exact entry/arrangement targets, signed offsets, supported frame formats and generation-safe gaps/dropouts. Decoded forward/reverse LTC and MTC drive a signed native timeline clock and direction-ordered cue scheduler at variable speed. Soundtracks follow varispeed and reverse through bounded decoded audio windows; seeks discard stale buffers, rate changes re-anchor playback, and EOF/loop recovery reloads exhausted players. Native clock, scheduler, actual decoder/Player and renderer regressions pass. Physical-generator timing/jitter and macOS acceptance remain unverified; these are verification limits rather than the previous missing reverse/varispeed execution path. | |
| AUT-18 | Calendar recurrence execution | I | Persisted Enable Calendar control gates native scheduled actions. One-time/weekly events honor local start time, selected weekdays, exceptions and inclusive end dates; strict native validation preserves the accepted schedule on invalid updates. All weekdays, year/DST boundaries and disable/re-enable idempotence pass regression tests; native UI enable/reopen/disable was manually accepted. Undocumented ProPresenter edge cases and production reliability remain subject to the general verification limits. | |
| AUT-19 | Calendar CRUD and scheduling UI | I | First-class event CRUD with date/time, weekdays, exceptions, end date and supported ordered actions. Strict validation and canonical persistence. | E11, E17; task 11 |
| AUT-20 | Jump/select presentation or playlist action | I | Exact local/imported identities, playlist entries, arrangements and enabled slide occurrences resolve consistently in authoring/runtime. Playlist selection/retrigger flags, independent announcement destination, invalid-target diagnostics and exact native persistence are implemented and regression-tested; native authoring/save/announcement execution passed acceptance. | |
| AUT-21 | External presentation action | I | The imported action starts a local PowerPoint (Windows, macOS) or Keynote (macOS) file in its slideshow through COM or AppleScript. The next/previous slide commands, a control strip and the observed macro run drive and report it. Remote URLs, other platforms and missing applications fail explicitly. The slideshow is played by the application, not embedded in Orbit output; macOS is compile-checked only. | E11 |
| AUT-22 | Capture action | I | Shared cue/macro authoring supports exact recording presets, explicitly saved Active Settings, direct Stop and confirmed Stop. ProPresenter import preserves both commands and confirmation flags. Native file recording includes the selected screen's video/audio; queued actions and recording identity guard asynchronous start/stop. Real cue author/save and two finalized H.264/AAC MP4s accepted. Streaming, per-channel audio mapping and hardware equivalence remain separate operations limits. | |

### I. Remote control and external devices

ProPresenter baseline: [Browser control and API overview](https://learn.renewedvision.com/propresenter/other-features), [Network Link](https://support.renewedvision.com/hc/en-us/articles/4412193892627-Network-Link), [Communication devices](https://support.renewedvision.com/hc/en-us/articles/360058141114-Communication-Devices-in-ProPresenter), [DMX](https://support.renewedvision.com/hc/en-us/articles/360059980973-Devices-DMX), and [Stream Deck feature overview](https://renewedvision.com/propresenter).

The official API documentation URL `https://renewedvision.com/propresenter/api/` was inaccessible through the research tool. API existence is supported by the official feature/guide pages; exact endpoint-by-endpoint ProPresenter API parity is **not** claimed. Device availability is platform-specific: the current support articles describe MIDI on Windows and additional communication options, including DMX/Art-Net, on macOS. They explicitly do not claim sACN support.

| ID | Capability | Orbit | Existing behavior / remaining gap | Orbit evidence |
| --- | --- | --- | --- | --- |
| CTL-01 | Configurable keyboard commands | I | Key mapping settings, keyboard runtime and persisted bindings. | E11, E16 |
| CTL-02 | Group shortcuts and imported trigger bindings | I | Cue/group/prop/timer/macro/clear-group and thirteen presentation-command bindings support keyboard/MIDI authoring, strict native cross-scope validation, localized search and conflict filtering. Twelve observed ProPresenter menu identifiers import as executable canonical commands. Saved bindings synchronize directly to the main runtime; tests execute there. Real Windows conflict, save, immediate keyboard/group dispatch and full-restart acceptance passed. Application/editor menus outside the presentation-command catalogue remain unsupported imports; physical MIDI/platform certification remains CTL-03's stated limit. | E03, E11, E13 |
| CTL-03 | MIDI input triggering | I | Native MIDI input (midir: WinMM, CoreMIDI, ALSA) opens only while a binding has a MIDI note. It rescans for hotplugged and reconnected devices, parses note-on natively and dispatches by channel/pitch through the shared binding runtime. Settings shows the open devices and errors. Physical controller rehearsal per platform remains unverified. | E11 |
| CTL-04 | MIDI output / device transport | I | Native MIDI destinations can be enumerated, explicitly selected and saved. Shared actions/test fields send Note On/Off with channel, note and velocity; ordered submission, persistent connections, owned-note release and explicit missing-port/send failures are implemented. Native tests, renderer regression and real Windows software-port/UI smoke passed. Physical devices, network MIDI sessions and Mac/Linux output remain unqualified; CC/Program Change/SysEx are outside this note-output scope. | |
| CTL-05 | TCP / UDP communication actions | I | Validated device CRUD, shared UTF-8/hex payload authoring, exact native text/byte sends, strict persistence and visible request/workspace-guarded feedback are implemented. Real socket tests and native operator TCP/UDP sends passed; unsupported vendor protocols retain their separate statuses. | |
| CTL-06 | Serial / production-device protocols | I | Network-focused scope: raw TCP/UDP, RossTalk send/receive, acknowledged GlobalCache relays and Sony BVW/VDCP/AMP video-deck receivers. AMP implements the nine documented ProPresenter-style operations against one selected media bin, including folder/list/cue queries, native transport/seek, status ranges and TCP session creation/closure. Generic serial remains available; Sony BVS/GVG100 were retired by the operator. Physical-controller/macOS qualification and full AMP-server conformance are not claimed. Outgoing profiles share a typed module registry; external plugin loading is separate future work. | [Current scope and module boundary](communication-modules.md),; , |
| CTL-07 | DMX / Art-Net control | I | The in-scope Art-Net network receiver implements all 17 fixture channels and 16 control ranges, collection/arrangement-aware targets, native media/timer/timeline controls, global transitions, bind-error reconnect and optional ArtPoll discovery using an explicitly assigned OEM code. Direct IP control needs no OEM code. Windows real-app and network acceptance passed. Duration/edge policies and qualification limits are explicit; this is not USB-DMX support or certified Art-Net/ProPresenter interchangeability.; . Applicable ProPresenter baseline is macOS. | E11, E12 |
| CTL-08 | HTTP status and control API | I | Versioned authenticated Orbit endpoints, native timer/prop controls, validated targets and revision/status feedback; not ProPresenter API compatibility. | E12; task 10 |
| CTL-09 | Browser remote panel | I | Authenticated browser controls show current/next cue, exact repeated occurrences and library browsing/triggering from persisted presentations. Library navigation alone leaves output unchanged; cue selection clears prior playlist context. Remote playlist browsing now preserves exact repeated entry/cue identities and entry arrangement overrides; library browsing can select named arrangements without persisting the choice. Media/audio bins support exact managed asset triggering and active-occurrence Play/Pause through native transport. Saved ticker browsing/triggering, generation-guarded pause/resume/clear and seeking now use the native ticker clock. Exact video/audio occurrences now support seeking and restart, video in/out jumps, and previous/next within authored audio playlists guarded by canonical order. Saved audience messages show/clear, stage layouts are assigned per stage screen and clear groups run through the local controllers, with active message and per-screen layout status. Hardware and exact ProPresenter comparison remain unverified. | [playlist/arrangement API](remote-api.md) |
| CTL-10 | Remote authentication/settings | I | Enable, port/password settings and Basic authentication implementation. Internet deployment/TLS was not assessed. | E12, E16 |
| CTL-11 | Remote looks, macros, timers, props, capture and stage messages | I | Authenticated controls include real recording, server-clock timer feedback and stage-message edit/show/hide/clear with authoritative live status. Stage controls preserve browser drafts during polling. A remote macro ends in `completed` only after every enabled action, including delayed, nested and awaited native actions, has finished, or in `error` with per-action failures. Authenticated clients edit persisted timer definitions (name, duration, overrun) and message definitions (text, text token values) through the local editors, bound to the workspace and validated natively. No comparison with a physical ProPresenter Remote app. | |
| CTL-12 | ProPresenter Remote / Stage mobile compatibility | P | The opt-in compatibility listener now serves the current Remote at `/v1/ws` using the verified SHA-512/256 and AES-256 CTR64BE JSON envelope. Native routes cover library/playlist/presentation browsing, exact cues and active feedback, timers, macros/collections, clear groups/layers, stage controls and saved looks. Remote-only setup no longer requires a Stage password. Native codec, socket, catalog and command tests pass. The operator confirmed direct connection and library browsing in the current mobile app after repairing streaming-route handling. Audio/media controls, text/clock/timer messages and props, system/video timer readings, session-bound JPEG/PNG presentation thumbnails and current-client mDNS discovery now have native routes. A real native clock/message/timer/video smoke test passed. Source metadata probing, focused selection, native presentation timeline transport, asset thumbnails/icons and current Look/theme/mask mapping are now implemented. The operator confirmed automatic discovery and identified saved-connection filtering on the same PC; On 2026-10-29 the operator confirmed automatic discovery, presentation thumbnails and active-cue feedback on the phone. A reported missing associated-media defect is fixed in the shared native cue selector and renderer layer feedback; The operator confirmed the image fix on the phone. Associated media also resumes a previously paused transport. Moving video was verified on the desktop after clearing the slide layer; the selected slide/theme content obscured it. Reusing the revision-checked workspace cache reduced six local HTTP cue requests from 300–357 ms to 54–100 ms; encrypted WebSocket triggers with slide-index feedback measured 74–120 ms. These are local request/response measurements, not phone-to-output latency. The operator confirmed working video and faster cue changes on the phone, while active-cue feedback still lagged. Its local baseline was 503–605 ms despite command acknowledgements at 66–67 ms; subscriptions now also refresh on the existing accepted-status signal instead of waiting for the 500 ms periodic check. Three local feedback measurements afterwards were 199–220 ms; the real-socket notification/periodic-refresh regression and all 76 modern Remote tests pass. The operator confirmed faster active-cue feedback and requested faster thumbnail loading. Requested-size rendering, revision-checked preview reuse, a bounded content-keyed encoded-image cache, startup preparation of shared static-preview resources and decoded-frame completion now reduce three first local text/image/video thumbnail requests to 129/169/229 ms and repeats to 49–56 ms. The 95 Remote tests plus focused font, real-video and export regressions pass. The operator confirmed that thumbnails now appear almost immediately on the phone when opening a presentation and reopening it. The operator also confirmed reconnect. Saved-connection selection while ProPresenter runs on the same PC remains unresolved: the phone shows one saved entry despite distinct advertised names, hosts and ports. Remote timer feedback now uses whole seconds while native timing retains millisecond precision; 79 modern Remote tests and npm run check pass. Seven imported props are empty in the source file too; restoring the remaining prop’s original full opacity through the editor made its Remote-triggered blue rectangle visible in the output preview. Three saved audience Looks are returned by the live catalog. The phone prop-loading spinner was traced to an incorrect collection-list response envelope; it now matches the current reference server, and the operator confirmed collection loading. Remote-triggered prop auto-clear execution is now implemented in the native output ticker, with timing/retrigger/cancellation coverage passing among 253 selected prop-related Rust tests, plus 80 modern Remote tests and npm run check. A live encrypted trigger/retrigger test cleared active feedback at 7.327 seconds from the first activation, after a retrigger at two seconds. The three saved Looks were repaired through the editor after a deleted-screen reference blocked activation; future native screen deletions remove their Look mappings atomically. Phone acceptance of auto-clear, Look activation and the remaining controls is still pending. Unsupported full-HTTP-API authoring extensions are documented separately from the selected mobile UI scope. On 2026-10-30 the operator reported slow Looks, a non-counting prop auto-clear after a remote trigger, laggy timer edits, missing macros/messages/audio, wrong stage show/hide feedback and an empty stage layout screen. Fixes: remote workspace commands validate against the revision cache and Look activation skips unchanged screens; the props panel counts down remote auto-clear deadlines; an untyped timer is no longer treated as a type change; macros outside any collection, messages without a theme or network flag and unfiled audio (as a virtual "Audio" playlist) are listed; per-screen stage layout routes exist; remotely triggered media patches convert `file://` URLs so the preview plays and its clock runs. Stage show/hide feedback showed no code defect, and the new stage layout routes follow the reference API from memory. Phone acceptance of these fixes is pending. Stage Display is available alongside the current Remote app. | [Protocol and limits](remote-api.md#propresenter-remote-and-stage-display-websocket) |
| CTL-13 | Dedicated Stream Deck integration | I | Locally installable Elgato SDK plugin provides direct authenticated keypad controls, exact workspace targets, inspector configuration and native-state feedback. Official validation/packaging, SDK event-harness tests and real Orbit cue/timer/macro/authentication smoke tests pass. Installed-host, physical-device and Mac acceptance remain unverified; Marketplace publication and dials are outside this bounded capability. | |
| CTL-14 | Companion module compatibility | I | Standalone, locally installable Companion API v2 module provides authenticated polling, exact workspace/cue targets, actions, feedback, variables and button presets. The installed Companion 5.0.7 host passed real Orbit cue/navigation, native timer and rendered feedback, queued macro completion, authentication failure and automatic network-recovery checks. Version/platform and physical-device certification remain bounded verification limits; official store publication is not claimed. Orbit also serves a `GET /api/v1/events` server-sent-event status stream for push feedback (see `docs/remote-api.md`). | |
| CTL-15 | Network Link between computers | I | Authenticated Orbit groups support LAN discovery, manual reciprocal joining, indexed library/playlist cues, media/audio transport, timer definitions/actions, message text/tokens, props, macros and stage layouts. Receiver-local mapping, bounded independent peer queues, confirmed application, group forwarding and replay/echo protection are implemented. Content must occupy corresponding stored positions; assets/themes are not copied. Three-node loopback and native app acceptance passed; physical multi-computer rehearsal and ProPresenter wire interoperability are not claimed. | [operator setup](network-link.md) |

### J. Worship integrations, content acquisition, and import/export

ProPresenter baseline: [Bible installation](https://support.renewedvision.com/hc/en-us/articles/360041814913-Installing-Bibles-in-ProPresenter), [Bibles and other features](https://learn.renewedvision.com/propresenter/other-features), [Planning Center integration](https://www.planningcenter.com/integrations/propresenter), [MultiTracks automation](https://helpcenter.multitracks.com/en/articles/8261041-propresenter-lyric-automation-what-do-i-need), [SongSelect overview](https://renewedvision.com/propresenter/all-features), and [editable PowerPoint import release notes](https://renewedvision.com/propresenter/download).

| ID | Capability | Orbit | Existing behavior / remaining gap | Orbit evidence |
| --- | --- | --- | --- | --- |
| INT-01 | Bible browser and passage/keyword lookup | I | Local Bible browser supports installed book/chapter/verse selection, typed references, paginated keyword lookup and verse/chapter loading. Official public-domain WEB content is installable; parallel preview rejects numbering mismatches. | |
| INT-02 | Bible installation and licensed translations | P | Strict local packages, explicit public-domain WEB download, source/license display and removal work. Paid publisher purchase, activation and marketplace licensing remain absent. | [package format](bible-package-format.md) |
| INT-03 | Generate scripture slides / parallel translations | I | Scripture generation maps up to eight translations to distinct theme Text objects, configures verse numbers/breaks/labels/references, paginates whole verses using native text metrics, and persists independent cues in a library and optional playlist. Oversized individual verses require a larger text object or smaller font; paid content licensing remains INT-02. | |
| INT-04 | SongSelect account search/import | N | CCLI metadata does not establish an authenticated SongSelect client. | E13, E17 |
| INT-05 | Automatic CCLI reporting | N | No usage reporting/submission service found. | E13, E17 |
| INT-06 | Planning Center service browsing/sync | I | Covered by LIB-18: Personal Access Token authentication, service type and upcoming plan browsing, and explicit plan-to-playlist synchronization with CCLI/title matching and a mapping report. OAuth app registration is not provided. Tested only against a mock server; no real Planning Center account. See; . | E13 |
| INT-07 | Planning Center media attachment upload/download | I | Download only; upload is not provided. The Planning Center dialog of a linked playlist lists the plan's own and its items' attachments (filename, content type, size) and downloads the chosen image, video and audio files into the workspace media through the existing media asset path. The short-lived file URL from the `open` action is fetched without credentials, without redirects, over the API's scheme, with a 120 s timeout and a 256 MiB cap; other file types (including PDF) are listed as not importable, and each file reports its own error without leaving a partial file. Built 2026-10-23; tested only against a mock server, no real Planning Center account. See; . | E13 |
| INT-08 | MultiTracks lyrics/chords/key integration | N | Generic MIDI/group/metadata infrastructure exists, but no MultiTracks account/content/chord service. | E11, E13, E17 |
| INT-09 | ProContent/media marketplace | N | Local media import only; no catalog, subscription, or download integration. | E08, E16 |
| INT-10 | Import ProPresenter workspace | I | Filesystem workspace import preserves hierarchy/order/empty content, exact presentation and cross-configuration references, and managed media in presentations, themes, props, masks and stage layouts. Combined configuration-family tests verify source preservation, edit/reload, rollback and native output without source files. Unsupported content is reported; general newer-version compatibility and lossless export remain separate. | E13, |
| INT-11 | Preserve empty slides and group labels | I | Parser preserves empty cues, empty primary text, labels and repeated group arrangement order; checked-in independent synthetic fixtures verify decoded and composed workspace semantics. | E13, |
| INT-12 | Import layered/text/media objects | I | Authentic ProPresenter 21.4.2 text/image/video objects retain identity, geometry, alignment and paint order. Workspace import captures managed media; source-removal/reload/native-render tests and ProPresenter/Orbit desktop comparison verify independent text and moving foreground video. | , |
| INT-13 | Import newer ProPresenter file versions | I | Accepted as implemented by the operator on 2026-10-22; defects found with real newer files are fixed as they appear. Every field the decoder reads was compared across the vendored 7.16, 7.16.2, 19 beta and 21.4 schemas: no renamed, renumbered, retyped or restructured field, only additions. The authentic 21.4.2 documents match the 21.4 schema field for field. Dropped newer content is named in the import report, files from a newer release than 21.4.2 are flagged, unreadable cues are rejected, enum values outside the schema no longer run as defaults, and five decoder reads that never matched real data are fixed. No authentic file from ProPresenter 18, 19, 20 or most document families of 21.x exists in the repository, so those columns are schema-level only. | E13,; , |
| INT-14 | Import warnings and skipped-file diagnostics | I | Native import displays persisted per-file action/fill capability findings alongside skipped-file warnings. Export requires a reviewed per-field loss overview before destination selection; fields no vendored schema defines cannot be named (INT-13 reports the newer content it recognizes). | E13, |
| INT-15 | Export ProPresenter workspace structure | I | Native target checks verify collision-safe libraries, hierarchical/empty playlists, portable references, themes/messages, timers, props, masks, stage layouts, groups, macros and clear groups. Exported transition favorites and effect presets load, select and apply in ProPresenter, with title-preserving file names. Export requires a new destination and discloses portable-text and renamed-file losses. | E13,; , |
| INT-16 | Export layered slides / media dependencies losslessly | I | Layered export preserves supported text/runs, edited primary styles, shapes, straight/cubic paths, media properties, data links and stable UUID references. Presentations export their groups, arrangements, selected arrangement, cue completion/trigger settings, notes and ordered cue actions (including imported media actions); slide media backgrounds export as cue media actions with copied media, and theme, prop and mask slides export their own layers. All native catalogue effects execute for whole-object and paragraph/underline builds, with independent child planes, ordered steps, predecessor completion and authoritative occurrence clocks shared by output and exact-cue previews. Build authoring reuses transition effects, direction, variables and favorites. Authentic source fixtures establish scroller rate, object-axis repeat distance, feather fractions, text-gradient angles/bounds and per-channel dB gain. Linear text gradients preserve positioned stops and hard edges; browser/native rendering supports object or aggregate text bounds, including scrolling copies. Evenly spaced shape gradients now render all imported colors in CSS, feathered SVG and native output, and endpoint edits retain intermediate colors. Closed vector/Bézier paths use the same SVG gradient paint, with CSS-angle and object-bound geometry shared with native output. Workspace export resolves local media aliases, copies shared dependencies once, exports media bins/audio routing and preserves marker identity/time/name/color. Timer format flags and literal suffixes remain exportable; supported linked timers render in browser/native output. Loss review checks effective primary/secondary styles and reports unsupported geometry/paint/media fields rather than silently accepting them. Custom video thumbnail times import and export as `thumbnail_position` seconds (-1 when unset), established by an authentic 21.4.2 media bin capture. Unencodable action variants are named per action, and ProPresenter-inexpressible Orbit options (slide theme links, playlist item colors, repeat-on-loop) remain explicit review findings. Source-only nonlinear/length/blend-point gradients and conditional/wall-clock timer formats are import/runtime variants Orbit cannot author; they are tracked under INT-21. Exported `.pro` files were not opened in ProPresenter; no source-application visual-parity claim is made. | E13 |
| INT-17 | Import editable PowerPoint without PowerPoint | I | A native `.pptx` importer (File > Import > PowerPoint (.pptx), or Import in the presentation-bundle dialog) creates one library presentation without PowerPoint. Slides follow the presentation order. Text shapes become editable text layers with position, size, text with paragraph breaks, font size, family, bold, italic, underline, colour and alignment, resolved through the shape, layout, master and theme; placeholder geometry is inherited from layout and master. Pictures become image layers whose files are copied into `Media/PowerPoint Imports`. Solid slide backgrounds and speaker notes are imported; simple groups are flattened with their scale and offset. The slide size maps to 1920×1080 with one scale and centred letterboxing. Everything else is dropped and listed in a native report dialog and in the workspace import warnings: tables, charts, SmartArt, video/audio, animations, transitions, gradient or picture backgrounds, shape fills, outlines and geometry, connectors, bullets and numbering, picture crops, rotated groups and layout/master decorations; colour transforms (tint, shade, lumMod) are not applied. Mixed font sizes inside one box use the first run. Built 2026-10-23 and tested only on small generated fixtures; no real-world deck corpus was compared and no deck was opened in PowerPoint to compare rendering. `.ppt`, `.key`, export to PowerPoint and embedded media are not covered. See; . | E13, E16 |
| INT-18 | PowerPoint image import / external slide control | P | Windows can import static slide images into the selected library using installed Microsoft PowerPoint, preserving notes, skipping hidden slides, and retaining managed images independently of the source file. External slideshow control already exists. On macOS (2026-10-30, `crates/orbit-host/src/external_presentation_macos.rs`), PowerPoint for Mac has no AppleScript command that exports one slide image, so an AppleScript saves the deck as PDF and lists each slide's hidden flag and notes; the PDF pages are rasterized natively by the shared hayro renderer (`crates/orbit-host/src/chord_chart_pdf.rs`) into the same validated `Slide-NNNN.png` import path. PDFs with or without hidden slides map to slide numbers; any other page count is an explicit error. The listing parser and PDF-to-slide mapping are tested on every platform; the AppleScript has not run against PowerPoint on a Mac, and PowerPoint's sandbox may prompt for folder access. Native editable PowerPoint import is the separate INT-17 gap. | , |
| INT-19 | Keynote-specific workflow | P | A macOS-only file picker adds a local Keynote action cue to the selected library and playlist; it persists without converting the deck and reuses slideshow controls. Mac scripts now bind controls and cleanup to the opened document ID. Actual Keynote playback, display handoff and Automation permission acceptance remain unverified on a Mac. | , |
| INT-20 | Export text / images / PDF and print handouts | I | Canonical text, image and PDF exports plus outline/thumbnail handouts and native print dispatch exist. Video backgrounds export at their media thumbnail frame, slide video objects/fills and animated images at their first frame, decoded deterministically; slide actions do not block export. Live/web/NDI and audio objects have no static picture and fail explicitly; physical printing remains unverified. | |
| INT-21 | Import advanced gradients and conditional/wall-clock timer formats | P | ProPresenter 21.4.2 (Windows) authors and renders shape and text gradients as a linear blend of two colors at an angle: its UI strings describe "a gradient between two colors" with only Angle and color controls, and its renders of Orbit-exported probes (2026-11-01) ignored the protobuf gradient type, length, stop positions, blend points and every stop beyond the second, for shapes and for text. Orbit therefore matches that exactly (commit 7de014ec): shapes and text render their first two colors, `fillAngle` follows ProPresenter's counter-clockwise-from-the-right direction (commit 62162559, verified against the authentic Influence and Vivid Dark themes), and export writes two stops without positions like authentic files. The earlier Orbit-only radial/angle/length/blend/positioned-stop extensions were removed for parity; previously stored keys are inert. Linked timer text applies the `millisecondsUnderMinuteOnly` and `wallClock` display rules in browser and native output, but ProPresenter thumbnails render timer text statically, so these semantics remain unverified against ProPresenter. Orbit's text-gradient export now also writes ProPresenter's per-paragraph `custom_attributes` copies (an inclusive UTF-16 range per paragraph, counting a trailing newline, plus the same gradient fill) ahead of the element-level field 14, matching the authentic gradient fixtures' ranges; verified on 2026-11-02 by ProPresenter 21.4.2 thumbnails, where an Orbit-exported red-to-blue text gradient rendered plain red before the change and as a gradient after it (affects INT-16). No authentic text cut-out, media-fill or background-effect fixtures exist, so the same per-paragraph copies (oneof fields 8, 9 and 10) were verified by probe on 2026-11-02: ProPresenter 21.4.2 drew Orbit-exported cut-out and background-invert text as plain colored text before the change and as background-dependent fills after it; media-fill text lost its plain color after the change, but ProPresenter thumbnails did not draw probe images at all (an image shape with the same URL also stayed black), so the media image itself remains unverified. A probe with character-count instead of UTF-16 ranges left an emoji paragraph's successor without its gradient, confirming UTF-16 units. | |

### K. Recording, streaming, synchronization, and deployment

ProPresenter baseline: [RTMP capture](https://support.renewedvision.com/hc/en-us/articles/360049229013-Streaming-a-ProPresenter-Screen-to-An-RTMP-Server), [Resi and capture overview](https://learn.renewedvision.com/propresenter/other-features), [Sync between computers](https://support.renewedvision.com/hc/en-us/articles/360041588774-Syncing-Between-Computers-with-ProPresenter), and [current download/product information](https://renewedvision.com/propresenter/download).

| ID | Capability | Orbit | Existing behavior / remaining gap | Orbit evidence |
| --- | --- | --- | --- | --- |
| OPS-01 | Record program output to a file | I | Native canonical program BGRA frames encode into H.264 MP4 with the recorded screen's audio mix muxed as AAC on the same timeline; decoded pixels and audio placement are verified, and a 10-minute 1080p30 recording with audio was qualified. Physical-device and cross-platform rehearsal remain unverified. | |
| OPS-02 | RTMP streaming | I | A recording preset can stream instead of writing a file: it takes an `rtmp://` or `rtmps://` ingest URL and a write-only stream key (kept in its own app-data file, never in the workspace, renderer or status). The same native program frames and screen audio mix encode live with FFmpeg H.264 (2 s keyframes, preset bitrate as bitrate/maxrate/bufsize) and AAC to an FLV muxer, started and stopped through the existing capture paths, macros and remote. Status reuses phase, frames, duration and repeated frames (size is not reported for streams); a dropped connection ends the stream with an explicit masked error and there is no reconnect. One capture at a time. Tested only against a local FFmpeg RTMP listener; no real streaming platform was tested, and RTMPS needs an FFmpeg build with TLS, which the bundled FFmpeg build does not have. | |
| OPS-03 | Resi integration | N | No Resi client/plugin/encoder workflow found. | E15, E17 |
| OPS-04 | Capture presets / capture status monitoring | I | Native recording presets support new editable copies with independent IDs and existing guarded save/delete/control paths, with live phase, frame, duration, size, repeated-frame and error status. Presets author ProPresenter's custom recording audio map: 1–8 recording channels, each summing chosen screen mix channels, muxed as discrete AAC channels. Imported presets seed their screen, size, whole frame rate, bitrate and representable custom audio map into the editable draft. H.264 MP4 only (no ProRes/HAP/H.265/MOV/fractional rates); RTMP streaming is OPS-02 and Resi is OPS-03; hands-on ProPresenter comparison remains unverified. | |
| OPS-05 | Local workspace selection and persistence | I | Explicit workspace choice, strict schema v1, JSON indexes and managed assets. | E14 |
| OPS-06 | Revision-safe multi-window authoring | I | Settings/workspace semantic mutations reject stale writes; this is local concurrency protection, not collaborative cloud editing. | E14, E16 |
| OPS-07 | Recover interrupted workspace writes | I | Staged generations and crash-recovery tests exist. Does not replace user backups. | E14 |
| OPS-08 | Sync libraries/media/themes between computers | I | Settings repository selection, selective library/playlist/media/theme upload/download, previews, explicit conflict replacement, portable assets and charts, stale-proposal rejection, exclusive repository locks and transactional local writes. Deletions and machine settings are not propagated. Native two-workspace transfer and Windows UI acceptance passed; cloud accounts and live collaboration remain separate. | |
| OPS-09 | Cloud backup / synchronization | N | Local folders can be copied externally; no built-in cloud workspace client/service. ProPresenter Cloud marketing availability should be rechecked before purchasing/planning against it. | E14 |
| OPS-10 | Multi-user / multi-campus collaboration | N | No hosted collaboration, role/permission, edit-merge or shared multi-user workspace service. Current ProPresenter pages also distinguish future richer collaboration from sync. | E14, E16 |
| OPS-11 | Windows/macOS deployment | P | Publisher signing is optional on both platforms. Source-built FFmpeg/ffprobe, x264, zlib and corresponding sources/notices are bundled. Windows x64 installer and full portable ZIP passed all eight native/WebView write/restart phases, in-place reinstall, removal, workspace preservation and bundled codec execution in a separate Hyper-V VM. ARM64 media cross-build checks passed. Earlier Mac installed replacement/restart/removal passed; verification of the new Mac payload is explicitly deferred due hosted-runner billing limits. Local Windows/WSL build runners start after sign-in. | E16 |
| OPS-12 | Linux deployment | I | Ubuntu 24.04 x64 X11/XWayland baseline: actual Debian install/restart/removal and AppImage FUSE/extract-and-run acceptance passed; release CI gates both packages. Direct Wayland and other distribution/hardware qualification are excluded. This is an Orbit distinction, not claimed ProPresenter parity. | E16 |
| OPS-13 | App update discovery / install flow | I | Native update service and settings/startup panels with tests. Release-host execution was not manually tested here. | E16 |
| OPS-14 | Diagnostics export | I | Native diagnostic service and settings actions; sanitization is tested. | E16 |
| OPS-15 | Language / appearance / density preferences | I | English/Dutch localization, appearance and density/text scaling settings. Broad ProPresenter locale equivalence is not claimed. | E16 |
| OPS-16 | Startup / tray lifecycle settings | I | Launch-at-login, start-minimized and minimize-to-tray settings/native lifecycle code. Platform behavior needs acceptance tests. | E16 |
| OPS-17 | Production capacity / redundancy assurance | I | Accepted as implemented by the operator on 2026-10-22 within the measured limits below; defects found on show hardware are fixed as they appear. A repeatable ignored Rust harness measures the native NDI output path (composition, real sender and receiver, 1080p and 2160p, 1-4 outputs, 60/50/30/24 fps) and a 48 s show-cycle soak. On one Windows machine with a concurrent ProPresenter, 1080p text sustains 50 fps and image backgrounds 30 fps; 1080p video held 30 fps only in the faster of two machine states; 2160p is not qualified. 30-minute soaks at 1080p and 2160p showed no memory, handle, thread or decoder growth. A manual primary/backup runbook exists; Orbit has no state sync, watchdog or automatic switching, and full-application recovery time, other hardware, physical outputs, two-machine rehearsal and a real show are unmeasured. | E10, E15, E16; [failover runbook](operations-failover.md) |

## 4. Important implementation boundaries

### Import is not full support

`WorkspaceData` contains `macros`, `messages`, `plans`, `scheduledEvents`, `triggerBindings`, and `communicationDevices`. The native importer can populate these. Dedicated macro, message and calendar controls now author supported native contracts. This does not establish every imported action, target or provider integration. Each unsupported contract needs explicit validation and a complete operator workflow before its row can be promoted.

### Capture and streaming have separate acceptance boundaries

Capture actions now start and stop the real native FFmpeg file encoder through the typed bridge. Canonical BGRA frames produce finalized H.264 MP4 files. This is video-only recording; it does not establish RTMP/Resi streaming, audio capture or native status-color fills.

### Input descriptors are not capture devices

`PresentationVideoInput` and `PresentationAudioInput` preserve device names and attributes. Selecting an imported audio input is a store update. An actual NDI object/receiver is implemented, but that does not imply a general USB/SDI video switcher or live audio-input mixer.

### Output layer names do not create independent players

Announcements now have a separate active item/slide, native playback clock, rotation/navigation, routing and clear controls. Concurrent clock regressions pass; show rehearsal and broader completion/automation fidelity remain unverified. A named video-input layer still does not establish a USB/SDI capture service.

### Native output has its own rendering limits

The editor can display a web iframe, while native output composes raster/SVG/text/media frames. Browser content needs a deliberate capture/rendering boundary before it can appear reliably in native display or NDI output. Native object links, group/label fills, bounded screen previews and managed charts now have resolvers and pixel tests. Planning Center LIVE and capture-status color remain without complete native rendering; dropdown presence is insufficient.

### Export is intentionally limited

[encode_presentation](../crates/orbit-host/src/propresenter_proto.rs) documents that unsupported Orbit layers/media remain in Orbit rather than being guessed into ProPresenter binary documents. Export writes several document categories, but a populated export directory does not prove visual or behavioral fidelity. It is unsuitable as the only backup of a richer Orbit workspace.

### Timecode and transport are different

NDI video/audio timestamps support stream timing. `timecodeOffsetSeconds` supports timeline data. Neither proves decoding/chasing an external LTC/MTC signal. ProPresenter's external timecode workflow includes an input device/channel, format, engagement and playhead behavior; Orbit has no established equivalent at this commit.

### MIDI output remains unimplemented

MIDI trigger input is native and independent of the WebView (CTL-03). A declared `midi` or `serial` communication transport is still not implemented by the native socket sender.

## 5. Orbit evidence index

Links are relative so the document remains usable in the repository. Source directories identify bounded families of related implementation; test links identify relevant existing checks, not complete ProPresenter conformance suites.

| Evidence | Production code and contract | Relevant tests / additional documentation |
| --- | --- | --- |
| **E01 Collections/search** | [Collection controllers](../src/renderer/features/collections/), [collection UI](../src/renderer/react/collections/), [native search](../crates/orbit-host/src/library_search.rs), [native workspace](../crates/orbit-host/src/workspace.rs) | [Library/playlist management](../tests/renderer/library-playlist-management.test.ts), [collection persistence](../tests/renderer/collection-actions-and-persistence.test.ts), [native library search runtime](../tests/renderer/native-library-search-runtime.test.ts) |
| **E02 Playlist entries** | [Entry controller](../src/renderer/features/collections/playlist-entry-actions-controller.ts), [placeholder links](../src/renderer/features/collections/placeholder-link-controller.ts), [continuous playlist](../src/renderer/features/cue-editor/continuous-playlist-controller.ts), [native mutations](../crates/orbit-host/src/workspace.rs) | [Entry/placeholder tests](../tests/renderer/playlist-entry-and-placeholder.test.ts), [continuous playlist](../tests/renderer/cue-context-and-continuous-playlist.test.ts), [document reorder](../tests/renderer/collection-document-menu-reorder.test.ts) |
| **E03 Cue editing** | [Cue controllers](../src/renderer/features/cue-editor/), [cue UI](../src/renderer/react/cue-editor/), [text import](../src/renderer/features/collections/text-import-utils.ts), [native workspace](../crates/orbit-host/src/workspace.rs) | [Editor workflows](../tests/renderer/cue-editor-controller-workflows.test.ts), [text/reorder](../tests/renderer/cue-reorder-and-text-context.test.ts), [text import](../tests/renderer/text-import-and-placeholder-link.test.tsx), [media drop](../tests/renderer/cue-media-drop-controller.test.ts) |
| **E04 Arrangements** | [Arrangement runtime contract](../src/renderer/core/arrangement-runtime-contract.ts), [arrangement UI](../src/renderer/react/cue-editor/ArrangementControls.tsx), [native arrangement resolution](../crates/orbit-host/src/output_arrangement.rs) | [Arrangement controllers](../tests/renderer/arrangement-controller.test.ts), [arrangement utility](../tests/renderer/arrangement-utils.test.ts), [controls](../tests/renderer/arrangement-controls.test.tsx) |
| **E05 Slide/theme editor** | [Slide controller](../src/renderer/features/cue-editor/slide-editor-controller.ts), [slide composition](../src/shared/slide-composition.ts), [theme controllers](../src/renderer/features/theme-editor/), [editor UI](../src/renderer/react/theme-editor/) | [Slide composition](../tests/renderer/slide-editor-composition.test.ts), [slide surface](../tests/renderer/slide-editor-theme-surface.test.tsx), [theme surface](../tests/renderer/theme-editor-surface.test.ts), [builds](../tests/renderer/slide-builds.test.tsx) |
| **E06 Object/typography controls** | [Text options](../src/renderer/react/theme-editor/text-inspector-options.ts), [text inspector](../src/renderer/react/theme-editor/TextInspectorPanel.tsx), [shape inspector](../src/renderer/react/theme-editor/ShapeInspectorPanel.tsx), [canvas objects](../src/renderer/react/theme-editor/ThemeCanvasObjects.tsx), [native renderer](../crates/orbit-host/src/native_render.rs) | [Text rendering](../tests/renderer/theme-text-rendering.test.ts), [shape rendering](../tests/renderer/theme-shape-rendering.test.ts), [media fills](../tests/renderer/theme-object-media-fill.test.tsx), [inspector panels](../tests/renderer/theme-inspector-panels.test.tsx) |
| **E07 Linked content/RSS** | [Linked controls](../src/renderer/react/theme-editor/LinkedTextControls.tsx), [renderer resolver](../src/renderer/features/theme-editor/theme-linked-text.ts), [native resolver](../crates/orbit-host/src/native_linked_text.rs), [RSS service](../crates/orbit-host/src/rss.rs), [RSS output](../crates/orbit-host/src/rss_output.rs) | [Linked content](../tests/renderer/linked-content-themes.test.tsx), [linked controls](../tests/renderer/linked-text-controls.test.tsx); inline Rust resolver/RSS tests |
| **E08 Media** | [Media controllers](../src/renderer/features/media-bin/), [media UI](../src/renderer/react/media-bin/), [asset inspector](../src/renderer/react/media-bin/MediaAssetInspector.tsx), [inspector properties](../src/renderer/react/media-bin/MediaAssetInspectorPanels.tsx) | [Media bin](../tests/renderer/media-bin-controller-and-view.test.ts), [inspector](../tests/renderer/media-asset-inspector.test.tsx), [inspector controller](../tests/renderer/media-asset-inspector-controller.test.ts), [transport settings](../tests/renderer/media-settings-transport.test.ts) |
| **E09 Stage/props/timers** | [Utility panel](../src/renderer/react/output/RightInspectorUtilityPanel.tsx), [stage controller](../src/renderer/features/output/stage-content-controller.ts), [prop controller](../src/renderer/features/output/prop-collection-controller.ts), [timer controller](../src/renderer/features/output/countdown-timer-controller.ts), [native timers](../crates/orbit-host/src/native_timers.rs) | [Stage controller](../tests/renderer/stage-content-controller.test.ts), [countdown controller](../tests/renderer/countdown-timer-controller.test.ts), [native timer controller](../tests/renderer/native-timer-controller.test.ts), [stage mutation design](stage-content-mutations.md), [timer mutation design](countdown-timer-mutations.md) |
| **E10 Screens/native scenes** | [Screen UI](../src/renderer/react/settings/VideoOutputSettingsBodies.tsx), [preview](../src/renderer/react/output/NativeOutputPreview.tsx), [output snapshot](../src/renderer/features/app/output-scene-state.ts), [native scene builder](../crates/orbit-host/src/output_scene.rs), [compositor](../crates/orbit-host/src/native_compositor.rs), [native surfaces](../crates/orbit-host/src/native_output.rs) | [Output feature checks](../tests/renderer/output-feature-coverage.test.ts), [native preview](../tests/renderer/native-output-preview.test.tsx), [output control](../tests/renderer/output-control.test.ts), [smoke suite](../tests/renderer/tauri-output-smoke.test.ts); inline Rust output/scene/compositor tests |
| **E11 Automation** | [Actions](../src/renderer/features/app/automation-actions.ts), [presentation handlers](../src/renderer/features/app/automation-action-presentation.ts), [media/capture/device handlers](../src/renderer/features/app/automation-action-media.ts), [runtime/MIDI](../src/renderer/features/app/automation-runtime.ts), [native scheduler](../crates/orbit-host/src/native_automation.rs) | [Automation runtime](../tests/renderer/automation-runtime.test.ts), [action values](../tests/renderer/automation-action-values.test.ts), [edge cases](../tests/renderer/automation-action-edge-cases.test.ts); inline Rust scheduler/coordinator tests |
| **E12 Remote/devices** | [Native HTTP service](../crates/orbit-host/src/remote.rs), [renderer commands](../src/renderer/features/remote/remote-control.ts), [native command/socket handling](../crates/orbit-host/src/lib.rs), [remote settings](../src/renderer/react/settings/RemoteSettingsPanel.tsx) | [Remote/layout](../tests/renderer/remote-and-layout.test.ts); inline Rust request/status/native-command tests |
| **E13 ProPresenter conversion** | [Protobuf conversion](../crates/orbit-host/src/propresenter_proto.rs), [workspace import/export](../crates/orbit-host/src/workspace.rs), [import/export settings](../src/renderer/react/settings/ImportSettingsPanel.tsx), [vendored schema description](../README.md) | Inline Rust binary decode/encode/import/export tests, including malformed documents and media; [import/search controller](../tests/renderer/core-import-and-search-controller.test.ts) |
| **E14 Persistence** | [Workspace storage](../crates/orbit-host/src/workspace.rs), [settings](../crates/orbit-host/src/settings.rs), [collection persistence](../src/renderer/features/collections/workspace-content-persistence.ts), [architecture](architecture-dataflow.md) | Inline Rust transaction/revision/recovery/smart-media tests; [persistence interactions](../tests/renderer/collection-actions-and-persistence.test.ts) |
| **E15 Audio/video/NDI** | [Native audio](../crates/orbit-host/src/native_audio.rs), [native media](../crates/orbit-host/src/native_media.rs), [playback](../crates/orbit-host/src/output_playback.rs), [NDI sender](../crates/orbit-host/src/ndi.rs), [NDI receiver](../crates/orbit-host/src/ndi_input.rs), [NDI editor integration](../src/renderer/features/ndi/ndi-input.ts), [audio settings](../src/renderer/react/settings/AudioSettingsPanel.tsx) | [NDI editor checks](../tests/renderer/ndi-input.test.tsx), [loopback test](../crates/orbit-host/src/ndi_input_loopback_tests.rs); inline Rust audio/playback/NDI tests |
| **E16 Shell/platform/operations** | [Package scripts](../package.json), [platform/development notes](../README.md), [settings UI](../src/renderer/react/settings/), [updates](../src-tauri/src/app_updates.rs), [diagnostics](../crates/orbit-host/src/diagnostics.rs), [lifecycle](../src-tauri/src/desktop_lifecycle.rs), [spell integration](../src-tauri/src/spell.rs) | [Updates](../tests/renderer/app-updates.test.tsx), [version](../tests/renderer/app-version.test.tsx), [settings windows](../tests/renderer/auxiliary-windows.test.tsx), [app menus](../tests/renderer/app-menu-bar.test.tsx) |
| **E17 Contracts vs implementation** | [Presentation data](../src/shared/presentation.ts), [automation data](../src/shared/presentation-automation.ts), [input/mask/preset data](../src/shared/presentation-io.ts), [runtime/screens](../src/shared/presentation-runtime.ts), [workspace data](../src/shared/application.ts), [output schema](../src/shared/output-scene.ts), [fill choices](../src/shared/theme-values.ts) | Use these to locate fields. Their presence alone must never upgrade a row to `I`. |
| **E18 Masks** | [Mask UI](../src/renderer/react/mask-editor/), [mask runtime](../src/renderer/features/app/lazy-mask-runtime.tsx), [mask persistence](../src/renderer/features/app/mask-persistence-controller.ts), [native compositor](../crates/orbit-host/src/native_compositor.rs) | [Mask runtime](../tests/renderer/mask-runtime.test.tsx), [mask surfaces](../tests/renderer/mask-editor-surfaces.test.tsx) |

## 6. Verification performed for this assessment

The working tree was clean at the start. This assessment only authored this Markdown document. At the final workspace check, unrelated in-progress changes had appeared in `crates/orbit-host/src/content_themes.rs`, `crates/orbit-host/src/propresenter_proto.rs`, and `crates/orbit-host/src/workspace.rs`, concerning imported theme matching and preservation of individual text boxes. Those changes were left untouched. The matrix and test results describe the inspected baseline during this audit, not certification of the final concurrently changing working tree. Reassess import/theme rows after those changes are finalized.

| Check | Result | Interpretation |
| --- | --- | --- |
| `npm run check` | **Passed** | Script syntax, formatting, lint, renderer typecheck, test typechecks, renderer production build and Cargo check completed. |
| `npm run tauri:test` | **Passed: 423 passed, 0 failed, 6 ignored** | Native unit tests succeeded. Ignored tests require runtime/media tools or are manual benchmarks. |
| `npm run test:renderer` | **Failed: coverage thresholds** | Reported total coverage: statements 94.10%, branches 85.00%, functions 92.28%, lines 95.34%. Cue-editor branch coverage 82.42% vs 85%; theme-editor 84.47% vs 85%. |
| Full `npm run verify` | **Not run** | The renderer coverage failure already prevents claiming a green full verification run. Maintenance, Node test suites, and renderer budget were not separately executed for this document. |
| Manual app workflow comparison | **Not performed** | No side-by-side installed ProPresenter/Orbit operation, import screenshot corpus, or production show rehearsal. |
| External displays / NDI / MIDI / audio hardware | **Not performed** | Code/tests are evidence of implementation, not qualification of this host's hardware/network/runtime. |
| macOS / Linux build and install | **Not performed** | Assessment ran on Windows. Packaging scripts are not evidence of tested cross-platform packages. |

The native ignored tests are: manual 1080p NDI update benchmark, manual preview-flatten benchmark, three FFmpeg-dependent native media/render/compositor tests, and installed-NDI/network-discovery loopback. Normal Cargo test does not cover these paths end to end.

Renderer test output included jsdom canvas and Node local-storage warnings. The recorded command failure was the two coverage gates above. This document does not change thresholds or repair unrelated application/tests. Coverage is a quality gate, **not a feature-parity score**.

## 7. Suggested development priorities

Refreshed 2026-10-29 from the current matrix. This order assumes the present Windows-based local presentation and ProPresenter migration workflow. These are recommendations, not agreed product requirements or effort estimates. Provider access, target hardware and Mac availability determine which later packages can proceed. Implementation status and acceptance evidence remain separate.

| Order | Work package | Relevant IDs | Why next | Definition of done |
| --- | --- | --- | --- | --- |
| 1 | Finish current Remote phone acceptance | CTL-12 | Existing native routes make this a bounded opportunity to finish an everyday control workflow. | Use the actual current mobile app to verify discovery/reconnect, images, media/audio, messages, props, timers, stage controls and saved Looks; record versions and fix observed failures before closing the row. |
| 2 | Verify gradient/timer semantics against authentic fixtures and add text gradients | INT-21 | This is the clearest remaining import/runtime gap that can progress locally without a provider account or specialist hardware. | Split gradients and timer formats into separate changes. Establish authentic source fixtures; connect strict persistence, import, authoring and browser/native execution; verify export behavior/loss reporting and compare output with ProPresenter. Cover radial/angle, length/blend points, conditional fractions and wall-clock formatting. |
| 3 | Qualify the recent layered export and core service | INT-13, INT-16; AC-02, AC-06, AC-11, AC-13 | Existing implemented rows carry more immediate migration/show value when their remaining acceptance limits are tested. This does not add another open inventory row. | Reopen recent exports in ProPresenter and compare text, layers, media, actions, arrangements and loss findings. Rehearse a representative service on actual outputs with stage, audio, Looks, edits and restart; record measured limits and repair observed defects. |
| 4 | Complete the Mac-specific workflow and package pass | OPS-11, INT-19, INT-18 | Three partial rows depend on one unavailable platform; consolidate them when a Mac and build capacity are available. | Validate the latest bundled media payload through install/playback/restart/removal, exercise real Keynote and Automation permissions, then implement and verify PowerPoint image import using installed PowerPoint on Mac. Keep each row open until its own acceptance passes. |
| 5 | Choose one account integration based on actual use | INT-04, INT-05, INT-08, INT-09 | SongSelect search/import is the suggested first provider workflow if it is used for service preparation; provider access and supported integration rights must be established before implementation. | Confirm the supported account/API/licensing path. Deliver real-account search, review, import and persistence with explicit errors. Treat automatic CCLI reporting as a separate deliverable with verified submission semantics; choose paid Bibles, MultiTracks or ProContent instead only when the actual workflow calls for them. |
| 6 | Add production hardware/provider integrations on demand | OUT-23, NDI-09, OUT-26, NDI-11, OPS-03 | DeckLink, Syphon and Resi each require a concrete output/input need and their own acceptance setup. | Choose one target with available hardware/platform/provider access; implement and qualify real video/audio, clocks, lifecycle and failure behavior. |
| 7 | Define cloud backup before collaborative editing | OPS-09, OPS-10 | Folder synchronization and Orbit Network Link already cover bounded transfer/control workflows; hosted collaboration is a separate large product scope. | Establish a concrete unmet backup requirement, provider and conflict/restore contract; verify upload/restore first. Scope identities, roles and concurrent edit merging separately before starting collaboration. |

The 2026-10-30 round implemented the INT-21 gradient and timer variants locally; the next steps are phone acceptance of the CTL-12 fixes and an authentic ProPresenter fixture to verify the INT-21 semantics, followed by text gradients. Avoid reopening completed macro, message, Look, timeline, Bible-browser or capture workflows merely because the original assessment listed them as gaps. Known limitations within implemented rows, including RTMPS TLS and real-account Planning Center acceptance, should be prioritized when they block an actual operator workflow. The checks in section 6 are historical assessment results, not the current working tree's test status; run the required checks for each implementation change.

## 8. Acceptance scenarios to turn this inventory into proven parity

Record Orbit/ProPresenter version, platform, display topology, runtime versions, fixture identity, expected behavior, observed behavior, and screenshots/logs for every scenario. Use copies of workspaces. A successful one-slide demo is not sufficient for an entire feature family.

| Scenario | Procedure | Pass conditions / evidence |
| --- | --- | --- |
| AC-01 Library and service preparation | Create two libraries; text-import songs; label sections; create/repeat arrangement sections; build nested playlist with headers, repeated presentation entries and a linked placeholder; restart. | Identities, order, titles, notes, arrangements and entry overrides reload correctly; repeated entries are independently addressable. |
| AC-02 Live lyrics and media | Send a presentation live; change background video; play music; advance and clear layers; disable a cue; use a repeated arrangement. | Stage next text and program current text match the actual occurrence; clearing one layer preserves the others; audio/video obey transport. |
| AC-03 Edit during presentation | While a slide is live, alter its text/theme/objects; undo/redo; explicitly send the edited slide. | Live output stays consistent with the frozen snapshot until the intended show/update command; edits survive restart. |
| AC-04 Theme appearance corpus | Render fixtures with gradients, strokes, shadows, rich text, auto sizing, rotations, feathering, paths, media fills and scrolling. | Screenshots show expected geometry and typography at 1080p/4K; define visual tolerances and unsupported modes explicitly. |
| AC-05 Stage information | Use two stage layouts/screens with current/next text and notes, clock, elapsed/countdown timers and stage messages. | Layout assignments persist; live data updates; message hide differs from clear; overrun/stop/reset behave as specified. |
| AC-06 Layer routing and Look changes | Configure room/stream/stage with different visibility and themes; trigger a Look; clear selected layers and enable/disable groups. | Every destination uses the intended content/theme; no stale frames/layers; unavailable layers are identified instead of silently assumed. |
| AC-07 NDI sender/receiver | Install runtime; run actual loopback test; discover a source; render it as an object; send video plus routed audio to a receiver. | Correct resolution/frame rate/alpha/audio; quantify latency, dropped frames and channel mapping; loss/reconnect behavior documented. |
| AC-08 Media transport and markers | Use clips with known duration/channels; pause/resume/seek/skip/loop; import markers; repeat after changing playback rate if supported. | Output clocks, decoded frames and audio remain synchronized; marker actions fire with specified seek/pause/retrigger semantics. |
| AC-09 Imported automation | Import a fixture with nested macros, delays, timer/prop/message/media/look actions and startup rules; re-trigger and switch presentations while actions are pending. | No retired-session/old-cue action leaks; order is deterministic; unsupported capture/device operations report a capability gap. |
| AC-10 External control | Test shortcuts, MIDI if available, HTTP authentication, status polling, slide/item navigation and layer clear; test invalid commands. | Correct target and live status, authentication enforced, explicit command errors; record Web MIDI availability per platform. |
| AC-11 Import/export fidelity | Import copied 7.16.2/current workspaces; record warnings; export to a new folder; reopen in ProPresenter. | Supported subset matches expected semantics; losses listed; no claim that media/layers survive unless visually proven. |
| AC-12 Persistence and stale windows | Open settings/editor windows concurrently; perform conflicting mutations; interrupt a staged save in a disposable test workspace. | Stale operations cannot overwrite newer state; recovery returns a valid generation; no cross-workspace leakage. |
| AC-13 Endurance and capacity | Run a representative two-hour service with configured displays/NDI/audio, repeated transitions, editing and long videos. | Measure CPU/GPU/RAM, frame drops, audio continuity, stability and recovery; document the tested capacity, not a universal maximum. |
| AC-14 Packaging | Build/install/run on actual Windows, macOS and Linux target machines; test fonts, dialogs, playback, updates, tray and output windows. | Report per-platform results; package existence alone is not a pass. |

## 9. Research sources and limitations

Only first-party product/support documentation and first-party integration-provider pages are used as the ProPresenter baseline. Community discussions appeared in searches but were not treated as authoritative feature evidence. No prices, license-seat claims, or subscription recommendations are needed for this parity assessment.

| Source | What it supports |
| --- | --- |
| [ProPresenter all features](https://renewedvision.com/propresenter/all-features) | Broad feature families; integrations, NDI, looks, arrangements, macros and announcements. |
| [ProPresenter product page](https://renewedvision.com/propresenter) | Stream Deck, Syphon and production feature overview. |
| [Download and release notes](https://renewedvision.com/propresenter/download) | Displayed 21.4/21.4.2 baseline, editable PowerPoint import, recent view/prop changes and current Cloud marketing statements. |
| [Interface](https://learn.renewedvision.com/propresenter/interface) | Library/playlist/media/operator interface and Show Controls. |
| [Working with slides](https://learn.renewedvision.com/propresenter/working-with-slides) | Slide/theme editors, object/text controls, arrangements and linked content. |
| [Working with media](https://learn.renewedvision.com/propresenter/working-with-media) | Bins, inputs, playback, inspector settings, effects and markers. |
| [Media Inspector](https://support.renewedvision.com/hc/en-us/articles/7200487649299-What-is-the-Media-Inspector) | Per-asset playback/property/effect/audio/marker controls. |
| [Screen configuration](https://learn.renewedvision.com/propresenter/screen-configuration) | Audience/stage screens, grouped/mirrored/blended arrangements, Looks and stage data objects. |
| [Stage display](https://renewedvision.com/propresenter/stage-display) | Stage layouts, clocks, notes/current-next text and preview/multiview. |
| [Stage screen potential](https://support.renewedvision.com/hc/en-us/articles/360041407794-Using-a-Stage-Screen-to-its-Full-Potential) | Chart, message, preview and linked stage objects. |
| [Alpha keying](https://support.renewedvision.com/hc/en-us/articles/18435523006739-Alpha-Keying-in-ProPresenter) | SDI/NDI/display key/fill workflows and alpha configuration. |
| [HDR](https://support.renewedvision.com/hc/en-us/articles/45240760314003-Using-HDR-in-ProPresenter) | Target/platform-dependent color pipeline limitations. |
| [Video inputs](https://support.renewedvision.com/hc/en-us/articles/360053482973-Setting-Up-A-Video-Input-in-ProPresenter) | USB, SDI, NDI and macOS Syphon input families. |
| [Macros](https://support.renewedvision.com/hc/en-us/articles/4402663090323-Using-Macros-in-ProPresenter) | Macro authoring, actions and triggering. |
| [Timecode](https://support.renewedvision.com/hc/en-us/articles/7798653564563-Timecode-in-ProPresenter) | External timecode input, engagement, timeline recording and chase workflow. |
| [Other features](https://learn.renewedvision.com/propresenter/other-features) | Calendar, browser control, Stream Deck, capture, Bible lookup and transitions. |
| [Communication devices](https://support.renewedvision.com/hc/en-us/articles/360058141114-Communication-Devices-in-ProPresenter) | External protocol/device capabilities and platform limits. |
| [DMX](https://support.renewedvision.com/hc/en-us/articles/360059980973-Devices-DMX) | macOS DMX/Art-Net behavior; no documented sACN support. |
| [Network Link](https://support.renewedvision.com/hc/en-us/articles/4412193892627-Network-Link) | Peer groups and indexed content/show-control synchronization. |
| [Bible installation](https://support.renewedvision.com/hc/en-us/articles/360041814913-Installing-Bibles-in-ProPresenter) | Built-in installation of public-domain and licensed translations. |
| [Planning Center integration](https://www.planningcenter.com/integrations/propresenter) | Service playlist/media linking and Services LIVE integration. |
| [MultiTracks automation](https://helpcenter.multitracks.com/en/articles/8261041-propresenter-lyric-automation-what-do-i-need) | Provider-supported lyric automation/MIDI integration. |
| [RTMP streaming](https://support.renewedvision.com/hc/en-us/articles/360049229013-Streaming-a-ProPresenter-Screen-to-An-RTMP-Server) | Built-in screen capture to stream/file and configurable presets. |
| [Sync between computers](https://support.renewedvision.com/hc/en-us/articles/360041588774-Syncing-Between-Computers-with-ProPresenter) | Repository-based synchronization; distinct from live control and Cloud. |
| [ProPresenter 7 user guide PDF](https://files.renewedvision.com/propresenter/support/Pro7UserGuide.pdf) | Guide-index/search evidence for printing and legacy feature breadth; not the current release baseline. |

Limitations to preserve when reusing this document:

- This is an extensive inventory of documented user-facing workflows, not a claim to enumerate every preference, protocol endpoint, animation, format, proprietary decoder detail, or release-specific edge case.
- Source inspection can identify implementation, but only side-by-side behavioral/output tests can establish actual parity. No such installed-app comparison was performed here.
- Several older guide URLs redirected or failed when opened. Search-index excerpts supplied baseline descriptions; exact details should be checked in current support pages or ProPresenter before implementation decisions.
- The public download page mixes current release notes, historical versions, a beta label, and Cloud marketing. Do not infer production Cloud rollout, plan entitlement, or rich collaborative editing solely from that page.
- Features are platform/runtime/hardware dependent. The presence of cross-platform Rust and packaging scripts does not prove every output/audio/MIDI feature works on every platform.
- `N` means no substantive implementation found in the inspected commit. It is not an assertion that the feature is impossible or that future branches lack it.
- Broad inventories can change as ProPresenter releases features. Preserve the assessment date and upgrade the baseline explicitly rather than silently changing the target.

## 10. Maintenance instructions

1. Preserve existing feature IDs when work is renamed or split. Add a suffix or new ID for a materially different bounded capability.
2. Update the assessment date, Orbit version/commit, and ProPresenter baseline after a new audit.
3. Change a status only with a source path and a clear explanation of the newly completed workflow. Remove obsolete gap text at the same time.
4. For hardware-dependent rows, attach an acceptance record with platform/runtime/device details before claiming verified parity.
5. Keep import, authoring, runtime and output fidelity separate. Add a loss table for converter work rather than promoting every decoded field.
6. Record quality-gate failures openly. Never substitute code coverage or the number of rows for a product-parity percentage.
7. Reprioritize section 7 when the target user/workflow is agreed. This report does not authorize implementing every feature.

Recommended acceptance-record fields:

```text
Feature ID:
Assessment date:
Orbit version and commit:
ProPresenter version and platform:
Target hardware and runtime versions:
Fixture / workspace copy:
Expected behavior:
Observed behavior:
Screenshots / logs / test references:
Known limitations:
Result: pass / partial / fail / blocked by environment
Reviewer:
```

### Change history

| Date | Change |
| --- | --- |
| 2026-10-07 | Initial source-based parity inventory, official-source research, native/build validation, coverage-gap record, prioritized backlog and acceptance scenarios. |
| 2026-11-01 | INT-21: ProPresenter 21.4.2 probes fixed the shape gradient angle direction and showed that ProPresenter renders only two-stop linear shape gradients; timer display rules remain unverified. |
| 2026-11-01 | INT-21: removed Orbit-only gradient extensions for parity (shapes and text render two colors); recorded that Orbit-exported text gradients do not render in ProPresenter. |
| 2026-11-02 | INT-21/INT-16: text-gradient export writes ProPresenter's per-paragraph custom-attribute gradient copies, verified against ProPresenter 21.4.2 thumbnails. |
| 2026-11-02 | INT-16: text cut-out, media-fill and background-effect exports write the same per-paragraph copies; cut-out, invert and UTF-16 ranges verified by ProPresenter 21.4.2 probes, the media-fill image unverified. |
| 2026-11-02 | With the owner's approval, the renderer budget was raised from 400 to 448 KiB per JavaScript file and from 1,536 to 1,664 KiB in total for the remote, macro-collection, design-system and build-animation growth; the build is 1,539.1 KiB with a 413.4 KiB `automation-ui` chunk. macOS native tests now run with ffprobe and a 127.0.0.2 loopback alias in CI and resolve the `/var` temp-directory symlink; the pseudo-terminal serial test is Linux-only because serialport cannot configure macOS pseudo-terminals. |
| 2026-10-30 | Recorded the evening round: Remote fixes (CTL-12), macOS PowerPoint image import (INT-18), shape gradient types/length/blend points and timer display rules (INT-21), the INT-02 operator exclusion and the Mac blockers in a new remaining-scope ledger; corrected the inventory count table to I 194, P 6, N 11. |

## 11. Reviewed implementation follow-up

These records supersede the corresponding baseline gaps only within their stated scope. Hardware acceptance, authentic current-version import fidelity, and the final integrated verification remain separate requirements.

### Task 01 — Cue and theme editor coverage

Reviewed ten behavior tests for selection and ranges, theme assignment/insertion, split-copy references, native deletion errors, retained selection, and autosave cancellation. No production code or coverage threshold changed. Full renderer coverage passed: cue-editor branches **86.10%**, theme-editor branches **85.17%**, compared with the unchanged 85% gates. Renderer and test typechecks passed. Two full `npm run verify` attempts stopped at a concurrent bridge-contract line-cap violation; final integrated verification is pending. See implementation and validation evidence, [cue tests](../tests/renderer/slide-theme-controller.test.ts), and [theme tests](../tests/renderer/theme-delete-controller.test.ts).

### Task 08 — Actual media metadata (MED-12)

Reviewed native parsing, the typed worker-backed probe bridge, explicit unknown/error states, audio handling and stale-result protection. `npm run check`, `typecheck`, `maintenance:check`, 27 renderer/bridge tests and three native parser tests passed. A generated real MPEG-4 clip also passed the normally ignored native probe test (codec, dimensions, duration and 24 FPS). ffprobe is required on PATH or beside FFmpeg and packaging does not yet ensure it. The attempted app launch encountered the existing development server on port 5173, so manual native UI verification remains unproven.

### Task 12 — ProPresenter import fixture corpus (INT-10–13)

Reviewed six independently generated synthetic protobuf fixtures, an authored JSON-pointer manifest, three native tests, deterministic regeneration and explicit provenance. The native tests exercise decoding, workspace theme extraction/composition and export/reimport losses. `npm run check` and all three focused corpus tests passed. INT-10/12 remain partial and INT-13 remains unverified: these fixtures cannot establish authentic current-version support, visual equivalence or media copying/playback. Portable export losses are asserted for secondary text, media layers/actions, text geometry/font, groups and arrangements. See [corpus instructions and losses](../tests/fixtures/propresenter/README.md) and implementation and validation evidence.

### Task 06 — Independent audience and stage messages (AUT-07)

Reviewed the renderer/native show and clear paths and the shared due-event contract. Audience operations preserve stage text; stage operations preserve audience state. Existing replacement-instance protection still prevents stale delayed clears. All 29 focused renderer tests, the native regression, renderer typecheck and test typechecks passed. `npm run check` encountered formatting in another task's in-progress files; integrated verification is pending. Display/NDI hardware output was not manually verified.

### Task 02 — Import capability and pre-export loss reporting (INT-10–16)

Reviewed persisted source-file/structural diagnostics, unsupported fill detection, state-only action limitations, portable export field losses and native confirmation before destination selection/write. Export review releases the workspace lock during dialogs and revalidates both revision and findings before writing. Root review connected diagnostics to props, masks and stage layouts as well as presentations, themes and macros. Seven reporting tests, one decoder regression, `npm run check` and renderer typecheck passed before that small caller correction; final integrated checks cover it. Native dialog interaction remains unverified because the existing devserver occupied port 5173. INT-10/12/15 remain partial, INT-13 unverified and INT-16 unsupported as a lossless-export claim.

### Task 03 — Named Audience Looks (OUT-08, AUT-12)

Reviewed the right-inspector CRUD dialog, per-screen layer/theme/mask persistence, canonical responses, validated activation, workspace-switch draft isolation and matching automation semantics. Twelve controller/UI tests and full `npm run check` passed; typechecks passed. Sequential native screen mutations can partially apply if a later screen fails; the interface reports that failure and does not invent rollback. Manual display/NDI verification remains unproven after the occupied devserver prevented the attempted app launch.

### Task 04 — Macro editor and control panel (AUT-08, AUT-09)

Reviewed canonical workspace CRUD, trigger behavior, workspace-switch draft isolation and the reusable supported-action editor. Six controller/runtime/UI tests and both renderer/test typechecks passed. Existing list execution order, independent delay offsets, enabled flags and recursion guards are preserved. Concurrent message-validator/native-remote work temporarily blocked combined checks; final integrated verification is pending. Native UI launch encountered the occupied devserver. Collection/color/image/hotkey authoring remains outside this requested minimal surface, with imported values retained.

### Task 07 — Audience Messages panel (AUT-05, AUT-06)

Reviewed message CRUD, original-draft conflict checks, canonical persistence, workspace isolation, text/clock/timer tokens and show/clear reuse of the existing instance-guarded runtime. Five focused controller/UI tests passed. Required checks were attempted and encountered concurrent timer-contract/slide-editor integration errors; final combined verification is pending. Manual app launch encountered the occupied devserver. Timer/clock tokens resolve at show time; themed rendering and clear-after-timers remain explicit limitations.

### Task 10 — authenticated HTTP timer and prop control

Historical first remote implementation; the absent Look/macro scope described here is superseded by Authoring 11 below.

Timer start/stop/reset/increment and prop show/clear now use the native accepted-output path with target validation, authentication, revision/status feedback and published API documentation. Renderer recovery adopts timer runs and prop visibility. CTL-11 remains partial for looks/macros/capture. Focused native remote tests and two renderer recovery tests passed; npm check passed during agent validation. Physical output and running-app network control were not manually verified. See task 10 evidence and [Orbit HTTP API](remote-api.md).


### Task 5 — slide action authoring

The slide editor now creates, edits, orders and removes supported actions with explicit runtime support and delay semantics. Canonical slide-document persistence preserves imported details, unrelated edits and frozen program output; stale workspace results cannot replace new collections. Thirteen renderer tests and one native disk round-trip test passed, with renderer/test typechecks and focused lint. Native UI walkthrough was blocked by the occupied development port.


### Task 11 — calendar event authoring

The Calendar events dialog creates, edits and deletes scheduled events with local date/time, weekdays, UTC exception dates and an inclusive end date. Strict validation, canonical persistence and workspace isolation are covered by seven renderer tests and one native scheduler test. AUT-18 remains partial: existing UTC-day/local-time, pre-base-date recurrence and session-only idempotence semantics are preserved and disclosed in the UI. Native manual interaction was not completed.


### Task 09 — playback marker authoring (MED-15)

Historical first implementation; its wall-time policy is superseded by Authoring 07 below.

The video inspector now authors named markers and supported ordered actions with queued persistence, validation and stale-edit protection. Workspace/media/selection guards cancel retired actions, including nested delays and native optimized paths. Nine renderer tests and one native guard test passed; typecheck and maintenance passed. MED-15 remains partial: wall-time deadlines are unchanged by pause/seek/restart and run once per selection without loop rearming, disclosed in the UI.


## Final integrated review — 2026-10-07

All twelve tasks have separate commits, with additional reviewed integration fixes. `npm run verify` passed on the combined source: 1315 renderer tests, 448 native tests (7 ignored), 24 Node tests, maintenance, typechecks, native checks, build and unchanged size budgets. Cue-editor branch coverage is 86.38%; theme-editor is 85.17%, both above the unchanged 85% gates. Total renderer output is 1244.6 KiB against 1250 KiB. This final result supersedes historical concurrent-check failures in the task entries above.

Partial parity claims remain explicitly bounded by the individual task evidence.

The final settings smoke regression repair passed another complete `npm run verify`; aggregate branch coverage is 85.05% (12131/14262), with both editor feature coverage gates unchanged and passing. The production budget remains 1244.6 KiB.

The final renderer suite contains 1316 passing tests in 152 files. Fresh `npm run tauri:smoke:webview` also passed (exit 0): native write/restart and WebView write/restart reports all passed on 2026-10-07. Actual auxiliary settings styling, output/workspace round trips, presentation cue flow and slide-editor open/reflow were confirmed, with no viewport overflow or runtime error. Physical hardware and a manual walkthrough of every feature remain unverified.

### Authoring 07 — decoded playback markers (2026-10-08)

Video markers now wait for consumed native decoded frame timestamps. Thirty focused renderer tests, four native semantic tests and the executable FFmpeg pause/seek regression passed. MED-15 stays partial for rate controls and loop-relative repetition. Combined verification passed; see the final review below.

### Authoring 08 — custom video thumbnails (2026-10-08)

Native frame capture and reset passed focused renderer/native tests and real FFmpeg image inspection. Combined verification passed; see the final review below.

### Authoring 09 — missing media relink (2026-10-08)

Missing managed assets persist through saves and can be replaced with validated native files while retaining shared identities. Four native tests and seven renderer tests passed, including workspace leave-and-return retirement. Combined verification passed; see the final review below.

### Authoring 10 — independent announcements (2026-10-08)

Independent navigation, rotation, clear and screen routing passed fifteen renderer tests and four native tests, including separate clocks for the same video while main transport is paused. Routing persists; playback resets when loading a workspace. Combined verification passed; hardware acceptance remains unverified.

### Authoring 11 — remote Looks and macros (2026-10-08)

Authenticated HTTP and browser controls reuse local Look/macro execution. Acceptance differs from applied Look status and dispatched macro status; failures and absent acknowledgements are explicit. Thirty-eight native tests and twenty-five initial renderer tests passed. Forty-five renderer tests passed after workspace leave-and-return guards were added. See evidence and limits and [API contract](remote-api.md). Combined verification passed; see the final review below.

### Authoring 12 — independent presentation export (2026-10-08)

Standalone text, PNG and PDF export uses canonical scene building and a separate native compositor. Six native tests and four renderer tests passed; genuine PDF pages were rendered and their embedded pixels matched the numbered PNGs. Unsupported dynamic content, assets and fonts are rejected before destination selection. See [semantics, evidence and limits](presentation-export.md). Print is out of scope, so INT-20 remains partial. Combined verification passed; see the final review below.

## Final authoring integration review — 2026-10-08

All twelve authoring tasks have distinct feature commits. A fresh npm run verify passed on the combined source: **1401 renderer tests in 169 files, 486 native tests (9 ignored), and 27 Node tests**. Maintenance, formatting, lint, renderer/test typechecks, native checks and the production build passed. Aggregate branch coverage is **85.09%**, React settings **86.13%** and shell **85.58%**, with all existing gates unchanged. Renderer output is **1212.2 KiB** against the unchanged **1250 KiB** limit.

Current bounded inventory counts are **I 118, P 34, D 10, N 36, U 12; total 210**. This result supersedes pending combined-check statements from this authoring round; the dated 2026-10-07 review records the preceding round.

Fresh isolated Tauri smoke also passed native and WebView UI write/restart checks. The UI report confirmed workspace/output round trips, presentation cue flow, settings styling and editor loading without overflow or runtime errors. Manual acceptance of every new workflow and physical screen/audio/NDI output remains unverified.

## Final parity implementation review — 2026-10-09

Twelve distinct feature commits and separate integration fixes are complete. A fresh `npm run verify` passed all maintenance, formatting, lint, typechecks, native checks, test suites, unchanged coverage gates and the production budget. Aggregate renderer branch coverage is **85.11%**; renderer output is **1243.5 KiB** against **1250 KiB**. `npm run check` also passed. The native suite passed **549 tests with 12 ignored**, with selected runtime-dependent acceptance fixtures executed separately using genuine FFmpeg video, PNG and PDF files.

Fresh isolated Tauri native/WebView write and restart checks passed. A separate manual development launch exposed no usable controls to desktop automation, so it does not establish a manual walkthrough. Physical audio, screens, NDI, printers, external hardware, macOS/Linux and authentic ProPresenter comparisons remain unverified.

The current inventory is **I 118, P 46, D 7, N 33, U 6; total 210**. See the integration evidence and commit inventory and ten-round backlog covering all 92 remaining rows. These results supersede intermediate failures for this round without certifying overall interchangeability.


## Six-point parity round — 2026-10-10

Six bounded gaps received working authoring and native behavior: CUE-15, AUT-20, CTL-02, CTL-11, EDT-22 and OUT-21. The current inventory is **I 118, P 48, D 6, N 33, U 5; total 210**. The prior dated backlog is a historical snapshot; its affected row descriptions are superseded by this round.

## Third six-point parity round — 2026-10-10

Six bounded gaps were extended in separate feature commits: LIB-17, AUT-18, MED-22, OUT-25, AUT-16 and OUT-22. Each row remains partial within its documented comparison/hardware boundaries. Inventory remains **I 118, P 48, D 6, N 33, U 5; total 210**.

## Fifth six-point parity round — 2026-10-11

Six bounded capabilities were extended in separate feature commits: CTL-09, OPS-04, DAT-10, MED-20, CTL-11 and MED-14. Installed ProPresenter media/capture UI and official remote/ticker illustrations informed the operator surfaces. MED-14 advances from data-only to partial for persisted native desaturation; the other five remain partial within their documented limits. Current inventory is **I 118, P 49, D 5, N 33, U 5; total 210**.

## Seventh six-point extension round — 2026-10-11

Six bounded increments extend CTL-09 (playlist occurrences and named library arrangements), DAT-10 (start position and repeat distance), and MED-14 (hue and saturation). These families remain partial; inventory classifications and counts are unchanged.

## Eighth six-point extension round — 2026-10-11

Six bounded increments extend CTL-09 (media bins, audio bins and playback controls), AUT-15 (authored timed actions and chronological loop catch-up), and AUT-16 (exact repeated occurrence recording/replay). These families remain partial; inventory classifications and counts are unchanged.

## Ninth parity round — 2026-10-12

Twelve bounded increments extend DAT-10 (live file/RSS sources and feed formatting), CTL-09 (saved tickers, transport and seeking), and MED-14 (four object/text-fill color adjustments, preset authoring and application). Classifications and counts are unchanged.

## Tenth parity round — 2026-10-12

Twelve bounded increments extend MED-20 (audio marker/action authoring, real main and NDI clocks, guarded lifecycle and portable dependencies) and CTL-09 (video/audio seeking and restart, video boundary jumps and authored audio playlist navigation). Classifications and counts remain unchanged.

## Completed bounded capability: AUT-18 — 2026-10-12

AUT-18 moves from Partial to Implemented after completing the native global enable gate, strict recurrence validation, documented baseline comparison, full regression suite and manual Calendar control acceptance. Its existing scope, Calendar recurrence execution, is unchanged. The current inventory is **I 119, P 48, D 5, N 33, U 5; total 210**. This supersedes earlier pending/partial AUT-18 statements, including the historical calendar authoring assessment.

## Completed bounded capability: AUT-09 — 2026-10-12

AUT-09 moves from Partial to Implemented after completing collection organization/order, palette navigation and clipboard operations, strict native/import persistence, slide assignment, regression verification and native UI acceptance. Manual inspection found and resolved a table-layout cascade conflict. The existing scope, Macro CRUD / collections / palette, is unchanged. The current inventory is **I 120, P 47, D 5, N 33, U 5; total 210**. This supersedes earlier pending/partial AUT-09 statements.

## Completed bounded capability: AUT-20 — 2026-10-12

AUT-20 moves from Partial to Implemented after completing imported identity resolution, the library-playlist wire decoder, playlist selection/retrigger controls, independent announcement destination, strict target diagnostics, persistence and native operator acceptance. The existing scope, Jump/select presentation or playlist action, is unchanged. The current inventory is **I 121, P 46, D 5, N 33, U 5; total 210**. This supersedes earlier pending/partial AUT-20 statements.

## Completed bounded capability: CTL-05 — 2026-10-13

CTL-05 moves from Partial to Implemented after completing exact binary/text payload authoring, native TCP/UDP byte transfer, strict persistence, request/workspace-safe feedback and native operator acceptance. The existing scope, TCP / UDP communication actions, is unchanged. The current inventory is **I 122, P 45, D 5, N 33, U 5; total 210**. This supersedes earlier pending/partial CTL-05 statements.

## Completed bounded capability: AUT-06 — 2026-10-13

AUT-06 moves from Partial to Implemented after completing native Show timer initialization, monotonic instance deadlines, independent stage preservation and native operator acceptance of manual, timed and timer-completion dismissal. The existing scope, Message display / timed clear runtime, is unchanged. The current inventory is **I 123, P 44, D 5, N 33, U 5; total 210**. This supersedes earlier pending/partial AUT-06 statements.

## Completed bounded capability: CUE-16 — 2026-10-13

CUE-16 moves from Partial to Implemented after matching ProPresenter's Slide Timer: authentic `.pro` encoding captured from the installed application, foreground-video end-plus-delay timing in native automation, nil-UUID import handling and the no-timer encoding shown as Off. The existing scope, Go-to-next / completion actions, is unchanged. The current inventory is **I 124, P 43, D 5, N 33, U 5; total 210**. Hands-on Orbit operator rehearsal of foreground-video advance was not performed in this round.

## Completed bounded capability: CUE-19 — 2026-10-13

CUE-19 moves from Partial to Implemented after completing native Windows spelling-menu ownership, exact word replacement and autosave/reload acceptance, and safe whole-word Orbit corrections for collapsed cursors and partial selections. The existing scope, Spelling tools, is unchanged. The current inventory is **I 125, P 42, D 5, N 33, U 5; total 210**. Dictionary language availability belongs to the host platform; grammar analysis and macOS/Linux operator equivalence are not claimed.

## Completed bounded capability: AUT-11 — 2026-10-13

AUT-11 moves from Partial to Implemented after decoding authentic Clear, Stage, Timer and Prop action encodings captured from the installed ProPresenter and correcting their Orbit interpretation: Clear Announcement no longer clears main output, imported prop and stage targets resolve to imported Orbit records, Set Configuration timer actions persist their definition before control, and stage actions preserve unassigned screens. The existing scope is unchanged. The current inventory is **I 126, P 41, D 5, N 33, U 5; total 210**.

## Completed bounded capability: CUE-15 — 2026-10-13

CUE-15 moves from Partial to Implemented. The work decoded the authentic ProPresenter `Configuration/ClearGroups` document and wired it into workspace import and export. It also completed operator authoring of every ProPresenter clear-group option: eight layers, Stop Timeline, Clear Next, hide-in-preview, icon and tint, unique names, ordering and duplication. The existing scope is unchanged. The current inventory is **I 128, P 39, D 5, N 33, U 5; total 210**. `npm run verify` passed. With the owner's approval, the renderer budget was raised from 1,250 to 1,254 KiB; the build is 1,252.2 KiB. Hands-on Orbit operator rehearsal was not performed in this round.

## Completed bounded capability: LIB-17 — 2026-10-13

LIB-17 moves from Partial to Implemented. The existing bundle import now also opens authentic ProPresenter `.proBundle` and `.proPlaylist` archives into the open workspace, with presentations, extracted media and playlists; an authentic exported playlist archive was imported successfully. The existing scope is unchanged. The current inventory is **I 129, P 38, D 5, N 33, U 5; total 210**. No authentic archive containing media was available, and exporting to ProPresenter archives remains bounded by INT-15/16.

## Completed bounded capability: OUT-21 — 2026-10-14

OUT-21 moves from Partial to Implemented. Orbit now has ProPresenter's current test pattern set with its properties, animated patterns on a shared native clock, an Audio Video Sync pattern with a beep through the configured audio routing and delays, screen-group targets, Identify Screens and logo options. The existing scope is unchanged. The current inventory is **I 130, P 37, D 5, N 33, U 5; total 210**. Hardware calibration and artwork comparison were not performed.

## Completed bounded capability: OPS-01 — 2026-10-14

OPS-01 moves from Partial to Implemented. Recordings now include the recorded screen's audio mix, drained from the same native tap as NDI, placed on the recording timeline by timestamp and muxed as AAC. A 10-minute 1080p30 recording with audio was qualified: 17,999 frames, 4 repeated, 11 s to finalize. The existing scope is unchanged. The current inventory is **I 132, P 35, D 5, N 33, U 5; total 210**. Physical-device and cross-platform rehearsal were not performed.

## Completed bounded capability: AUT-05 — 2026-10-14

AUT-05 moves from Partial to Implemented. A message's template is now a theme slide, as in ProPresenter. It is authored in the Messages editor, rendered natively, imported from the authentic slide-UUID template reference and exported back by slide UUID. The same work fixed import of current single-file `Configuration/Messages` and `Configuration/Macros` documents; before this fix, authentic workspaces imported no messages or macros. The existing scope is unchanged. The current inventory is **I 133, P 34, D 5, N 33, U 5; total 210**. Visual comparison with ProPresenter output was not performed.

## Completed bounded capability: MED-15 — 2026-10-15

MED-15 moves from Partial to Implemented after the native Orbit author/save/reload and repeated paused marker-action rehearsal passed against the ProPresenter workflow. Three integration defects found during rehearsal were fixed: real video source matching, measured native duration for seeking, and renderer preview alignment with the armed marker timestamp. The current inventory is **I 135, P 32, D 5, N 33, U 5; total 210**. Full isolated renderer and native suites, project check, maintenance and the unchanged size budget passed.

## Completed bounded capability: INT-12 — 2026-10-15

INT-12 moves from Partial to Implemented after authentic layered text/image/video import, managed media ownership, source-removal/reload/native-render verification and ProPresenter/Orbit desktop comparison passed. The inventory is **I 136, P 31, D 5, N 33, U 5; total 210**. The full isolated native suite passed 687 tests with 21 existing ignored tests; project check, maintenance and the unchanged renderer budget passed. General cross-version compatibility remains unverified under INT-13.

## Progress on a partial capability: CUE-18 — 2026-10-15

CUE-18 stays Partial. Orbit now plays slide transitions natively. The precedence is slide, then presentation, then the workspace default, as in ProPresenter. Transitions are imported from and exported to ProPresenter fields 18 and 5, and the slide editor has an authoring panel. All 37 library effects are catalogued by UUID. Of these, 13 animate and the other 24 output as a cut, which is disclosed. Favorites and layer-level defaults are still absent. With the owner's approval, the renderer budget was raised from 1,255 to 1,259 KiB. The inventory is unchanged: **I 136, P 31, D 5, N 33, U 5; total 210**.

## Progress on a partial capability: EDT-19 — 2026-10-15

EDT-19 stays Partial. Web objects now render the live page on native display and NDI output. Each page loads in a hidden WebView2 webview; a worker captures it, and the compositor reads the latest frame. The capture rate is about 15 fps at 1080p, and scripts and animations keep running. ProPresenter web fills import as web objects, and only http(s) addresses load. The row stays Partial for three reasons: capture is Windows-only, output pages are not interactive and their audio is not routed, and no authentic ProPresenter web fill was available to compare. The inventory is unchanged: **I 136, P 31, D 5, N 33, U 5; total 210**.

## Progress on a partial capability: EDT-19 on macOS and Linux — 2026-10-15

EDT-19 stays Partial. Web objects now also capture on Linux, through WebKitGTK snapshots, and on macOS, through WKWebView snapshots. The Linux path passes the live page test under Xvfb. The macOS path is only type-checked by cross-compilation and has not run on a Mac. The inventory is unchanged: **I 136, P 31, D 5, N 33, U 5; total 210**.

## Progress on a partial capability: CUE-18 effect library — 2026-10-15

CUE-18 stays Partial. All 37 library effects now render natively. The 24 that previously output as a cut (warps, waves, 3D flips, cube and doors, mosaics, blurs, color and pattern effects) are reimplemented on the CPU and blended across threads. Effect variables, favorites and layer-level defaults are still absent. The inventory is unchanged: **I 136, P 31, D 5, N 33, U 5; total 210**.

## Completed bounded capability: MED-17 audio playlist progression — 2026-10-15

MED-17 moves from Partial to Implemented after authored-order playback, pause/resume, stop/next/repeat, canonical media persistence and native preview EOF ownership passed ProPresenter comparison and native Orbit acceptance. The original workspace and inactive outputs were restored. All 1,628 renderer tests and 698 native tests pass, alongside project check, maintenance and the unchanged renderer budget. The inventory is **I 137, P 30, D 5, N 33, U 5; total 210**.

## Progress on a partial capability: CUE-18 variables and favorites — 2026-10-15

CUE-18 stays Partial. Effect variables are authored, validated, rendered, imported and exported, and transition favorites are stored as `transitionPresets`, authored in the Transition panel and round-tripped through ProPresenter's `Presets/Transition` folder. Layer-level defaults stay absent because their storage in ProPresenter 7 is unknown here, and only slide layers use library transitions. The inventory is unchanged: **I 137, P 30, D 5, N 33, U 5; total 210**.

## Progress on a partial capability: MED-14 effect chains — 2026-10-15

MED-14 stays Partial. Image and video assets carry ordered chains of 38 ProPresenter library effects with their variables, authored in the media inspector and rendered natively after the color adjustments, with animated effects driven by the output clock. ProPresenter effect import/export, chains on media-filled objects and WebView previews remain open. The inventory is unchanged: **I 137, P 30, D 5, N 33, U 5; total 210**.

## Completed bounded capability: DAT-10 independent tickers — 2026-10-16

DAT-10 moves from Partial to Implemented after ProPresenter motion/distance comparison, native constant-velocity and copy-tiling regressions, and actual manual text, pause/edit/resume, watched-file, RSS revalidation and restart acceptance. Project check, maintenance, 54 focused renderer tests, 707 native tests and the unchanged renderer budget pass. The original workspace and disabled outputs were restored. Inventory: **I 138, P 29, D 5, N 33, U 5; total 210**.

## Completed bounded capability: MED-14 media effects and presets — 2026-10-16

MED-14 moves from Partial to Implemented. Chains of the 38 ProPresenter media library effects now cover assets, media actions (per selection), media-filled objects and glyph fills. They import from authentic-shaped and authentic ProPresenter media bins, media actions, object fills and effect presets, using ProPresenter's label naming. Named presets carry the chain and export to `Presets/Effect`, and the media inspector shows a natively rendered still. The effects are Orbit reimplementations; pixel-exact equivalence with ProPresenter's shaders is not claimed, and media bin export stays under INT-15. The inventory is **I 139, P 28, D 5, N 33, U 5; total 210**.

## Completed bounded capability: AUT-22 Capture actions — 2026-10-16

AUT-22 moves from Partial to Implemented. Cue and macro authoring now cover exact file recording presets, explicitly saved Active Settings, direct Stop and confirmed Stop. Imported commands retain their preset/confirmation semantics, queued commands preserve order, and confirmed stops verify recording identity. Two genuine cue-driven H.264/AAC recordings and the native audio regression passed; the original workspace and disabled outputs were restored. Streaming, per-channel audio mapping and hardware comparisons remain separate operations limits.

## Completed bounded capability: EDT-17 text media fill, cut-out and background effects — 2026-10-16

EDT-17 moves from Partial to Implemented. Text Cut Out now cuts through the text box fill to the backdrop on native output instead of drawing the editor's checkerboard. Slide fill videos on objects and glyphs restart with each slide occurrence and honor Loop, Stop, Hold, Ramp In and Ramp Out. The editor shares one background blur amount per slide, as ProPresenter does, and shows a native output still for background effects and cut-out text. Frame-exact comparison with ProPresenter and the exact 100 % blur radius are not claimed. The inventory is **I 142, P 25, D 5, N 33, U 5; total 210**.

## Completed bounded capability: CUE-18 full transition library and presets — 2026-10-16

CUE-18 moves from Partial to Implemented. Media, prop and message changes now play ProPresenter library transitions natively, as slides already did. A media selection's or prop's own transition comes first, then the workspace default of its layer. The four layer defaults are authored in the Transition panel and props in the props editor. ProPresenter's global layer transitions import from its Windows `AppPreferences.proPref` (Cut, Auto and Advanced) and the props document. Frame-exact comparison with ProPresenter and the macOS preferences location are not claimed. The inventory is **I 144, P 23, D 5, N 33, U 5; total 210**.

## Completed bounded capability: MED-22 smart-folder history — 2026-10-17

MED-22 moves from Partial to Implemented. Smart folders now remember the files they held. A file that disappears, also while Orbit is closed, stays listed under Missing media with its identity and settings. It can be relinked into managed storage under the same id or explicitly forgotten. ProPresenter smart media playlists import as smart libraries linked to the same folder, with the files of their last sync and their import behavior. Synced files that are gone are reported and listed as missing. Hands-on comparison with a running ProPresenter is not claimed. The inventory is **I 145, P 22, D 5, N 33, U 5; total 210**.

## Completed bounded capability: MED-20 per-asset audio inspector — 2026-10-17

MED-20 moves from Partial to Implemented. Per-asset routing follows ProPresenter's `Media.AudioProperties` for audio and video assets: up to eight source channels, each feeding ProPresenter channel 1 and/or 2 with its own gain, Mute, Solo and Compress/Limit, under a master gain. Audio assets can loop. The decoded audio clock counts passes, so audio markers repeat on each loop. Routes and audio Loop import from ProPresenter's media and audio bins. More than two logical channels (global routing), Loop for count/time and physical audition are not claimed. The inventory is **I 146, P 21, D 5, N 33, U 5; total 210**.

## Completed bounded capability: OPS-04 recording audio map and imported presets — 2026-10-17

OPS-04 moves from Partial to Implemented. Recording presets now author ProPresenter's custom recording audio map: one to eight recording channels, each summing the chosen channels of the screen's stereo mix, muxed as discrete AAC channels with no LFE. Imported ProPresenter presets now seed their screen, size, whole frame rate, bitrate and custom audio map into the editable draft. A map that needs more channels than the stereo mix is disclosed and not applied. Codecs other than H.264 MP4, RTMP/Resi and a hands-on ProPresenter comparison are not claimed. The inventory is **I 147, P 20, D 5, N 33, U 5; total 210**.

## Completed bounded capability: INT-20 video and animated media in static export — 2026-10-17

INT-20 moves from Partial to Implemented. Slide export, PDF, print and thumbnail handouts no longer reject slides with video or animated media. Video backgrounds draw the full-resolution frame at their media thumbnail time. Slide video objects, text video fills and animated images draw their first frame. Frames are decoded once per export into temporary stills, so preflight and final rendering match exactly. Slide actions no longer block export. Live/web/NDI sources and audio objects still fail explicitly, and physical printing is not claimed. The inventory is **I 148, P 19, D 5, N 33, U 5; total 210**.

## Completed bounded capability: CTL-09 remote messages, stage layouts and clear groups — 2026-10-17

CTL-09 moves from Partial to Implemented. The browser remote now covers the ProPresenter Remote views it lacked:

- **Messages:** show a saved audience message or clear it.
- **Stage Screen Layouts:** assign a layout per stage screen.
- **Clear Groups:** run a saved clear group.

Each follows the validated queued look/macro path and runs the local controller. Status reports the active message and each stage screen's layout. Remote definition editing (CTL-11) and a physical comparison with the Remote app are not claimed. The inventory is **I 149, P 18, D 5, N 33, U 5; total 210**.

## Completed bounded capability: MED-13 video transport — 2026-10-17

MED-13 moves from Partial to Implemented. Video asset and action timing now covers authoring, strict persistence, ProPresenter import, native selection overrides and Restart/Continue/Automatic behavior. Live Orbit acceptance verified save/reload, native playback, delay, trim, pause, paused seek, end/replay and Continue after the end. The inventory is **I 150, P 17, D 5, N 33, U 5; total 210**.

## Completed bounded capability: CTL-03 native MIDI input — 2026-10-17

CTL-03 moves from Partial to Implemented. MIDI triggers no longer depend on Web MIDI, which WKWebView and WebKitGTK do not provide. The native host opens MIDI inputs through `midir` only while a binding has a MIDI note. It picks up hotplugged and reconnected devices, filters note-on natively and the renderer dispatches it through the shared binding runtime. Settings > Trigger bindings shows the open devices and any open errors. A physical controller rehearsal on each platform is not claimed. The inventory is **I 151, P 16, D 5, N 33, U 5; total 210**.

## Completed bounded capability: AUT-16 timeline recording — 2026-10-18

AUT-16 moves from Partial to Implemented. Record arms at the entered position and Play starts recording. The live playhead, repeated cue clicks, pause exclusion, stopped draft editing, strict save and native replay are covered by regression tests and real Windows save/restart/replay acceptance. The installed ProPresenter timeline supplied the UI comparison. Soundtrack synchronization, graphical multitrack editing and external timecode remain AUT-15 gaps. The inventory is **I 152, P 15, D 5, N 33, U 5; total 210**.

## Completed bounded capability: CTL-11 remote macro completion and stored definitions — 2026-10-18

CTL-11 moves from Partial to Implemented. A remote macro request now ends in an authoritative `completed` or `error`: `completed` only after every enabled action, including delayed actions, nested macros and awaited native operations, has finished, and `error` with the failed action of each failure otherwise. Authenticated clients can also edit persisted timer definitions (name, countdown duration, overrun) and message definitions (text and text token values), as the ProPresenter Remote app allows for timers and messages. Each edit is bound to the workspace, validated natively under the workspace lock and persisted through the local editors. Marker actions, automatic prop and message clears, physical output delivery and a hands-on comparison with the Remote app are not claimed. The inventory is **I 153, P 14, D 5, N 33, U 5; total 210**.

## Completed bounded capability: CTL-02 group shortcuts and trigger bindings — 2026-10-18

CTL-02 moves from Partial to Implemented. Presentation-command authoring and twelve observed ProPresenter menu imports now join the existing cue, group, prop, timer, macro and clear-group targets. Native cross-scope validation, a command/input table with search and conflict filtering, immediate main-runtime synchronization and native main-window test requests complete the saved binding workflow. Real Windows conflict, save, keyboard/group dispatch and full-restart acceptance passed. Commands outside the presentation-command catalogue and physical MIDI/platform certification are not claimed. The inventory is **I 154, P 13, D 5, N 33, U 5; total 210**.

## Completed bounded capability: AUT-21 external presentation action — 2026-10-18

AUT-21 moves from Partial to Implemented. The imported external-presentation action no longer opens a URL. The native host drives Microsoft PowerPoint through COM on Windows (verified with a real PowerPoint run) and Keynote or PowerPoint through AppleScript on macOS (compile-checked only). It opens the referenced file read-only, starts its slideshow and supports next, previous, go-to and stop. The operator's slide commands and a status strip control the show. Only the presentation Orbit opened is closed, and a PowerPoint that was already running is never quit. Remote URLs, Linux and missing applications fail with an explicit error. Embedding the slideshow in Orbit output windows is not claimed. The inventory is **I 155, P 12, D 5, N 33, U 5; total 210**.

## Completed bounded capability: OUT-25 NDI alpha — 2026-10-18

OUT-25 moves from Partial to Implemented. Native alpha-only changes retain the existing sender and audio timestamps, recompose current content and retire delayed frames from the previous mode. Actual NDI SDK discovery, transmission and receiving verified Straight/Premultiplied/None and matching receiver keying, with zero and two frames of delay. Installed ProPresenter controls and native Orbit save/full-restart acceptance were verified. This is Windows NDI acceptance, not certification of every broadcast keyer or other platform. The inventory is **I 156, P 11, D 5, N 33, U 5; total 210**.

## Completed bounded capability: OUT-13 stage multiview — 2026-10-18

OUT-13 moves from Partial to Implemented. Valid nested outputs now use the full existing sixteen-source allowance, with memoized validation, explicit cycle rejection and child chart validation/preparation. Output/chart placeholders, chart scaling and revised English/Dutch help complete the stage-editor controls. Actual six-panel native-window and NDI receiver acceptance, independent video clocks, final-cue clearing and native save/full-restart verification passed. Existing resource/raster limits and separate platform/hardware classifications remain explicit. The inventory is **I 157, P 10, D 5, N 33, U 5; total 210**.

## Completed bounded capability: OUT-22 video delay — 2026-10-18

OUT-22 moves from Partial to Implemented. Delay-only NDI edits retain the sender and audio timestamp while retiring pending frames. The selected-output frame control now shows the duration at the native cadence. Actual NDI receivers measured median additional delays of 207.4 ms for six frames at 30 fps and 210.2 ms for twelve frames at 59.94 fps, within the explicit acceptance tolerance. Installed ProPresenter controls and native Orbit save/full-restart acceptance passed. Physical A/V chain calibration, identical ProPresenter latency and other-platform hardware qualification are not certified. The inventory is **I 158, P 9, D 5, N 33, U 5; total 210**.

## Completed bounded capability: DAT-14 portable PDF decoding and imported charts — 2026-10-18

DAT-14 moves from Partial to Implemented. Stage chord-chart PDFs are rasterized by one pure-Rust decoder (`hayro`) on Windows, macOS and Linux, replacing the Windows-only `Windows.Data.Pdf` path, with the original 32 MiB, 1-64 page, 2048-pixel and 128 MiB decoded-pixel bounds and the managed PNG format unchanged. During ProPresenter import, a presentation-level `chordChartUrl` that names an existing local PDF, PNG or JPEG is imported through the manual import path and attached to every slide of that presentation (page 1; ProPresenter stores no page). Missing, remote and unsupported references are kept on the presentation and listed in the import warnings. macOS and Linux were not run here, and no physical stage comparison with ProPresenter was made. The inventory is **I 159, P 8, D 5, N 33, U 5; total 210**.

## Completed bounded capability: CUE-20 presentation and playlist printing — 2026-10-18

CUE-20 moves from Partial to Implemented. Ordered presentation selections, an actual PDF preview and direct Windows printer/page-range/driver controls complete the existing canonical thumbnail/outline handouts. Native cancellation and a real Microsoft Print to PDF job verified selection order, notes, captions and printing only the chosen handout page. Printer copies follow driver support; physical delivery and other-platform printer qualification are not certified. The inventory, including the separately completed DAT-14 work, is **I 160, P 7, D 5, N 33, U 5; total 210**.

## Completed bounded capability: OPS-12 Linux deployment — 2026-10-18

OPS-12 moves from Partial to Implemented for the documented Ubuntu 24.04 x64 X11/XWayland baseline. Actual Linux Debian and AppImage builds, installed Debian restart/removal, normal AppImage FUSE launching and extract-and-run passed. Both packages completed native and embedded-renderer write/verify phases under WSLg and Xvfb: 16 successful process phases. Explicit GTK backend/handle initialization, Debian FFmpeg dependency and a reproducible release package gate complete the deployment path. Direct Wayland, other distributions and physical hardware qualification remain outside this baseline. Linux is an Orbit distinction, not a ProPresenter compatibility claim. The inventory is **I 161, P 6, D 5, N 33, U 5; total 210**.

## Completed bounded capability: AUT-15 timeline soundtrack, timecode chase and multitrack lanes — 2026-10-18

AUT-15 moves from Partial to Implemented. A timeline can have one soundtrack (a workspace audio asset, validated natively and in the renderer, imported from the ProPresenter timeline audio action with an explicit warning when the file cannot be mapped). Rust plays it on the main audio output in sync with the authored transport: a new generation seeks, pause and resume follow the running flag, drift above 0.10 s is corrected, a loop wrap restarts it and a stopped or retired transport unloads it. Timelines carry a validated timecode switch and start point, and an application setting selects LTC from an audio input channel (a native biphase-mark decoder for 24, 25, 29.97 drop-frame and 30 fps) or MTC from a MIDI port (quarter-frame and full-frame messages). The selected presentation's timeline follows the timecode (position = timecode − start; restart on a jump above 0.5 s, pause on stopped timecode or after a 500 ms dropout; nothing fires before the start) and the panel shows the lock status. The panel shows slide, action and soundtrack lanes over one zoomable axis with a native, cached waveform and draggable or keyboard-movable cue markers. Decoder and chase behaviour is verified with generated LTC audio, synthetic MTC messages and state-machine tests; no LTC or MTC hardware was available, so real-equipment lock, drift and latency are untested, and the panel was not exercised in a hands-on desktop session. Playlist-wide timecode, additional audio or media tracks and routing the soundtrack into the SDI/NDI mix are not claimed. The inventory is **I 162, P 5, D 5, N 33, U 5; total 210**.

## Completed bounded capability: INT-10 workspace import — 2026-10-19

INT-10 moves from Partial to Implemented. Native workspace import now preserves folders, empty content and source order, uses exact/explicit workspace-relative presentation references, retains missing occurrences with warnings, remaps header/keyboard prop references, and captures media in props, masks and stage layouts. Combined configuration-family acceptance verifies source preservation, failed-import cleanup, editing/reloading and exact native output pixels after removing the source. The 7.16.2 filesystem contract and documented authentic 21.4.2 objects are the baseline; general newer-version compatibility and lossless export remain separate. The inventory is **I 163, P 4, D 5, N 33, U 5; total 210**.

## Completed partial capability: EDT-19 — 2026-10-19

EDT-19 moves from Partial to Implemented for the native live-webpage raster workflow. Real browser probes now pass on Windows, macOS and Linux, including page-script updates, straight alpha, idle retirement, reopening and shutdown. Windows PNG preserves transparent web overlays; macOS TIFF associated alpha is normalized before composition. A ProPresenter-authored web object verifies native URL, geometry and UUID import, and a real WebView2/compositor test verifies alpha over an existing background. The bounded raster scope and remaining browser/audio, throughput and physical-target verification limits are explicit in the evidence. The inventory is **I 164, P 3, D 5, N 33, U 5; total 210**.

## Completed bounded capability: INT-15 workspace structure export — 2026-10-20

INT-15 moves from Partial to Implemented. The remaining native preset acceptance passed in ProPresenter 21.4.2 for Windows. The run was driven through Windows UI Automation because the transition picker closes on focus loss. The exported `Orbit INT-15 Fade` favorite is listed, selected and set as the active transition with its duration. The exported `Orbit INT-15 Sepia` effect preset is listed and applied to media, rendering in sepia. ProPresenter's own saved favorite and effect preset match Orbit's encoding apart from optional instance UUIDs. The acceptance exposed that ProPresenter labels effect presets by file name. Preset files now keep their titles, with dots replaced as ProPresenter does, and the export loss review reports any renamed effect preset. The inventory is **I 165, P 2, D 5, N 33, U 5; total 210**.

## Completed partial capability: MED-23 — 2026-10-20

MED-23 moves from Partial to Implemented for HDR on native display outputs. Screens choose SDR, Rec. 2100 HLG or PQ and Settings holds the Normal/Enhanced rendering bit depth, applied at startup. HDR frames are composed into a 16-bit intermediate and presented to a 10-bit swap chain after sRGB-to-Rec.2020 conversion, BT.2408 203-nit reference white and PQ/HLG encoding, with the DXGI (Windows, PQ) or CAMetalLayer (Apple Silicon, PQ and HLG) color space applied and reported to the operator. Validation rejects NDI, capture, Windows HLG, Intel Macs and a normal bit depth. A GPU readback test matches the CPU reference. No HDR monitor hands-on check was performed, the macOS path is only compile-checked in isolation, and compositor sources remain 8-bit SDR content. The inventory is **I 166, P 1, D 5, N 33, U 5; total 210**.

### 2026-10-21: NDI-10 independent Video Input layer

NDI-10 moves from Data/runtime only to Implemented for the bounded NDI-backed workflow. The inventory is **I 167, P 1, D 4, N 33, U 5; total 210**. Operator authoring, persistence, independent switching/clearing, actions, per-output Looks and actual received-frame composition are covered. The existing renderer size-gate failure and limited manual desktop verification remain explicit in the completion evidence.

### 2026-10-21: MED-21 native audio inputs

MED-21 moves from Data/runtime only to Implemented for native device capture and mix-mode authoring. The inventory is **I 168, P 1, D 3, N 33, U 5; total 210**. Named sources, channel pairs, gain/delay/latency, shared audience mode gating, cue/macro overrides, Settings-window synchronization and real captured PCM are established. Stereo mixer limits, separate NDI input audio, platform calibration and the existing renderer size gate remain explicit in the completion evidence.

### 2026-10-21 — Serial output and Planning Center scope

The operator explicitly deferred LIB-18 and DAT-15 because Planning Center is not used. They remain D, rather than being counted as implemented. CTL-06 moves from D to P: configured serial output now executes through the native host, with port discovery, exact UTF-8/binary payloads, strict settings, saved devices and action targets. Production-device protocol handling is still outstanding. Counts are **I 168, P 2, D 2, N 33, U 5; total 210**.

### 2026-10-21 — NDI-07 input audio trace

NDI-07 moves from Unverified to Not found. The source trace shows the NDI receiver discards audio and the audio input path has no NDI source kind; no production code changed. The inventory is **I 168, P 2, D 2, N 34, U 4; total 210**. No hardware rehearsal was performed.

### 2026-10-21 — EDT-12 paragraph and tab-stop trace

EDT-12 moves from Unverified to Not found. The source trace finds no tab stop, indent, paragraph spacing, line height or list support in import, persistence, authoring, rendering or export; only alignment exists. No production code changed. The inventory is **I 168, P 2, D 2, N 35, U 3; total 210**. No hands-on ProPresenter comparison was performed.

### 2026-10-21 — Companion HTTP contract

CTL-14 moves from Unverified to Partial. `docs/remote-api.md` now documents how Bitfocus Companion's generic HTTP client can drive Orbit, and a parser test pins every published request body. There is no dedicated Companion module, no ProPresenter API compatibility and no push feedback channel, and no live Companion instance was run. The inventory is **I 168, P 3, D 2, N 35, U 2; total 210**.

### 2026-10-21 — NDI-07 live input audio

NDI-07 moves from Not found to Implemented. Audio inputs can now select an NDI source instead of a device; a dedicated audio-only receiver feeds the selected channel pair through the existing input gain, mode, Auto On, main-mix and NDI/recording tap, with Settings authoring, strict persistence, cue/macro targets and native error reporting. The inventory is **I 169, P 3, D 2, N 34, U 2; total 210**. The receiver, channel selection and capture into the ring and NDI tap ran against the installed NDI runtime with a local sender; the later mixer stage is shared with device inputs. Listening on hardware, a hand-run Settings session and long-run clock drift were not verified, and an audio input and a Video Input on the same source use two receiver connections.

### 2026-10-21 — EDT-12 layer paragraph formatting

EDT-12 moves from Not found to Partial. Text layers now carry line height, paragraph spacing, first-line, left and right indents, left/right/center tab stops with a default interval, and a layer-wide list (disc, circle, square, hyphen, decimal, lower/upper alpha and roman, with prefix, postfix and start). Native and WebView rendering share the same formulas and are checked against shared fixture tables; values import from ProPresenter ParagraphStyle and `text_list`, and anything unsupported is dropped and reported. Per-paragraph and per-run formatting, nested and per-paragraph lists, decimal and justified tab stops, wrapping of tabbed lines, `line_spacing`, minimum/maximum line height and export are absent, and no authentic ProPresenter fixture with these attributes was compared. The inventory is **I 169, P 4, D 2, N 33, U 2; total 210**.

### 2026-10-21 — INT-13 newer ProPresenter versions

INT-13 moves from Unverified to Partial. The decoder's reads (137 messages, 522 field numbers) were compared with the vendored 7.16, 7.16.2, 19 beta and 21.4 schemas: no field was renamed, renumbered, retyped or moved into a oneof, and the checked-in authentic 21.4.2 documents match the 21.4 schema field for field; the only field they carry beyond 7.16.2 is a typographic run attribute. The comparison also exposed five decoder reads that never matched real data, now fixed: external-presentation action URLs, underline and strikethrough, prop-slide action slides, a dead glyph media-fill branch (now reported) and prop auto clear without its enabled flag. Newer content is reported rather than dropped (alternate texts, data links, single-prop collections, unsupported playlist items, unreadable slide objects), prop-collection cues and collection-embedded macros are imported, unreadable cues reject their presentation, enum values outside the schema no longer become "clear all" or "start", unknown-group nesting is bounded, and one warning names each ProPresenter release newer than 21.4.2. Twenty synthetic-fixture tests include a byte-mutation sweep of every fixture. No authentic files from ProPresenter 18, 19, 20 or other 21.x document families were available, so the matrix for them is schema-level; unconverted features Orbit supports elsewhere (scrolling text, capitalization, text fills, feathering) remain unimported. The inventory is **I 169, P 5, D 2, N 33, U 1; total 210**.

### 2026-10-22 — OPS-17 capacity and redundancy

OPS-17 moves from Unverified to Partial. An opt-in harness (`ndi_capacity_tests.rs`, all tests ignored) measures composition and the real NDI sender at 1080p and 2160p with p50/p95/p99/max cue and frame cost, delivered fps, missed frames, memory and CPU, and a soak mode checks memory, handle, thread, decoder and frame-time drift. On the one tested Windows machine, shared with a running ProPresenter, 1080p text slides sustain 50 fps and image backgrounds 30 fps, 1080p video held 30 fps only when the machine was in its faster state, and 2160p is not qualified; 30-minute soaks at 1080p and 2160p showed no growth. [docs/operations-failover.md](operations-failover.md) is a manual backup-machine runbook; there is still no state sync, watchdog or automatic switching. Full-application restart time, other hardware, physical outputs, a two-machine rehearsal and a real show were not measured. The inventory is **I 169, P 6, D 2, N 33, U 0; total 210**.

### 2026-10-22 — INT-13 and OPS-17 accepted

INT-13 and OPS-17 move from Partial to Implemented on the operator's decision. Their evidence is unchanged: INT-13 still has no authentic ProPresenter 18, 19, 20 or other 21.x files, and OPS-17 still lacks qualified 2160p video, full-application restart time, other hardware, physical outputs and a two-machine rehearsal. Defects found with real files or show hardware are to be fixed as they appear. The inventory is **I 171, P 4, D 2, N 33, U 0; total 210**.

### 2026-10-22 — CTL-14 push feedback and Companion module

CTL-14 moves from Partial to Implemented. Orbit serves `GET /api/v1/events` (server-sent events, covered by `remote_events.rs` tests), and a separate Companion module consumes it. Not verified against a running Companion instance. The inventory is **I 172, P 3, D 2, N 33, U 0; total 210**.

### 2026-10-23 — LIB-18 Planning Center playlist linking

LIB-18 moves from D to I on the operator's request, with an honest limit: it was built and tested only against a local mock HTTP server, because no Planning Center account is available. A Personal Access Token (Application ID and secret) is entered in Settings > Planning Center and stored in its own app-data file, never in `settings.json`, the workspace or the renderer. A playlist can be linked to an upcoming plan and synced on demand; sync replaces the playlist entries with the plan's headers, matched songs and placeholders for unmatched songs, and reports unmatched songs and skipped items. DAT-15 remains D. The inventory is **I 173, P 3, D 1, N 33, U 0; total 210**.

### 2026-10-23 — DAT-15 Planning Center LIVE stage data

DAT-15 moves from D to I on the operator's request, with the same honest limit as LIB-18: it was built and tested only against a local mock HTTP server, because no Planning Center account is available. Turning on Planning Center Live for a linked playlist starts a native 5-second poll of the plan's live resource, reusing the LIB-18 client and credentials. The values reach stage and text layers through a new "Planning Center Live" linked-text source (current item, next item, time elapsed, time remaining) in both the native compositor and the WebView. On 401, 403, 404, network or JSON errors the values are cleared and the error is shown; polling stops on toggle off, when the playlist's plan link or the workspace changes, when credentials are cleared and at exit. The inventory is **I 174, P 3, D 0, N 33, U 0; total 210**.

### 2026-10-23 — INT-06 covered by LIB-18

INT-06 moves from Not found to Implemented: the LIB-18 client provides authenticated service browsing and plan synchronization. No real Planning Center account was used. The inventory is **I 175, P 3, D 0, N 32, U 0; total 210**.

### 2026-10-23 — INT-07 Planning Center attachment download

INT-07 moves from Not found to Implemented as download only: a linked playlist's Planning Center dialog lists the plan's attachments and imports the chosen image, video and audio files into the workspace media. Uploading to Planning Center is not provided, matching the download direction of ProPresenter's integration. It was built and tested only against a local mock HTTP server, because no Planning Center account is available. The inventory is **I 176, P 3, D 0, N 31, U 0; total 210**.

### 2026-10-23 — OPS-02 RTMP streaming

OPS-02 moves from Not found to Implemented. A recording preset can now stream to an `rtmp://` or `rtmps://` URL with a write-only stream key, reusing the capture pipeline, status and start/stop paths. It was tested only against a local FFmpeg RTMP listener; no real streaming platform was used. The bundled FFmpeg build has no TLS, so RTMPS works only with an FFmpeg build that has it, and Resi (OPS-03) is not covered. The inventory is **I 177, P 3, D 0, N 30, U 0; total 210**.

### 2026-10-23 — INT-17 PowerPoint import

INT-17 moves from Not found to Implemented as a bounded native `.pptx` importer: slides in order, editable text layers with inherited placeholder geometry and resolved formatting, pictures copied into the workspace media, solid backgrounds and speaker notes. Tables, charts, SmartArt, media, animations, transitions, gradients and shape graphics are dropped and listed in an import report. It was tested only on small generated fixtures; no real-world decks were compared. The inventory is **I 178, P 3, D 0, N 29, U 0; total 210**.

### 2026-10-21 — Outgoing RossTalk authoring and approved renderer budget

CTL-06 stays P while adding explicit Carbonite/Graphite and Acuity command authoring to cue/macro actions and device tests. Five command families use validated parameters and exact CR/LF framing; six real renderer-to-native TCP loopbacks passed. Broader vendor/inbound/session support remains outstanding. The operator explicitly requested a renderer budget increase: total 1536 KiB, unchanged CSS/JS per-file limits, current build 1315.3 KiB. Inventory stays **I 168, P 2, D 2, N 33, U 5; total 210**.

### NDI input audio update — 2026-10-21

NDI-07 moves from U to P after implementing and verifying the real receiver-to-output stereo path. Its remaining multichannel matrix gap is explicit. Counts are **I 168, P 3, D 2, N 33, U 4; total 210**. Planning Center remains deferred, old Sony BVS/GVG100 switchers remain retired, and ordinary serial communication is retained at the operator's request.

### NDI input-bus matrix completion — 2026-10-21

NDI-07 moves from P to I after closing the multichannel matrix gap. Local and NDI inputs author up to 64 source channels into 16 buses, with per-route gain/mute and explicit Auto/Clear. The native capture, mixer and independent main/NDI output matrices retain higher buses. Real installed-runtime acceptance verifies eight-channel input through bus 16 and a separate 16-channel NDI output receiver, alongside stereo and video regressions. All 1837 renderer tests and 95 native checks passed, as did the required source/maintenance checks and existing renderer budget. Counts are **I 169, P 2, D 2, N 33, U 4; total 210**. Physical-device, external-source and Mac certification remain verification limits.

### 2026-10-23 — NDI-08 USB camera / capture device input

NDI-08 moves from Not found to Implemented for the bounded capability: a Video Input can use a locally attached camera instead of an NDI source, listed and captured through FFmpeg and composed through the existing live-input layer. Windows was exercised with a real USB webcam (listing and 1280x720 capture) and an FFmpeg `lavfi` stand-in; macOS and Linux are untested, and the bundled macOS FFmpeg recipe uses `--disable-autodetect`, which disables AVFoundation, so macOS capture needs a different FFmpeg build until the recipe is changed. The inventory is **I 179, P 3, D 0, N 28, U 0; total 210**.

### 2026-10-23 — Merge of the parallel parity branches

The two parallel branches were merged into `main`. Where both built the same capability, NDI-07 keeps the multichannel input-bus implementation (the earlier stereo-pair implementation was reverted before the merge) and CTL-14 keeps the in-repository Companion module together with the `GET /api/v1/events` push stream. The inventory is **I 189, P 8, D 0, N 13, U 0; total 210**.

### 2026-10-24 — EDT-12 runs survive plain text edits

EDT-12 stays Partial. Per-run formatting of the slide text is no longer dropped by a plain text edit in the cue editor or the text inspector, or by splitting a slide: `remapContentRuns` keeps the formatting of the unchanged start and end and gives inserted text the style before it. A rewrite with nothing in common still drops the runs, and merging two slides keeps only the formatting of the slide merged into. The inventory is unchanged: **I 189, P 8, D 0, N 13, U 0; total 210**.

### 2026-10-24 — EDT-12 run export

EDT-12 stays Partial. The portable ProPresenter export now writes bold, italic, underline, strikethrough and text color runs into the slide RTF instead of reporting them as an export loss; an exported file imports back with the same formatting, with uncolored runs taking the export text color. Paragraph formatting, tab stops and lists are still not exported, and the RTF was not opened in ProPresenter itself. The inventory is unchanged: **I 189, P 8, D 0, N 13, U 0; total 210**.

### 2026-10-24 — EDT-12 slide merges keep both slides' runs

EDT-12 stays Partial. Merging two slides in the cue editor now joins the runs of both slides instead of keeping only those of the slide merged into. The inventory is unchanged: **I 189, P 8, D 0, N 13, U 0; total 210**.

### 2026-10-24 — AUT-17 reverse and variable-speed chase

AUT-17 stays Partial. LTC read backwards (the sync word is recognised in reverse bit order) and MTC quarter-frames arriving 7 to 0 are decoded as valid timecode with a direction, and the chase scrubs the timeline to the incoming position instead of unlocking. The input speed is measured from the last second of readings; when it is more than 5% from 1x the chase re-anchors the running timeline at every reading, at most every 200 ms. The native timeline clock, cue scheduler and soundtrack only run at 1x and forwards, so true slow, fast or reverse playback is not implemented: reverse chase is a paused seek (new transport generation, live slide follows), and slow or fast chase is position following. A timeline soundtrack therefore does not play backwards or at varispeed. No physical LTC/MTC generator was available, so decoding was verified with synthetic signals only. The inventory is unchanged: **I 189, P 8, D 0, N 13, U 0; total 210**.

### 2026-10-24 — EDT-12 per-run font family and size

EDT-12 stays Partial. A run of the slide text can now carry its own font family and font size next to bold, italic, underline, strikethrough and text color. They are set from the font and size fields of the slide text toolbar, stored in strict version 1 `contentRuns` (family a non-empty string, size a positive number in the unit of the layer `fontSize`, both rejected otherwise), rendered by the WebView in canvas height units like the layer font size and by the native compositor, where the run size scales with the layer font size and a line takes the height of its tallest run. The RTF import reads `N` through `onttbl` and `sN` half-points, keeping a font or size that is the same throughout the text as the layer font rather than as run formatting; the export writes both. A run family that is not installed falls back through the system font matching and was not compared against ProPresenter. The inventory is unchanged: **I 189, P 8, D 0, N 13, U 0; total 210**.

### CTL-15 Network Link completion — 2026-10-24

CTL-15 is now Implemented for Orbit-to-Orbit groups: discovery and reciprocal membership, indexed media/audio and show controls, timer/message editing, bounded independent peer delivery and loop-safe group forwarding are complete. Native app acceptance and a three-node HTTP cycle passed. ProPresenter wire interoperability and physical multi-computer deployment certification are not claimed. The inventory is **I 190, P 7, D 0, N 13, U 0; total 210**.

### 2026-10-24 — CTL-06 Grass Valley AMP receiver

CTL-06 stays Partial. A deck input can receive Grass Valley AMP over TCP: `CRAT`, `CMDS` and `STOP` messages carry the Sony BVW play, stop, cue and sense commands to the live clip, with replies in AMP hex. The framing follows two public AMP clients and the SimplyLive AMP manual; no Grass Valley controller or specification was available, and AMP clip loading is rejected. The inventory is unchanged: **I 190, P 7, D 0, N 13, U 0; total 210**.

### 2026-10-24 — INT-16 layered slide and media export

INT-16 moves from Not found to Partial. A presentation's layers now export to the `.pro` in the field mapping the importer reads: text, shapes, color/gradient/no fills, strokes, shadows and image and video media fills in their stacking order, with the media files copied into the show's `Media` folder and referenced by absolute URL and show-relative path. Export-then-import tests reproduce a multi-layer slide (text, image, video, shapes, a pen path) layer for layer, text-only slides export unchanged, and the export loss review now names each still-unexported layer field instead of reporting every layer as lost. Builds, data links, web/slide-image/background-effect fills, media effects and transport details, paragraph spacing and lists, audio media and the workspace media bin remain reported losses. The `.pro` files were not opened in ProPresenter itself. The inventory is **I 190, P 8, D 0, N 12, U 0; total 210**.

### 2026-10-24 — CTL-12 ProPresenter Remote and Stage Display protocol

### 2026-10-24 — EDT-12 paragraph export

Portable ProPresenter export now preserves the primary text layer's supported paragraph fields, tab coordinates and list metadata after composing the assigned theme and slide overrides. Matching RTF paragraph controls accompany the protobuf. Zero-based decimal numbering is reported as an explicit round-trip loss. Native filesystem export/reimport acceptance confirms the original workspace stays unchanged. EDT-12 remains Partial; per-paragraph/ruler work is still open. Inventory stays **I 190, P 7, D 0, N 13, U 0; total 210**.

### 2026-10-24 — EDT-12 rich paragraphs and ruler completed

EDT-12 moves from Partial to Implemented. Per-paragraph metrics, mixed and nested lists, decimal tabs, wrapping columns, range authoring and the pointer/keyboard ruler are connected through strict persistence and both renderers. Native plain-edit, split and merge mutations preserve paragraph patches and runs across reload. The authentic ProPresenter 21.4.2 fixture passes import/reload/native render/export; an isolated Orbit rehearsal verifies selected-range ruler edits and reopening. Orbit-only justified tabs have an explicit left-aligned export boundary. Inventory is **I 191, P 6, D 0, N 13, U 0; total 210**.

### 2026-10-24 — CTL-06 network baseline completed

CTL-06 moves from Partial to Implemented for the operator's network-focused scope. AMP now connects all nine documented ProPresenter-style operations to the native video deck, including the remaining session creation/closure, one-folder workflow, loaded-ID/source-start and transport-status queries. Thirty-one codec tests and both phases of the real TCP/native-video restart smoke passed, including a queued command blocked after session closure. Raw TCP/UDP, RossTalk, GlobalCache and BVW/VDCP retain their existing implementations; retired switchers are not reintroduced. Physical-controller/macOS qualification and full AMP-server conformance are explicitly unclaimed. Inventory is **I 192, P 5, D 0, N 13, U 0; total 210**.

### 2026-10-24 — Merge of the parallel sessions

The EDT-12, CTL-06 and paragraph-export work of one session was merged with the INT-16 and CTL-12 work of the other. The CTL-06 AMP media catalog of the first session replaces the smaller In Preset clip loading of the second, which was reverted. Layered text layers and plain text slides share one text encoder, so both carry the exported paragraph formatting. The inventory is **I 192, P 7, D 0, N 11, U 0; total 210**.

### 2026-10-28 — INT-16 layered slide export completed

INT-16 moves from Partial to Implemented. Presentations now export their groups, arrangements, selected arrangement, cue completion and trigger settings, notes and ordered cue actions, including imported media actions. Slide media backgrounds export as cue media actions with copied media. Theme, prop and mask slides export their own layers, and a disabled text stroke keeps its color. An authentic ProPresenter 21.4.2 media bin capture establishes that `thumbnail_position` stores seconds, with -1 when unset; custom video thumbnail times now import and export. On the operator's real workspace the export review drops from about 25,000 findings to about 90 ProPresenter-inexpressible or invalid-source findings. The source-only gradient and timer-format variants move to the new INT-21 row. Exported files were not opened in ProPresenter. The inventory is **I 194, P 5, D 0, N 12, U 0; total 211**.

### 2026-10-30 — Remote checkpoint and linked timer authoring

Remote work is committed as `0d21d8d3`. The operator confirmed phone discovery,
manual connection, browsing, slide images/video, near-immediate thumbnails and
faster cue feedback. Prop collections load after choosing a nonempty collection.
The remaining phone checklist is in `docs/remote-api.md`; CTL-12 remains Partial.

As the first local step toward INT-21, the text inspector now exposes the existing
linked countdown unit formats: hours, minutes, seconds and tenths/hundredths.
Operators can choose widths, omit units and suppress zero-valued whole units.
Edits preserve other imported format flags and use the existing text-style
persistence, browser/native execution and export paths. Fractional zero-removal
choices remain disabled, with imported values still visible. Shared Select now
supports defaultValue and native change bindings without adding specialist
controls.

A local ProPresenter probe showed that setting the preserved
`millisecondsUnderMinuteOnly` protobuf flag alone did not hide fractions above
one minute. Its fractional unit menu also left the two struck-through choices
unselectable. These observations are insufficient to establish conditional
runtime semantics. Orbit's existing explicit import/export findings and fallback
remain unchanged. No conditional cutoff was inferred from the field name.
INT-21 therefore remains Not found, with nonlinear/advanced gradients,
conditional fractions and wall-clock formatting still open; inventory counts are
unchanged. Native editor acceptance of the new controls remains pending because
the Windows desktop was locked during the final manual check. Shared Select was
checked in the browser showcase in dark/comfortable and light/compact layouts.

Validation: `npm run check` (including typecheck, renderer build and Cargo check),
`npm run maintenance:check` and all 2,206 renderer tests passed with the existing
coverage thresholds. The initial sandbox temp-cache rename error was resolved by
using a project-local temp directory. No renderer-size-budget pass is claimed;
the previously recorded automation chunk overage remains outside this change.


### 2026-10-30 — Uniform multi-color shape gradients

INT-16 follow-up: shape imports already retained every gradient color and exported
uniform stop positions, but CSS, feathered SVG and native rendering used only the
first two colors. All three paths now use the complete color array. Native CSS
backgrounds share the corrected sampler, including nested rgb()/rgba() arguments.
Editing an endpoint keeps intermediate colors, and the endpoint picker displays
the last color rather than the second. The existing workspace model and storage
format are unchanged.

Verification passed: `npm run check` (including typecheck/build/Cargo check),
`npm run maintenance:check`, the full 2,209-test renderer suite with its coverage
thresholds, an additional eight-test component run including the endpoint picker,
94 native compositor tests (six existing manual/media tests ignored), and the
layered export/import test using four gradient colors. Four storage/cache tests
initially encountered sandbox access errors; the compositor suite passed with
normal filesystem access. Physical outputs and a source-application visual
comparison were not exercised. Native desktop UI acceptance remains pending
from the locked-desktop session.

This closes the silently dropped colors within the supported uniform linear
shape-gradient path. Arbitrary stop positions, nonlinear gradients, length and
noncentral blend points remain explicit import losses under INT-21; no inventory
status or count changes.


### 2026-10-30 — Closed vector-path gradient previews and direction

INT-16 follow-up: closed vector and Bezier paths now reference the shared SVG
shape gradient instead of reducing a gradient fill to its first color. The path
keeps its curves, stroke and optional feather filter; open paths remain unfilled.
Gradient paint IDs are unique across simultaneous previews. No workspace or
export representation changes were needed.

The shared SVG definition now uses CSS angle conventions and explicit object
bounds. It also fixes a quarter-turn mismatch in feathered shape previews.
Native shape/background gradients now project through the physical width and
height, matching CSS diagonal geometry instead of stretching the angle in a
non-square box. The two-color midpoint is tested as an exact equal mix after
channel rounding.

An isolated Vite fixture using the actual components was visually checked next
to a CSS reference at 45 degrees in a 400 by 200 box, with an additional
three-color Bezier path and a feathered ellipse. Automated checks cover cardinal
and diagonal SVG coordinates, open/closed paths, curves, feathering, unique
paint references and native gradient pixels. This does not qualify physical
outputs or source-application visual parity, and INT-21's unsupported gradient
and timer variants remain open. Inventory counts are unchanged.

Validation passed: `npm run check`, `npm run maintenance:check`, all 2,212
renderer tests with the existing coverage thresholds, and 95 native compositor
tests (six existing manual/media tests ignored).


### 2026-10-30 — INT-21 positioned linear shape gradients

INT-21 moves to Partial. The optional `fillPositions` array pairs ordered
normalized coordinates with `fillColors`; its absence retains the existing
uniform layout. Nonuniform source positions are preserved, including repeated
coordinates for hard edges and inset first/last stops. Native and browser
rendering clamp outside the endpoint stops. Export writes the exact coordinates
back to ProPresenter, while malformed source positions remain explicit import
losses.

Workspace read/write validation rejects null, wrong-length, nonnumeric,
out-of-range and descending coordinates. Endpoint edits retain positions;
replacing or disabling the gradient and projecting solid data fills remove
stale coordinates. A filesystem test exercises save/reload, rejected-write
atomicity and rejection of invalid persisted data. No migration or compatibility
alias was introduced.

The actual CSS, path, Bezier and feathered SVG components were compared visually
in an isolated browser fixture with a sharp transition at 40%. This is browser
acceptance against CSS, not source-application or physical-output qualification.
Radial/angle, length, blend-point and timer-format variants remain open. Inventory
is **I 194, P 6, D 0, N 11, U 0; total 211**.

Validation passed: `npm run check`, `npm run maintenance:check`, all 2,215
renderer tests with the existing coverage thresholds, and the complete Rust
suite (1,550 passed, 52 existing manual/hardware-dependent tests ignored). The
layered presentation return test includes nonuniform and repeated positions.

### 2026-11-05 — Import/export, text, notes and stage message fidelity

Stage message objects now import, author, export and render their box flash and
flash color. Slide notes keep runs and paragraph styles through RTF import/export,
editing and stage output (`preserveNotesFormat`). Text gains letter spacing,
superscript levels, Small Caps export, natural alignment, margins, underline and
strikethrough variants, tab leaders and bar tabs. Feathering gains center and
outside styles. Linked text gains named slide elements, line transforms and
playlist arrangement names. All six ProPresenter visibility condition kinds now
round-trip and evaluate natively. Audio assets keep loop-count and loop-time
playback.

Known limits: superscript levels beyond one, unknown transition or effect names
and unknown actions are still reported on import; a video input visibility
condition imports by index and must be linked to an Orbit input in the inspector;
RTF cannot hold combined underline settings or strikethrough style, pattern or
color on individual runs, which export reports. No comparison against ProPresenter
itself or physical output was made in this pass.

Validation: `npm run maintenance:check`, `npm run check`, 2,352 renderer tests with
the existing coverage thresholds, the renderer budget, and the complete Rust suite.

### 2026-11-05 — Sync and bundles accept layers that carry their own media

Sample theme layers hold ProPresenter's media id next to their own source path.
Workspace sync and portable bundles treated that id as a media bin reference, so
syncing themes from a new workspace and bundling a presentation on such a theme
both failed. A layer with its own source now travels as an asset; a bare media id
still requires the media bin. Importing a bundle reuses an installed theme when its
content and asset bytes match, whatever the workspace paths are.
