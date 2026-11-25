use gpui::{
    div, prelude::*, AnyElement, Context, IntoElement, Role, SharedString, Subscription, Window,
};
use orbit_ui::controls::{EmptyState, SegmentedControl, SegmentedOption};
use orbit_ui::{icon, ActiveTheme, Translator};
use serde_json::Value;

use crate::state::runtime::Runtime;

use super::video::VideoSection;
use super::{initial_panel, persist, PANELS};

/// The Settings window body: navigation on the left, the selected panel on the right. Panels are `impl` blocks
/// in their own files, so each one sits next to the React file it ports.
pub struct SettingsView {
    pub(super) runtime: Runtime,
    pub(super) panel: &'static str,
    /// The last failure of a save, shown above the panel until the next successful one.
    pub(super) status: Option<SharedString>,
    pub(super) general: super::general::General,
    pub(super) appearance: Option<super::appearance::Sliders>,
    pub(super) audio: super::audio::Audio,
    pub(super) remote: super::remote::Remote,
    pub(super) groups: super::groups::Groups,
    pub(super) shortcuts: super::shortcuts::Shortcuts,
    pub(super) sync: super::sync::SyncPanel,
    pub(super) services: super::services::Services,
    pub(super) video: super::video::Video,
    pub(super) triggers: super::triggers::Triggers,
    pub(super) looks: super::audience_looks::Looks,
    pub(super) audio_inputs: super::audio_inputs::AudioInputs,
    pub(super) timecode: super::timecode::Timecode,
    pub(super) devices: super::devices::Devices,
    pub(super) link: super::network_link::NetworkLink,
    pub(super) membership: super::membership::Membership,
    pub(super) clear_groups: super::clear_groups::ClearGroups,
    pub(super) composition: super::composition::Composition,
    pub(super) capture: super::capture::Capture,
    pub(super) patterns: super::patterns::Patterns,
    /// The Screens window shows the video panel alone, without the navigation.
    standalone: Option<&'static str>,
    /// The registered workspaces the General panel switches between. Absent in a view built without a shell.
    pub(super) workspaces: Option<gpui::Entity<crate::workspaces::Workspaces>>,
    pub(super) inputs: super::inputs::Inputs,
    /// The open subpage of the selected panel, as the panels' `useState` of an opened row.
    pub(super) page: Option<&'static str>,
    pub(super) media_settings: super::media_settings::MediaSettings,
    pub(super) slide_labels: super::slide_labels::SlideLabels,
    pub(super) screen_options: super::screen_options::ScreenOptions,
    pub(super) input_tab: super::input_tab::InputTab,
    _subscriptions: Vec<Subscription>,
}

impl SettingsView {
    pub fn new(runtime: Runtime, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = vec![
            cx.observe(&runtime.output, |_, _, cx| cx.notify()),
            cx.observe(&runtime.content, |_, _, cx| cx.notify()),
        ];
        let workspaces = cx
            .try_global::<crate::workspaces::SharedWorkspaces>()
            .map(|shared| shared.0.clone());
        if let Some(workspaces) = &workspaces {
            subscriptions.push(cx.observe(workspaces, |_, _, cx| cx.notify()));
        }
        Self {
            runtime,
            panel: PANELS[0].0,
            status: None,
            general: Default::default(),
            appearance: None,
            audio: Default::default(),
            remote: Default::default(),
            groups: Default::default(),
            shortcuts: Default::default(),
            sync: Default::default(),
            services: Default::default(),
            video: Default::default(),
            triggers: Default::default(),
            looks: Default::default(),
            audio_inputs: Default::default(),
            devices: Default::default(),
            timecode: Default::default(),
            link: Default::default(),
            membership: Default::default(),
            clear_groups: Default::default(),
            composition: Default::default(),
            capture: Default::default(),
            patterns: Default::default(),
            standalone: None,
            workspaces,
            inputs: Default::default(),
            page: None,
            media_settings: Default::default(),
            slide_labels: Default::default(),
            screen_options: Default::default(),
            input_tab: Default::default(),
            _subscriptions: subscriptions,
        }
    }

    /// The Screens window: the video panel with no navigation.
    pub fn screens(runtime: Runtime, cx: &mut Context<Self>) -> Self {
        let mut view = Self::new(runtime, cx);
        view.panel = "video";
        view.standalone = Some("video");
        view
    }

    pub fn select(&mut self, panel: &str, cx: &mut Context<Self>) {
        let looks = panel == super::LOOKS_PANEL;
        if looks {
            self.video.section = VideoSection::Looks;
            cx.notify();
        }
        let panel = initial_panel(if looks { "video" } else { panel });
        if self.panel != panel {
            self.panel = panel;
            self.page = None;
            self.status = None;
            cx.notify();
        }
    }

    pub(super) fn open_page(&mut self, page: &'static str, cx: &mut Context<Self>) {
        self.page = Some(page);
        self.status = None;
        cx.notify();
    }

    pub(super) fn close_page(&mut self, cx: &mut Context<Self>) {
        self.page = None;
        cx.notify();
    }

    /// The settings every panel reads: what the host last saved.
    pub(super) fn settings(&self, cx: &gpui::App) -> std::sync::Arc<Value> {
        std::sync::Arc::new(persist::shown(&self.runtime, cx))
    }

    /// Saves global settings and shows a failure above the panel.
    pub(super) fn save(&mut self, payload: Value, cx: &mut Context<Self>) {
        let task = persist::save(&self.runtime, cx, payload);
        self.report(task, cx);
    }

    pub(super) fn report(
        &mut self,
        task: gpui::Task<Result<Value, String>>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |view, cx| {
            let result = task.await;
            view.update(cx, |view, cx| {
                view.status = result.err().map(|error| {
                    let t = *cx.global::<Translator>();
                    t.format("status.workspace.saveFailed", &[("error", &error)])
                        .into()
                });
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// A panel failure that is not a save, such as a device list that could not load.
    pub(super) fn fail(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status = Some(message.into());
        cx.notify();
    }

    fn body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.global::<Translator>();
        if self.settings(cx).is_null() {
            return EmptyState::new(t.t("status.workspace.loading")).into_any_element();
        }
        match self.panel {
            "general" => self.general_panel(window, cx),
            "appearance" => self.appearance_panel(window, cx),
            "audio" => self.audio_panel(window, cx),
            "remote" => self.remote_panel(window, cx),
            "mcp" => self.mcp_panel(window, cx),
            "groups" => {
                let groups = self.groups_panel(window, cx);
                let clear = self.clear_groups_panel(window, cx);
                fill_then(groups, clear, window, cx)
            }
            "keys" => {
                let keys = self.keys_panel(window, cx);
                let triggers = self.triggers_section(window, cx);
                div()
                    .flex()
                    .flex_col()
                    .gap(cx.theme().metrics.panel_gap)
                    .child(keys)
                    .child(triggers)
                    .into_any_element()
            }
            "sync" => self.sync_panel(window, cx),
            "slideLabels" => self.slide_labels_panel(cx),
            "inputs" => self.inputs_panel(window, cx),
            "media" => self.media_panel(cx),
            "video" => {
                let padding = cx.theme().metrics.panel_padding;
                let section = self.video.section;
                let weak = cx.weak_entity();
                let mut options = vec![SegmentedOption::text(
                    "outputs",
                    t.t("settings.video.outputs"),
                )];
                if self.standalone.is_none() {
                    options.push(SegmentedOption::text(
                        "options",
                        t.t("settings.screenOptions.title"),
                    ));
                }
                options.push(SegmentedOption::text("looks", t.t("audienceLooks.title")));
                let selected = match section {
                    VideoSection::Outputs => "outputs",
                    VideoSection::ScreenOptions => "options",
                    VideoSection::Looks => "looks",
                };
                let sections = SegmentedControl::new(
                    "video-sections",
                    t.t("settings.video.sections"),
                    options,
                    selected,
                )
                .on_change(move |value, _, cx| {
                    weak.update(cx, |view, cx| {
                        view.video.section = match value {
                            "options" => VideoSection::ScreenOptions,
                            "looks" => VideoSection::Looks,
                            _ => VideoSection::Outputs,
                        };
                        cx.notify();
                    })
                    .ok();
                });
                let panel = div()
                    .flex()
                    .flex_col()
                    .child(div().flex().p(padding).child(sections));
                match section {
                    VideoSection::Looks => {
                        let looks = self.looks_panel(window, cx);
                        panel
                            .child(div().p(padding).child(looks))
                            .into_any_element()
                    }
                    VideoSection::ScreenOptions => {
                        let options = self.screen_options_section(cx);
                        panel
                            .child(div().p(padding).child(options))
                            .into_any_element()
                    }
                    VideoSection::Outputs => {
                        let video = self.video_panel(window, cx);
                        panel.child(video).into_any_element()
                    }
                }
            }
            "devices" => self.devices_panel(window, cx),
            other => match self.service_panel(other, window, cx) {
                Some(panel) => panel,
                None => EmptyState::new(t.t("shell.region.unavailable")).into_any_element(),
            },
        }
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let t = *cx.global::<Translator>();
        let rows: Vec<AnyElement> = PANELS
            .iter()
            .map(|(id, glyph, label)| {
                let id: &'static str = id;
                let selected = self.panel == id;
                let (selected_bg, hover, accent_text) = (
                    theme.colors.selected,
                    theme.colors.accent_soft,
                    theme.colors.accent_text,
                );
                div()
                    .id(SharedString::from(format!("settings-nav-{id}")))
                    .relative()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(theme.metrics.control_gap)
                    .w_full()
                    .min_w_0()
                    .h(theme.metrics.control_height)
                    .px(theme.metrics.field_padding_x)
                    .rounded(theme.metrics.radius_sm)
                    .text_size(theme.metrics.font_sm)
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(theme.colors.text)
                    .cursor_pointer()
                    .when(selected, |this| {
                        this.bg(selected_bg).text_color(accent_text).child(
                            div()
                                .absolute()
                                .left_0()
                                .top(theme.metrics.space_xs)
                                .bottom(theme.metrics.space_xs)
                                .w(gpui::px(3.))
                                .rounded_r(theme.metrics.radius_xs)
                                .bg(accent_text),
                        )
                    })
                    .when(!selected, |this| this.hover(|style| style.bg(hover)))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(theme.metrics.space_lg * 1.5)
                            .child(icon(glyph).size(theme.metrics.font_sm).text_color(
                                if selected {
                                    accent_text
                                } else {
                                    theme.colors.text
                                },
                            )),
                    )
                    .child(div().min_w_0().truncate().child(t.t(label)))
                    .on_click(cx.listener(move |view, _, _, cx| view.select(id, cx)))
                    .into_any_element()
            })
            .collect();
        let nav = div()
            .id("settings-nav")
            .role(Role::TabList)
            .aria_label(t.t("settings.nav.sections"))
            .flex()
            .flex_col()
            .gap(gpui::px(1.))
            .children(rows);
        let status = self.status.clone();
        let body = self.body(window, cx);
        let overlays = self.output_target_overlays(cx);
        if self.standalone.is_some() {
            let capture = self.capture_panel(window, cx);
            let patterns = self.patterns_panel(window, cx);
            return div()
                .id("screens-content")
                .children(overlays)
                .size_full()
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .children(status.map(|status| {
                    div()
                        .id("settings-status")
                        .p(theme.metrics.panel_padding)
                        .text_color(theme.colors.warning)
                        .child(status)
                }))
                .child(content_column(body))
                .child(content_column(
                    div()
                        .flex()
                        .flex_col()
                        .p(theme.metrics.panel_padding)
                        .gap(theme.metrics.panel_gap)
                        .child(capture)
                        .child(patterns)
                        .into_any_element(),
                ))
                .into_any_element();
        }
        div()
            .size_full()
            .flex()
            .children(overlays)
            .child(
                div()
                    .id("settings-nav-column")
                    .flex_none()
                    .w(gpui::px(188.))
                    .h_full()
                    .overflow_y_scroll()
                    .p(theme.metrics.space_sm)
                    .border_r_1()
                    .border_color(theme.colors.border_strong)
                    .bg(theme.colors.panel_bg_strong)
                    .child(nav),
            )
            .child(
                div()
                    .id("settings-content")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .bg(theme.colors.surface_bg)
                    .when(self.panel != "video", |this| {
                        this.p(theme.metrics.panel_padding)
                    })
                    .flex()
                    .flex_col()
                    .gap(theme.metrics.panel_gap)
                    .children(status.map(|status| {
                        div()
                            .id("settings-status")
                            .text_color(theme.colors.warning)
                            .child(status)
                    }))
                    .child(content_column(body)),
            )
            .into_any_element()
    }
}

/// Every panel sits in one centred column, and it never shrinks below its content so sections stack and the window scrolls.
fn content_column(body: AnyElement) -> impl IntoElement {
    div()
        .flex_none()
        .w_full()
        .max_w(gpui::px(super::rows::CONTENT_MAX_WIDTH))
        .mx_auto()
        .child(body)
}

/// A panel that fills the window with its own scrolling list, as `groups-settings` does with `height: 100%`, and a second section below it that the window scroll reaches.
fn fill_then(first: AnyElement, second: AnyElement, window: &Window, cx: &gpui::App) -> AnyElement {
    let metrics = cx.theme().metrics;
    let header = metrics.control_height + metrics.space_md * 2. + gpui::px(1.);
    let height = window.viewport_size().height - header - metrics.panel_padding * 2.;
    div()
        .flex()
        .flex_col()
        .gap(metrics.panel_gap)
        .child(div().flex_none().h(height).child(first))
        .child(second)
        .into_any_element()
}
