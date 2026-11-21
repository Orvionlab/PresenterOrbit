# Video output layout analysis

## Existing path

A workspace stores logical screen configuration in `Configuration/Settings.json`. Looks and stage assignments reference those screen IDs. Output scenes already support a full source canvas, cropped followers, projector homographies, gain, black level, and software edge weights. Display and NDI consumers use the same host compositor. The existing screen settings expose these as separate screen records and long expanded forms.

Before this change, ProPresenter workspace decoding retained `arrangement` and `outputScreens`, but only the first device supplied runtime settings. The Workspace exporter wrote masks without output configuration, losing grouped-screen settings when moving through ProPresenter.

## Implementation

Keep logical screen IDs stable. An optional strict `outputLayout` contains device targets, arrangement, uniform canvas tile dimensions, per-device calibration, alpha targets, and per-seam blend parameters. Expand it into temporary physical followers at the host output boundary; do not store those generated records as independent user screens. Reuse canvas viewport rendering for grouped and overlapping outputs. Looks, stage layouts and presentation selection remain on the logical source.

The settings view shows a screen sidebar, a selectable device/overlap diagram, and compact Hardware, Color, Corner Pin and Alpha Key tabs. Selecting an overlap shows blend controls for its two edges. Edits appear through the existing optimistic settings lane before persistence finishes. Placeholders remain explicitly unconfigured, including on import; another computer's display ID must never select an arbitrary local monitor.

Hardware settings show output and target side by side. Selecting a physical monitor copies its detected pixel dimensions; those dimensions are read-only in the inspector. NDI dimensions and frame rate remain editable. SDR/HDR signal format is separate from pixel dimensions. Corner controls follow a 2×2 arrangement, and both sides of an edge blend have adjacent control columns.

Supported arrangements are Single, Mirror, Grouped and horizontal Edge Blend, with up to 16 devices and uniform logical tile dimensions. The existing canvas budget remains 8192 pixels per dimension and 33,554,432 pixels total. Each device has a display or NDI target, size, frame rate, format, delay, color adjustment, and optional corner pin. Audience outputs also support separate SDR fill/key destinations. Stage outputs retain their stage layout assignment. Existing independent canvas/key/projector routing stays in its original advanced controls and cannot be combined with a device layout.

Edge width is measured in logical pixels and reduces the total canvas width. Each overlap is positive and smaller than half a tile. Independent edge curves use linear, quadratic or smooth cubic weights, scaled by radius, gamma, intensity and black level. These are Orbit's software curves; preserving ProPresenter's numeric settings does not establish identical projector appearance. Neutral devices skip projector correction entirely.

Program preview sizing uses the total logical canvas. Completed static previews do not request another bitmap merely because a slow render exceeded the visibility interval; source and size changes still request new frames, and dynamic media keeps updating.

Screen mutation acknowledgements explicitly adopt their latest canonical configuration into the active scene. They do not depend on the local host patch revision matching the output child's independently advancing scene revision.

Pending screen updates and group switches also overlay the scene sent to the output child immediately. They do not wait for the workspace write lane. The last pending intent wins over earlier acknowledgements; a rejected save removes its draft and restores the canonical configuration. Group commands read that same pending state so rapid toggles alternate correctly. Their live indicator still follows the acknowledged scene.

Activation intents are planned against the current runtime configuration, including when saved activation differs from the launch state. The launch policy itself updates runtime state before the first scene submission and does not queue a workspace operation.

## ProPresenter reference

Pinned schema revision: `1b63dda196eb7e079721a8a4a7e7773520cb5ad2` (screens.proto, proscreen.proto, proworkspace.proto). `ProPresenterScreen` uses oneof fields 3/4/5 for Single, Combined and EdgeBlend. Combined stores rows/columns; edge blend stores repeated devices and seams. Each device stores bounds, display, corner values, color adjustment and alpha settings. Seam records reference devices by UUID and carry independent curve/radius/intensity/gamma/black-level values.

Reference schemas are unofficial reverse-engineered material, not a guarantee of optical matching. Preserve supported numeric settings through export/import and document any unsupported fields explicitly. Renewed Vision describes grouped output and edge blend at https://support.renewedvision.com/hc/en-us/articles/360041879173-Screen-Configuration-in-ProPresenter and https://support.renewedvision.com/hc/en-us/articles/360041820053-Edge-Blending-in-ProPresenter .

Export writes logical screens, device arrangements, calibration, alpha destinations and supported audience Look layer flags alongside masks in `Configuration/Workspace`. Import/export preserves the supported layout values. ProPresenter hardware identities remain stored identities, not a guarantee that the destination exists on another computer. Card/Syphon/custom targets remain unassigned in Orbit. Nonuniform or irregular device geometry, rotation, partial hardware targets, device colors, legacy gamma/black fields and blend compensation are rejected explicitly rather than silently approximated. The pinned format cannot represent Orbit HDR output formats; exporting those screens reports an error. This is not a complete ProPresenter workspace settings exporter: presentation theme overrides, Look transitions and stage assignments are not covered by this screen-layout mapping.

## Verification

The GPUI workflow uses temporary workspaces, real host mutations and a real inert output child. It checks immediate edits while storage is held, grouped and mirrored viewports, overlap edits, color, corner pin, fill/key scenes, malformed layout rejection, persistence after reopening and ProPresenter export/import. Native BGRA pixel assertions check attenuation on both overlapping edges. Physical projector alignment, HDR appearance, device timing, and NDI runtime availability remain hardware acceptance checks.

## Destination chooser

System destinations show their monitor number, friendly name, resolution, and physical desktop arrangement. New NDI and named placeholder destinations use the shared video format presets or a custom-size dialog. Dimensions and frame rate are validated before the optimistic screen edit is submitted. Physical display dimensions remain determined by hardware selection; named placeholders reserve a canvas without enabling a physical destination.
