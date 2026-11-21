# Presentation export (INT-20)

File > Export presentation opens an independent, lazily loaded export dialog. It exports the canonical saved presentation in the active workspace through the typed native bridge. Export never changes the main or announcements selection, runs slide actions, or uses a live output compositor/player.

## Content and ordering

- UTF-8 text contains presentation title, song/CCLI metadata, and each selected slide's text. Operator notes and plain/rich slide notes are optional.
- Master order uses the saved slide sequence and ignores the presentation's selected arrangement. Choosing an arrangement uses its section order, including repeated occurrences; missing arrangements and missing section references are errors.
- Disabled slides are excluded unless the operator explicitly includes them. This policy applies after arrangement expansion.
- Numbered PNG files (`slide-0001.png`, etc.) preserve exported occurrence order inside a new dedicated folder.
- PDF contains one lossless RGB raster page per exported slide in the same order. Pages preserve canvas aspect ratio; rasterized PDF text is not selectable. These are slide canvases, not notes handouts.
- Visual export uses `slide_composition::compose_slide`, canonical native library theme assignment/output scene building, and a private `NativeCompositor::new_preview` instance. Theme objects, overrides, local objects, stacking, linked CCLI/presentation/slide text, and authored build steps are included. Every authored build step is visible in the exported canvas. Saved media background foreground/background and scale behavior use the canonical output scene builder.

## Explicit boundaries

Visual preflight runs before opening the destination dialog and again after destination selection. Missing themes/assets/fonts, corrupt image/SVG assets, absent required image sources, unsupported object types, CSS image backgrounds, remote assets, video/audio, GIF/animated WebP, NDI/web/live inputs, scrolling text, and live/external linked-text sources block visual export with an error. Supported linked sources are None, CCLI, Current/Next Slide, Slide/Next Slide Notes, Slide/Next Slide Label, Presentation Name, and Operator Notes. Text export does not require visual assets.

The export operation supports at most 500 slide occurrences. Native canvas dimensions may not exceed 4096 pixels per axis; visual exports are limited to 64 million total pixels to bound buffered encoding memory. Notes are text-only. Export is independent of audience look overrides and playlist-specific theme assignment: it exports the presentation with its saved slide/library theme assignment.

A canceled native destination dialog writes nothing. Workspace/revision changes after preflight are rejected. Existing files and directories are never overwritten; choose a new file for text/PDF. Encoding completes before destination creation. File write failures remove the newly created incomplete file; PNG failures remove files created earlier in that export and the new folder. ProPresenter roundtrip is outside this export path.

## Verification evidence

`cargo test -p orbit-host presentation_export::tests -- --test-threads=1` verifies order/repeated arrangements, disabled selection, notes, CCLI, native library theme inheritance, missing sections/sources/assets/fonts, corrupt images, unsupported dynamic sources, existing-file protection, write failures, real native PNG pixels, PDF page order and selected static media foreground/scale pixels.

The native fixture creates two genuine 320 x 180 PNGs and a PDF. Poppler `pdfinfo` confirmed PDF 1.4, two pages, correct page dimensions, no suspects and no encryption. `pypdf.PdfReader(..., strict=True)` reopened both pages and extracted one RGB image per page; both extracted images exactly match their native PNG pixels. Both pages were rendered with Poppler and visually inspected: the blue theme background, orange theme object, centered white verse text and occurrence order are correct. Fixture outputs are evidence artifacts outside the source tree, not generated files committed to the repository.

Desktop end-to-end destination-dialog testing still depends on a working native UI automation/runtime session. Automated native output evidence and React interaction tests do not establish manual desktop workflow verification.

## Final combined verification — 2026-10-08

The complete integrated npm run verify passed with 1401 renderer tests, 486 native tests (9 ignored), 27 Node tests and unchanged coverage/build gates. The actual lazy export menu/dialog workflow is covered by additional shell tests.
