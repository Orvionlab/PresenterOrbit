use gpui::{px, Pixels};

/// Spacing and control sizes scale with density. Font sizes scale with text scale. The two never mix.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiDensity {
    Compact,
    Comfortable,
    Spacious,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextScale {
    Small,
    Default,
    Large,
    XLarge,
}

impl UiDensity {
    pub const ALL: [Self; 3] = [Self::Compact, Self::Comfortable, Self::Spacious];

    pub fn id(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Comfortable => "comfortable",
            Self::Spacious => "spacious",
        }
    }

    fn scale(self) -> f32 {
        match self {
            Self::Compact => 0.8,
            Self::Comfortable => 1.0,
            Self::Spacious => 1.2,
        }
    }
}

impl TextScale {
    pub const ALL: [Self; 4] = [Self::Small, Self::Default, Self::Large, Self::XLarge];

    pub fn id(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Default => "default",
            Self::Large => "large",
            Self::XLarge => "xlarge",
        }
    }

    fn scale(self) -> f32 {
        match self {
            Self::Small => 0.92,
            Self::Default => 1.0,
            Self::Large => 1.1,
            Self::XLarge => 1.2,
        }
    }
}

macro_rules! metrics {
    (density { $($d_field:ident $d_css:literal $d_base:literal),* $(,)? }
     text { $($t_field:ident $t_css:literal $t_base:literal),* $(,)? }
     fixed { $($f_field:ident = $f_base:literal),* $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub struct Metrics {
            $(pub $d_field: Pixels,)*
            $(pub $t_field: Pixels,)*
            $(pub $f_field: Pixels,)*
        }

        impl Metrics {
            /// The only place that multiplies by a scale. Views read the fields.
            pub fn new(density: UiDensity, text_scale: TextScale) -> Self {
                Self {
                    $($d_field: px($d_base * density.scale()),)*
                    $($t_field: px($t_base * text_scale.scale()),)*
                    $($f_field: px($f_base),)*
                }
            }

            /// The scaled tokens by their `tokens.css` name, in logical pixels.
            pub fn entries(&self) -> Vec<(&'static str, f32)> {
                vec![
                    $(($d_css, f32::from(self.$d_field)),)*
                    $(($t_css, f32::from(self.$t_field)),)*
                ]
            }
        }
    };
}

metrics! {
    density {
        control_height "orbit-control-height" 30.0,
        control_gap "orbit-control-gap" 8.0,
        panel_gap "orbit-panel-gap" 12.0,
        field_padding_x "orbit-field-padding-x" 10.0,
        space_2xs "orbit-space-2xs" 2.0,
        space_xs "orbit-space-xs" 4.0,
        space_snug "orbit-space-snug" 6.0,
        space_sm "orbit-space-sm" 8.0,
        space_md "orbit-space-md" 12.0,
        space_lg "orbit-space-lg" 16.0,
        row_height "orbit-row-height" 22.0,
        control_height_compact "orbit-control-height-compact" 24.0,
        header_height "orbit-header-height" 24.0,
        tabbar_height "orbit-tabbar-height" 28.0,
        panel_padding "orbit-panel-padding" 14.0,
    }
    text {
        font_xs "orbit-font-size-xs" 11.0,
        font_sm "orbit-font-size-sm" 12.0,
        font_md "orbit-font-size-md" 13.0,
        font_base "orbit-font-size-base" 14.0,
        font_lg "orbit-font-size-lg" 15.0,
        font_xl "orbit-font-size-xl" 18.0,
        font_2xl "orbit-font-size-2xl" 22.0,
        font_3xl "orbit-font-size-3xl" 30.0,
    }
    fixed {
        radius_xs = 2.0,
        radius_sm = 3.0,
        radius_md = 4.0,
        radius_pill = 999.0,
    }
}
