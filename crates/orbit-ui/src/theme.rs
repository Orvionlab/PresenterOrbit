use gpui::{
    point, px, rgb, App, BoxShadow, Context, Global, Rgba, Subscription, Window, WindowAppearance,
};
use serde_json::Value;

use crate::color::{mix_oklch, mix_srgb};
use crate::metrics::{Metrics, TextScale, UiDensity};

/// Program output stays black even before its first frame, in every theme.
pub const PROGRAM_PREVIEW_BG: Rgba = Rgba {
    r: 0.,
    g: 0.,
    b: 0.,
    a: 1.,
};

/// Neutral gray behind transparent media, paired with black checker squares.
pub const MEDIA_TRANSPARENCY_BG: Rgba = Rgba {
    r: 0.22,
    g: 0.22,
    b: 0.22,
    a: 1.,
};

macro_rules! colors {
    ($($field:ident $css:literal),* $(,)?) => {
        /// Every `--app-*` color token of `tokens.css`, by its CSS name without the `--`.
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct Colors { $(pub $field: Rgba,)* }

        impl Colors {
            pub fn entries(&self) -> Vec<(&'static str, Rgba)> {
                vec![$(($css, self.$field),)*]
            }
        }
    };
}

colors! {
    bg "app-bg", shell_bg "app-shell-bg", panel_bg "app-panel-bg", panel_bg_strong "app-panel-bg-strong",
    panel_bg_soft "app-panel-bg-soft", surface_bg "app-surface-bg", surface_raised_bg "app-surface-raised-bg",
    field_bg "app-field-bg", popover_bg "app-popover-bg", stage_bg "app-stage-bg", canvas_bg "app-canvas-bg",
    preview_matte_bg "app-preview-matte-bg", canvas_transparent_bg "app-canvas-transparent-bg",
    slide_guide "app-slide-guide", object_guide "app-object-guide", text "app-text", text_strong "app-text-strong",
    text_muted "app-text-muted", text_subtle "app-text-subtle", border "app-border",
    border_strong "app-border-strong", border_soft "app-border-soft", accent "app-accent",
    accent_hover "app-accent-hover", accent_soft "app-accent-soft", accent_selected "app-accent-selected",
    live "app-live", selected "app-selected", accent_text "app-accent-text", warning "app-warning",
    danger "app-danger", success "app-success", focus_ring "app-focus-ring", cue_default "app-cue-default",
    cue_accent "app-cue-accent", control_bg "app-control-bg", control_bg_hover "app-control-bg-hover",
    control_border "app-control-border", overlay_bg "app-overlay-bg", menu_separator "app-menu-separator",
    menu_icon_bg "app-menu-icon-bg", disabled_text "app-disabled-text",
}

fn alpha(hex: u32, opacity: f32) -> Rgba {
    Rgba {
        a: opacity,
        ..rgb(hex)
    }
}

/// The `:root` block. The other themes are overrides of it.
fn dark() -> Colors {
    Colors {
        bg: rgb(0x1f1f1f),
        shell_bg: rgb(0x202020),
        panel_bg: rgb(0x2f2f2f),
        panel_bg_strong: rgb(0x252525),
        panel_bg_soft: rgb(0x3f3f3f),
        surface_bg: rgb(0x2d2d2d),
        surface_raised_bg: rgb(0x3f3f3f),
        field_bg: rgb(0x1f1f1f),
        popover_bg: rgb(0x707070),
        stage_bg: rgb(0x222222),
        canvas_bg: rgb(0x222222),
        preview_matte_bg: rgb(0x050607),
        canvas_transparent_bg: rgb(0x48484b),
        slide_guide: rgb(0xffff00),
        object_guide: rgb(0xff0000),
        text: rgb(0xf2f2f2),
        text_strong: rgb(0xffffff),
        text_muted: rgb(0xbdbdbd),
        text_subtle: rgb(0x8f8f8f),
        border: rgb(0x555555),
        border_strong: rgb(0x151515),
        border_soft: rgb(0x3f3f3f),
        accent: rgb(0x2f78bf),
        accent_hover: rgb(0x3e7fc3),
        accent_soft: rgb(0x575757),
        accent_selected: rgb(0x1d73c9),
        live: rgb(0xf7a600),
        selected: rgb(0x1d73c9),
        accent_text: rgb(0xffffff),
        warning: rgb(0xf7a600),
        danger: rgb(0xb43c3c),
        success: rgb(0x30d56f),
        focus_ring: rgb(0x5aa7ff),
        cue_default: rgb(0x087fc0),
        cue_accent: rgb(0xc40065),
        control_bg: rgb(0x242424),
        control_bg_hover: rgb(0x3b3b3b),
        control_border: rgb(0x555555),
        overlay_bg: alpha(0x000000, 0.56),
        menu_separator: alpha(0xffffff, 0.18),
        menu_icon_bg: alpha(0x000000, 0.14),
        disabled_text: alpha(0xffffff, 0.45),
    }
}

fn light() -> Colors {
    Colors {
        bg: rgb(0xf4f5f7),
        shell_bg: rgb(0xe8ebef),
        panel_bg: rgb(0xf8fafc),
        panel_bg_strong: rgb(0xeef2f6),
        panel_bg_soft: rgb(0xe7edf4),
        surface_bg: rgb(0xffffff),
        surface_raised_bg: rgb(0xffffff),
        field_bg: rgb(0xffffff),
        popover_bg: rgb(0xffffff),
        stage_bg: rgb(0xdfe5ec),
        preview_matte_bg: rgb(0xeef2f6),
        canvas_transparent_bg: rgb(0xaeb9c6),
        text: rgb(0x1f2933),
        text_strong: rgb(0x102033),
        text_muted: rgb(0x607085),
        text_subtle: rgb(0x7b8794),
        border: rgb(0xd6dde6),
        border_strong: rgb(0xc7d0dc),
        border_soft: rgb(0xdce4ee),
        accent: rgb(0x155fa8),
        accent_hover: rgb(0x0f4f8f),
        accent_soft: rgb(0xc6ddf5),
        accent_selected: rgb(0x155fa8),
        warning: rgb(0xc47800),
        success: rgb(0x17884c),
        focus_ring: rgb(0x155fa8),
        control_bg: rgb(0xffffff),
        control_bg_hover: rgb(0xe7edf4),
        control_border: rgb(0xc8d2df),
        overlay_bg: alpha(0x1c2734, 0.34),
        menu_separator: alpha(0x102033, 0.16),
        menu_icon_bg: alpha(0x102033, 0.07),
        disabled_text: alpha(0x1f2933, 0.48),
        ..dark()
    }
}

fn red() -> Colors {
    Colors {
        bg: rgb(0x211415),
        shell_bg: rgb(0x241617),
        panel_bg: rgb(0x312020),
        panel_bg_strong: rgb(0x271819),
        panel_bg_soft: rgb(0x4a2c2d),
        surface_bg: rgb(0x362123),
        surface_raised_bg: rgb(0x442729),
        field_bg: rgb(0x1d1112),
        popover_bg: rgb(0x5b3235),
        stage_bg: rgb(0x1f1415),
        preview_matte_bg: rgb(0x251819),
        canvas_transparent_bg: rgb(0x60484b),
        text: rgb(0xfff1f1),
        text_muted: rgb(0xe2bfc0),
        text_subtle: rgb(0xb9898b),
        border: rgb(0x704548),
        border_strong: rgb(0x190c0d),
        border_soft: rgb(0x5d383b),
        control_bg: rgb(0x362123),
        control_bg_hover: rgb(0x4a2c2d),
        control_border: rgb(0x704548),
        overlay_bg: alpha(0x0f0506, 0.62),
        menu_separator: alpha(0xfff1f1, 0.18),
        menu_icon_bg: alpha(0x0f0506, 0.22),
        ..dark()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ThemeSetting {
    System,
    Dark,
    Light,
    Red,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AccentSetting {
    Blue,
    Red,
    Purple,
    Green,
}

/// The resolved color scheme, the CSS `color-scheme`. The red theme is dark.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Appearance {
    Dark,
    Light,
}

/// `generalSettings.appTheme`, `appAccent`, `textScale`, and `uiDensity`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AppearanceSettings {
    pub theme: ThemeSetting,
    pub accent: AccentSetting,
    pub text_scale: TextScale,
    pub density: UiDensity,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: ThemeSetting::System,
            accent: AccentSetting::Blue,
            text_scale: TextScale::Default,
            density: UiDensity::Comfortable,
        }
    }
}

impl AppearanceSettings {
    /// The host normalizes `generalSettings`, so a missing or unknown value is a contract violation.
    pub fn from_settings(settings: &Value) -> Result<Self, String> {
        let general = &settings["generalSettings"];
        let read = |key: &str| {
            general[key]
                .as_str()
                .ok_or_else(|| format!("The settings have no valid generalSettings.{key}."))
        };
        let unknown = |key: &str, value: &str| format!("Unknown generalSettings.{key} '{value}'.");
        let (theme, accent, text_scale, density) = (
            read("appTheme")?,
            read("appAccent")?,
            read("textScale")?,
            read("uiDensity")?,
        );
        Ok(Self {
            theme: match theme {
                "system" => ThemeSetting::System,
                "dark" => ThemeSetting::Dark,
                "light" => ThemeSetting::Light,
                "red" => ThemeSetting::Red,
                other => return Err(unknown("appTheme", other)),
            },
            accent: match accent {
                "blue" => AccentSetting::Blue,
                "red" => AccentSetting::Red,
                "purple" => AccentSetting::Purple,
                "green" => AccentSetting::Green,
                other => return Err(unknown("appAccent", other)),
            },
            text_scale: TextScale::ALL
                .into_iter()
                .find(|candidate| candidate.id() == text_scale)
                .ok_or_else(|| unknown("textScale", text_scale))?,
            density: UiDensity::ALL
                .into_iter()
                .find(|candidate| candidate.id() == density)
                .ok_or_else(|| unknown("uiDensity", density))?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub colors: Colors,
    pub metrics: Metrics,
    pub appearance: Appearance,
    /// `--app-shadow`, for dialogs.
    pub shadow_color: Rgba,
    /// `--app-shadow-soft`, for popovers and menus.
    pub shadow_soft_color: Rgba,
}

impl Global for Theme {}

impl Theme {
    pub fn resolve(settings: &AppearanceSettings, system: WindowAppearance) -> Self {
        let resolved = match settings.theme {
            ThemeSetting::System => match system {
                WindowAppearance::Light | WindowAppearance::VibrantLight => ThemeSetting::Light,
                WindowAppearance::Dark | WindowAppearance::VibrantDark => ThemeSetting::Dark,
            },
            explicit => explicit,
        };
        let (shadow_color, shadow_soft_color) = match resolved {
            ThemeSetting::Light => (alpha(0x1c2734, 0.18), alpha(0x1c2734, 0.12)),
            ThemeSetting::Red => (alpha(0x0f0506, 0.58), alpha(0x0f0506, 0.34)),
            _ => (alpha(0x000000, 0.45), alpha(0x000000, 0.24)),
        };
        let (mut colors, appearance) = match resolved {
            ThemeSetting::Light => (light(), Appearance::Light),
            ThemeSetting::Red => (red(), Appearance::Dark),
            _ => (dark(), Appearance::Dark),
        };
        apply_accent(&mut colors, settings.accent, appearance);
        // The red theme's tuned accent belongs to the red accent alone; the others derive from the theme.
        if resolved == ThemeSetting::Red && settings.accent == AccentSetting::Red {
            colors.accent = rgb(0xd4413f);
            colors.accent_hover = rgb(0xf05a55);
            colors.accent_soft = rgb(0x67383a);
            colors.accent_selected = rgb(0xb83232);
            colors.focus_ring = rgb(0xf05a55);
        }
        colors.live = colors.warning;
        colors.selected = colors.accent_selected;
        colors.canvas_bg = colors.stage_bg;
        Self {
            colors,
            metrics: Metrics::new(settings.density, settings.text_scale),
            appearance,
            shadow_color,
            shadow_soft_color,
        }
    }

    /// The dialog elevation.
    pub fn shadow(&self) -> Vec<BoxShadow> {
        vec![drop_shadow(18., 50., self.shadow_color)]
    }

    /// The popover and menu elevation.
    pub fn shadow_soft(&self) -> Vec<BoxShadow> {
        vec![drop_shadow(10., 24., self.shadow_soft_color)]
    }
}

fn drop_shadow(y: f32, blur: f32, color: Rgba) -> BoxShadow {
    BoxShadow {
        color: color.into(),
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(0.),
        inset: false,
    }
}

/// `gpui-base` paints caret, selection, and placeholder from its own tokens.
fn sync_base_theme(cx: &mut App) {
    let colors = cx.global::<Theme>().colors;
    let tokens = &mut gpui_base::Theme::global_mut(cx).tokens.colors;
    tokens.background = colors.bg.into();
    tokens.foreground = colors.text.into();
    tokens.surface = colors.surface_bg.into();
    tokens.surface_foreground = colors.text.into();
    tokens.primary = colors.accent.into();
    tokens.primary_foreground = colors.accent_text.into();
    tokens.secondary = colors.control_bg.into();
    tokens.secondary_foreground = colors.text.into();
    tokens.muted = colors.panel_bg_soft.into();
    tokens.muted_foreground = colors.text_subtle.into();
    tokens.accent = colors.accent_soft.into();
    tokens.accent_foreground = colors.text_strong.into();
    tokens.destructive = colors.danger.into();
    tokens.destructive_foreground = colors.accent_text.into();
    tokens.border = colors.border.into();
    tokens.input = colors.control_border.into();
    tokens.ring = colors.focus_ring.into();
    tokens.selection = colors.accent_soft.into();
}

/// Blue is the theme's own accent. The others derive their four tokens from a source color, with different
/// percentages on light.
fn apply_accent(colors: &mut Colors, accent: AccentSetting, appearance: Appearance) {
    let source = match accent {
        AccentSetting::Blue => return,
        AccentSetting::Red => colors.danger,
        AccentSetting::Green => colors.success,
        AccentSetting::Purple => mix_oklch(colors.cue_default, 0.55, colors.cue_accent),
    };
    colors.accent = source;
    match appearance {
        Appearance::Dark => {
            colors.accent_hover = mix_srgb(source, 0.78, colors.text_strong);
            colors.accent_soft = mix_srgb(source, 0.45, colors.panel_bg);
            colors.accent_selected = mix_srgb(source, 0.88, colors.border_strong);
            colors.focus_ring = colors.accent_hover;
        }
        Appearance::Light => {
            colors.accent_hover = mix_srgb(source, 0.88, colors.text_strong);
            colors.accent_soft = mix_srgb(source, 0.24, colors.surface_bg);
            colors.accent_selected = source;
            colors.focus_ring = source;
        }
    }
}

pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    fn theme(&self) -> &Theme {
        self.global::<Theme>()
    }
}

/// What the theme is resolved from. Both change at runtime.
#[derive(Clone, Copy)]
struct ThemeInputs {
    settings: AppearanceSettings,
    system: WindowAppearance,
}

impl Global for ThemeInputs {}

pub fn init_theme(cx: &mut App, settings: AppearanceSettings, system: WindowAppearance) {
    gpui_base::init(cx);
    cx.set_global(ThemeInputs { settings, system });
    cx.set_global(Theme::resolve(&settings, system));
    sync_base_theme(cx);
}

fn update_theme(cx: &mut App, change: impl FnOnce(&mut ThemeInputs)) {
    let mut inputs = *cx.global::<ThemeInputs>();
    change(&mut inputs);
    cx.set_global(inputs);
    let theme = Theme::resolve(&inputs.settings, inputs.system);
    // Setting an equal global would still repaint every window that observes it.
    if *cx.global::<Theme>() != theme {
        cx.set_global(theme);
        sync_base_theme(cx);
    }
}

/// The settings the theme is currently resolved from.
pub fn appearance_settings(cx: &App) -> AppearanceSettings {
    cx.global::<ThemeInputs>().settings
}

/// Applies changed settings to every window at once.
pub fn set_appearance_settings(cx: &mut App, settings: AppearanceSettings) {
    update_theme(cx, |inputs| inputs.settings = settings);
}

/// Applies the operating system's light or dark appearance to every window that follows the system theme.
pub fn set_system_appearance(cx: &mut App, system: WindowAppearance) {
    update_theme(cx, |inputs| inputs.system = system);
}

/// Makes `view` repaint when the theme changes and feeds this window's system appearance into it, now and on
/// every change. Keep the subscriptions for as long as the view lives.
pub fn follow_theme<V: 'static>(window: &mut Window, cx: &mut Context<V>) -> [Subscription; 2] {
    set_system_appearance(cx, window.appearance());
    let appearance = window
        .observe_window_appearance(|window, cx| set_system_appearance(cx, window.appearance()));
    [cx.observe_global::<Theme>(|_, cx| cx.notify()), appearance]
}
