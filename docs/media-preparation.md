# Media preparation

The active graphics library is prepared in the background. Hovered or selected media and the following playlist item have priority; preparation continues through the remaining library in playlist order. Requests contain absolute source paths, media kinds and video in points. New priorities supersede unfinished work. The UI and program output process maintain separate shared caches for their renderers. Reconnecting the output process resubmits the active library.

Each cache retains at most 128 entries and 256 MiB of decoded pixels. Each decoded asset and each encoded still-image input are limited to 128 MiB; a request accepts at most 4096 items. Assets outside the cache budget remain available through normal playback. Recently used entries are retained when space permits, while active-library priority determines eviction under pressure. Preparation runs on one worker outside renderer and invoke locks and writes no files.

Images include their native animation frames. Video preparation decodes only the full initial frame at the configured in point. It does not cache entire video files, multiple seconds of decoded video or an advancing player. On activation, the renderer can display that frame while the regular FFmpeg decoder starts. Subsequent frames use the normal small playback buffer. This removes the first decode from the visible startup path but does not guarantee a fixed click-to-output latency for every codec or device.

Playback identifies a new selection separately from metadata clock revisions, seeks and shuttle changes. An intentionally delayed selection retains its preparation until playback starts. Retaking a video restarts its decoder even when the clock revision matches the previous selection. Source modification time and length are checked before using cached pixels; changed files cannot reuse stale prepared data.

`prepareMedia` validates requests and queues work. `getMediaPreparationStatus` reports readiness, errors, budget usage and cache hits for diagnostics and the complete media E2E workflow. Both calls are read-only with respect to workspace storage and are allowed in the output process.

Metadata probing shares a process-local LRU of 128 file-stamped results across preparation, inspectors and audio playback. A prepared video therefore does not launch ffprobe again when audio establishes its duration. File size or modification changes invalidate the entry, including cached probe errors. `getMediaPreparationStatus` includes `metadataLoads` and `metadataHits` to verify reuse. No cache mutex is held while probing.

Forward video decoder process creation runs on its existing decode worker. The render path can present a prepared first frame while FFmpeg starts; cancellation also covers decoders whose process has not started yet. Process cleanup does not hold the render path waiting for a child to exit.

Prepared still images include progressively reduced copies for plain fit rendering. Resizing a large decoded original is therefore kept off the click-to-preview path. These copies share the existing per-asset and process memory budgets; animated images and source-resolution effects retain the original rendering path. File changes invalidate the original and all reduced copies together.

Scene synchronization shares its accepted snapshot between the program connection and local preview protocol instead of deep-copying the complete theme catalog twice per take. Linked-file overlays skip traversal when there are no file entries. Scene ordering and native-state recovery remain unchanged.

A viewport covering the complete logical canvas renders at the consumer's requested dimensions when the aspect ratio matches (allowing one pixel of size rounding). Different aspect ratios retain the original canvas fit and transparent letterboxing. It does not rasterize the full output resolution and then crop and resize the same whole image for a small preview. Partial viewports retain source-canvas rendering so grouped outputs and edge blends preserve their crop coordinates; projector correction still runs after rendering.

Output protocols retain the immutable authoring theme source separately from live scene fields. Unchanged catalogs use shared identity when calculating scene patches, and are materialized for the UI only after the local output scene has committed. Theme edits and linked-file updates still replace their source; native-state recovery retains the full snapshot contract.

Each compositor also retains up to eight scaled static background images, bounded to 64 MiB including retained source pixels. Returning to a recent still reuses its scaled pixels. Dimensions, source-image identity, scale and effects all participate in the lookup; changed files receive a new source image. Video and animated effects bypass this cache.
