//! The Settings window content of Phase 14: the panel navigation of `SettingsDialog.tsx` and one module per
//! panel. Every edit sends the same semantic host call as the React panel. Settings are written only here, in
//! the UI process; program output follows through the scene (`output::scene`) and never reads them.

use gpui::Global;

use crate::state::runtime::Runtime;

mod appearance;
mod audience_looks;
pub(crate) use audience_looks::complete as complete_audience_look;
mod audio;
mod audio_inputs;
mod audio_inputs_view;
pub mod audio_logic;
pub mod audio_routing;
mod capture;
mod clear_groups;
mod comm_input_view;
mod composition;
pub(crate) mod devices;
pub(crate) mod drafts;
mod general;
mod groups;
mod input_tab;
mod inputs;
mod mcp;
mod media_settings;
mod membership;
mod network_link;
mod patterns;
pub mod persist;
mod pro_presenter_import;
mod protocols;
mod remote;
pub(crate) mod rows;
mod screen_options;
mod services;
mod shortcuts;
mod slide_labels;
pub mod sync;
mod timecode;
mod triggers;
mod ui;
pub(crate) use ui::path_for_display;
pub(crate) mod video;
pub(crate) mod video_extra;
mod video_layout;
mod video_preview;
mod video_targets;
mod view;

pub(crate) use slide_labels::slide_labels;
pub use view::SettingsView;

/// The state runtime of the main window, which the auxiliary windows share so that a saved setting reaches the
/// scene and every other window without a second source of truth.
#[derive(Clone)]
pub struct SharedRuntime(pub Runtime);

impl Global for SharedRuntime {}

/// `settingsPanels` of `SettingsDialog.tsx`, in its order: the ids a window request may name.
pub const PANELS: [(&str, &str, &str); 15] = [
    ("general", "settings", "settings.nav.general"),
    ("appearance", "settings", "settings.nav.appearance"),
    ("import", "folder", "settings.nav.import"),
    ("sync", "folder", "settings.nav.sync"),
    ("groups", "arrows-exchange", "settings.nav.groups"),
    ("slideLabels", "clipboard-text", "settings.nav.slideLabels"),
    ("keys", "arrows-move-vertical", "settings.nav.keys"),
    ("video", "video", "settings.nav.video"),
    ("audio", "music", "settings.nav.audio"),
    ("inputs", "music", "settings.nav.inputs"),
    ("devices", "settings", "settings.nav.devices"),
    ("media", "video", "settings.nav.media"),
    ("remote", "settings", "settings.nav.remote"),
    ("mcp", "settings", "settings.remote.mcp.title"),
    ("updates", "settings", "settings.nav.updates"),
];

/// The panel request that opens Video on its Looks section.
pub const LOOKS_PANEL: &str = "video/looks";

/// `getInitialPanel`: an unknown or missing panel opens General.
pub fn initial_panel(requested: &str) -> &'static str {
    PANELS
        .iter()
        .find(|(id, _, _)| *id == requested)
        .map_or(PANELS[0].0, |(id, _, _)| *id)
}
