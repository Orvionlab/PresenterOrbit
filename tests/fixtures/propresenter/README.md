# ProPresenter import semantic corpus

The six generated inputs below are **synthetic fixtures**, authored against the
vendored `Proto7.16.2` schemas. The additional authentic fixtures are described
separately. The synthetic inputs do not establish real-version compatibility.

Regenerate the checked-in binary inputs and expected manifest from the repository root:

```sh
node tests/fixtures/propresenter/generate.mjs
```

The generator writes protobuf wire fields independently of Orbit's production
encoder. Expected JSON pointers are authored source semantics, not decoder
snapshots. Tests read the checked-in bytes, so running tests does not regenerate
or overwrite inputs or expectations. The corpus does not access external files
at the synthetic media URLs.

| Input | Semantic coverage |
| --- | --- |
| `layered.pro` | Two separately positioned text objects, primary lyrics, secondary copyright, RTF newline/Unicode, font family/size, cue/source IDs, operator notes. |
| `empty-groups-arrangements.pro` | Cue without a slide action, empty primary text with nonempty copyright, disabled cue, group labels and IDs, selected arrangement, repeated group references in source order. |
| `media.pro` | Image/video layers, URL references, loop transport, foreground video action. |
| `Theme` | Named theme slides, two text regions, dimensions and a blank slide. |
| `unknown-version.pro` | Synthetic application-version 21 header plus unknown field 19000, retaining the same known fields as `layered.pro`. Demonstrates unknown-field tolerance only. It does not model actual ProPresenter 21 output. |
| `truncated.pro` | Incomplete varint, rejected at the decoder boundary. |

Run the focused native tests:

```sh
npm run test:e2e
```

The tests verify expected decoder semantics, preservation through workspace
theme extraction and composition, and actual portable export/reimport losses.
Layered export now preserves editable text and secondary character runs, supported shapes and local pen paths, media fills and their dependencies, schema-backed data links, and supported build-in order/groups. It also writes workspace media/audio bins. Advanced source-only settings remain explicit import/export findings; these checks do not authorize a blanket lossless-export claim. Current INT-16 coverage and remaining limitations are documented in `docs/propresenter-parity.md`.


`object-fill.stage` and `object-text.stage` are authentic, locally authored ProPresenter Stage.Document fixtures from the owned "Orbit EDT-20 Probe" layout (2026-10-13). They contain only test shapes/text and exact UUID links. The fill fixture compares a rectangular destination with an ellipse source; the text fixture additionally contains an object text link. Their native tests verify UUID identity, destination geometry, dynamic paint and explicit malformed-link rejection.

`slide-images.stage` contains the two authentic image objects from the locally authored "Orbit DAT-12 Probe" stage layout (2026-10-14), based on ProPresenter's Current + Next Previews preset. Other preset objects and their timer references are excluded. The current-source link has an empty proto message; the next-source link has selector 1. Native tests verify exact source selection, independent current/next pixels, source-change invalidation and clearing an absent next slide.

`output-screen.stage` contains only the authentic Screen Preview ellipse from the owned "Orbit DAT-13 Output Probe" layout (2026-10-16). Its OutputScreen UUID matches the local ProPresenter Workspace screen identity. Native tests decode the link, resolve the imported canonical output ID and verify live blue-to-green pixels. Other objects are excluded; no personal presentation content is included.

Known import limitations: blank theme `content` remains empty but its `text`
preview is currently synthesized as `Text`. The corpus covers portable semantic
fields, not every action, rich-text run, fill/effect, workspace configuration,
or source-file variant. The original synthetic cases do not compare rendered
output or validate playback. The authentic case below adds those checks for
its bounded object workflow; general cross-version coverage remains necessary
for INT-13. The schema comparison and the synthetic newer-release tests
(`crates/orbit-host/src/propresenter_proto/newer_versions_tests.rs`) cover that row.

`authentic-layered-21.pro` was saved by ProPresenter
21.4.2 (352584193) on 2026-10-15. It contains only the owned “Orbit INT-12 Probe”
text, a generated red/green/blue PNG and a four-second silent generated video.
The video overlaps the image, while a template text object and independent
Unicode footer retain separate geometry. The stored media paths use an example
profile name. Tests relocate those paths into temporary test sources; geometry,
style, flags, identities and unknown fields retain the authentic encoding.

The decoded object list preserves source order. Explicit `themeLayerOrder`
records back-to-front painting, and composition applies that order. The six
synthetic inputs and their existing expected semantic manifest are unchanged.

The authentic integration test copies and deduplicates media dependencies,
removes the sources, reloads the saved workspace and renders both text objects,
exact uncovered image pixels and advancing foreground video. ProPresenter and
Orbit desktop acceptance are summarized in the INT-12 row of
`docs/propresenter-parity.md`.

`authentic-web-fill-21.pro` contains a Web object inserted and saved with the
native Web tool and Shape inspector in ProPresenter 21.4.2 on 2026-10-19. The
presentation started as the owned INT-15 portable-text seed; the web object,
its identity, geometry and URL were authored by ProPresenter. The local URL
serves only generated test shapes. The JSON sidecar records provenance and
SHA256. The import test preserves the native web URL, geometry and UUID and
verifies that no filesystem media reference is fabricated for the web page.

`authentic-paragraphs-21.pro` is an unmodified ProPresenter 21.4.2 (352584193) presentation authored by the operator on 2026-10-24 in the owned `Orbit EDT-12 Paragraph Probe` test. Its first slide contains eight logical paragraphs: two plain text lines, an empty paragraph, two bulleted lines, another empty paragraph, and two bold lines. Two more slides are empty. The native RTF uses a real list table/override and per-paragraph list membership, spacing and indentation controls. The accompanying JSON records provenance and the original SHA-256. This fixture verifies paragraph indexing, blank-line preservation, character runs, and list membership from an actual saved application file; it does not certify every rich-text source variant.

`authentic-build-in.pro` is an unchanged, locally authored copy of the owned `Orbit EDT-12 Paragraph Probe 2` test presentation captured on 2026-10-26 after the operator added a Fade build-in. Its accompanying JSON records the SHA-256 and provenance. It confirms that `Slide.element_build_order` contains build UUIDs rather than graphics-element UUIDs; the importer derives the first on-click visibility step from that order. It does not establish build-out or delivery/reveal index semantics.

`authentic-build-reveal.pro` is the operator-authored continuation of the paragraph-test copy in ProPresenter 21.4.2: By Bullet delivery, five ChildBuild rows, Fade Build In and Build Out. Its JSON records capture provenance and SHA-256. This source establishes zero-origin paragraph reveal ordering only.

`authentic-build-underline.pro` is the next operator-authored snapshot of that copy, with one underlined word and By Underline delivery. Build In shows ordinary text, ChildBuild index 0 reveals the word, and Build Out follows. Its JSON records the unchanged-file hash and producer.

`authentic-scroller-*.pro` are seven unchanged ProPresenter 21.4.2 (352584193) captures from the owned INT-16 scroller calibration presentation, recorded on 2026-10-27. Their sidecar records the UI settings, protobuf values, provenance and SHA-256 hashes. The probes establish speed values 0, 25, 50 and 100; horizontal and vertical repeat-distance fractions against the actual text-object bounds; and vertical feather fractions against slide width. The focused native test verifies decoded pixel values and round-trip behavior.

`authentic-gradient-*.pro` are three unchanged ProPresenter 21.4.2 (352584193) captures from the owned INT-16 text-gradient calibration probe, recorded on 2026-10-28. Their sidecar records the UI angles, the Use object bounds checkbox, raw angle and stop-field presence, provenance, byte sizes and SHA-256 hashes. The two-stop UI gradient omits both stop positions and midpoint fields; those two stops represent the endpoints with the default midpoint. ProPresenter angle 90 maps to Orbit angle 0, preserving the observed red-bottom-to-cyan-top direction. Enabling Use object bounds sets the fill-level bounds flag; Orbit maps the flag to `textGradientUseObjectBounds` and emits `true` when the Orbit key is absent to preserve the current renderer default.

`authentic-audio-gain-*.playlist` are three ProPresenter 21.4.2 (352584193) PlaylistDocument captures from the owned INT-16 audio-channel gain probe, recorded on 2026-10-28. Each references the same generated silent WAV through a path with an example profile name. `Media.Audio.Channel.volume` field 3 stores direct dB values. An omitted field represents 0 dB. The separate `Media.AudioProperties.volume` master field stores linear gain.

`authentic-thumbnail-position-21.playlist` is a ProPresenter 21.4.2 `Playlists/Media` capture recorded on 2026-10-28 after setting a 40.9 s video's custom thumbnail at 23.4 s, reduced to that one video with the user name in its path neutralized. `Media.VideoProperties.thumbnail_position` stores 23.4, so the field is in seconds; ProPresenter stores -1 for an unset thumbnail. The sidecar records provenance, size and SHA-256.
