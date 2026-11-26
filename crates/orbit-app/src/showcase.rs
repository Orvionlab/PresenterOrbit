use gpui::{div, prelude::*, px, uniform_list, Context, Entity, Point, Subscription, Window};
use orbit_ui::controls::{
    input_state, list_row, range_state, textarea_state, tree_row, ActionButton, ButtonVariant,
    CheckboxField, CollectionAction, CollectionOption, CollectionPicker, ColorPicker, ContextMenu,
    Dialog, Disclosure, EmptyState, Field, IconButton, InputState, MenuEntry, MenuItem,
    NumberInput, OutputField, PanelHeader, RangeInput, SegmentedControl, SegmentedOption, Select,
    SelectOption, SliderState, TabItem, TabList, TextInput, Textarea, TextareaState, ToggleSwitch,
    STANDARD_COLORS,
};
use orbit_ui::{
    appearance_settings, follow_theme, set_appearance_settings, AccentSetting, ActiveTheme,
    AppearanceSettings, TextScale, ThemeSetting, Translator, UiDensity,
};

const LIST_ROWS: usize = 10_000;

/// Developer window (`--showcase`) with every primitive in every state, the counterpart of `?ui-showcase=1`.
pub struct ShowcaseView {
    dutch: bool,
    text: Entity<InputState>,
    compact_text: Entity<InputState>,
    disabled_text: Entity<InputState>,
    filter: Entity<InputState>,
    disabled_filter: Entity<InputState>,
    rich_text: Entity<orbit_ui::controls::RichText>,
    long_text: Entity<TextareaState>,
    number: Entity<InputState>,
    angle: f64,
    range: Entity<SliderState>,
    media_range: Entity<SliderState>,
    checked: bool,
    switched: bool,
    choice: usize,
    stroke: usize,
    collections: Vec<CollectionOption>,
    collection: String,
    tab: usize,
    open_section: bool,
    selected_row: usize,
    color: String,
    menu: Option<Point<gpui::Pixels>>,
    dialog: bool,
    _theme: [Subscription; 2],
}

impl ShowcaseView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            dutch: true,
            text: input_state("Presenter Orbit", window, cx),
            compact_text: input_state("Presenter Orbit", window, cx),
            disabled_text: input_state("Presenter Orbit", window, cx),
            filter: orbit_ui::controls::filter_state("", window, cx),
            disabled_filter: orbit_ui::controls::filter_state("", window, cx),
            rich_text: cx.new(|cx|orbit_ui::controls::RichText::new(orbit_ui::controls::RunDocument::new(serde_json::json!([{"text":"Orbit ","style":{"bold":true}},{"text":"rich text","style":{"italic":true,"underline":true,"textColor":"#ff0000"}}])).unwrap(),"Rich text".into(),false,window,cx)),
            long_text: textarea_state("First line\nSecond line", window, cx),
            number: input_state("42", window, cx),
            angle: 45.,
            range: range_state(0., 100., 1., 40., cx),
            media_range: range_state(0., 100., 1., 40., cx),
            checked: true,
            switched: true,
            choice: 0,
            stroke: 0,
            collections: vec![CollectionOption::new("default", "Default Collection"), CollectionOption::new("second", "Second Collection")],
            collection: "default".into(),
            tab: 0,
            open_section: true,
            selected_row: 3,
            color: STANDARD_COLORS[5].to_string(),
            menu: None,
            dialog: false,
            _theme: follow_theme(window, cx),
        }
    }

    fn appearance_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let view = cx.entity();
        let setter = |change: fn(&mut AppearanceSettings, usize)| {
            move |value: usize, _: &mut Window, cx: &mut gpui::App| {
                let mut settings = appearance_settings(cx);
                change(&mut settings, value);
                set_appearance_settings(cx, settings);
            }
        };
        let segmented = |id: &'static str, names: &[&'static str], value: usize| {
            SegmentedControl::new(
                id,
                id,
                names
                    .iter()
                    .enumerate()
                    .map(|(index, name)| SegmentedOption::text(index, *name))
                    .collect(),
                value,
            )
        };
        let settings = appearance_settings(cx);
        let language_view = view.clone();
        div()
            .flex()
            .flex_wrap()
            .flex_none()
            .items_center()
            .gap(theme.metrics.space_md)
            .p(theme.metrics.space_sm)
            .border_b_1()
            .border_color(theme.colors.border)
            .bg(theme.colors.panel_bg_strong)
            .child(
                segmented(
                    "theme",
                    &["Dark", "Light", "Red", "System"],
                    match settings.theme {
                        ThemeSetting::Dark => 0,
                        ThemeSetting::Light => 1,
                        ThemeSetting::Red => 2,
                        ThemeSetting::System => 3,
                    },
                )
                .on_change(setter(|settings, value| {
                    settings.theme = [
                        ThemeSetting::Dark,
                        ThemeSetting::Light,
                        ThemeSetting::Red,
                        ThemeSetting::System,
                    ][value];
                })),
            )
            .child(
                segmented(
                    "accent",
                    &["Blue", "Red", "Purple", "Green"],
                    match settings.accent {
                        AccentSetting::Blue => 0,
                        AccentSetting::Red => 1,
                        AccentSetting::Purple => 2,
                        AccentSetting::Green => 3,
                    },
                )
                .on_change(setter(|settings, value| {
                    settings.accent = [
                        AccentSetting::Blue,
                        AccentSetting::Red,
                        AccentSetting::Purple,
                        AccentSetting::Green,
                    ][value];
                })),
            )
            .child(
                segmented(
                    "density",
                    &["Compact", "Comfortable", "Spacious"],
                    UiDensity::ALL
                        .iter()
                        .position(|density| *density == settings.density)
                        .unwrap_or(1),
                )
                .on_change(setter(|settings, value| {
                    settings.density = UiDensity::ALL[value];
                })),
            )
            .child(
                segmented(
                    "text-scale",
                    &["Small", "Default", "Large", "XLarge"],
                    TextScale::ALL
                        .iter()
                        .position(|scale| *scale == settings.text_scale)
                        .unwrap_or(1),
                )
                .on_change(setter(|settings, value| {
                    settings.text_scale = TextScale::ALL[value];
                })),
            )
            .child(
                SegmentedControl::new(
                    "language",
                    "language",
                    vec![
                        SegmentedOption::text(true, "NL"),
                        SegmentedOption::text(false, "EN"),
                    ],
                    self.dutch,
                )
                .on_change(move |dutch, _, cx| {
                    cx.set_global(Translator::new(if dutch { "nl" } else { "en" }));
                    language_view.update(cx, |this, cx| {
                        this.dutch = dutch;
                        cx.notify();
                    });
                }),
            )
    }

    fn section(
        title: &'static str,
        cx: &mut Context<Self>,
        content: impl IntoElement,
    ) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .flex()
            .flex_col()
            .border_1()
            .border_color(theme.colors.border_soft)
            .child(PanelHeader::new(title))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(theme.metrics.control_gap)
                    .p(theme.metrics.space_sm)
                    .child(content),
            )
    }

    fn buttons(cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let row = |disabled: bool, prefix: &'static str| {
            let id = move |name: &'static str| format!("{prefix}-{name}");
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(theme.metrics.control_gap)
                .child(
                    ActionButton::new(id("primary"), "Primary")
                        .icon("plus")
                        .disabled(disabled),
                )
                .child(
                    ActionButton::new(id("secondary"), "Secondary")
                        .variant(ButtonVariant::Secondary)
                        .disabled(disabled),
                )
                .child(
                    ActionButton::new(id("ghost"), "Ghost")
                        .compact()
                        .variant(ButtonVariant::Ghost)
                        .disabled(disabled),
                )
                .child(
                    ActionButton::new(id("danger"), "Danger")
                        .variant(ButtonVariant::Danger)
                        .disabled(disabled),
                )
                .child(IconButton::new(id("icon"), "search", "Search").disabled(disabled))
                .child(
                    IconButton::new(id("icon-secondary"), "x", "Close")
                        .variant(ButtonVariant::Secondary)
                        .disabled(disabled),
                )
                .child(IconButton::new(id("icon-selected"), "eye", "Show").selected(!disabled))
                .child(
                    IconButton::new(id("icon-inline"), "x", "Remove")
                        .inline()
                        .disabled(disabled),
                )
                .child(
                    ActionButton::new(id("value-selected"), "10s")
                        .icon("clock-play")
                        .compact()
                        .variant(ButtonVariant::Ghost)
                        .selected(!disabled)
                        .disabled(disabled),
                )
        };
        div()
            .flex()
            .flex_col()
            .gap(theme.metrics.control_gap)
            .child(row(false, "enabled"))
            .child(row(true, "disabled"))
            .child(
                div()
                    .flex()
                    .gap(theme.metrics.control_gap)
                    .child(orbit_ui::icons::media_layer_icon(true, theme))
                    .child(orbit_ui::icons::media_layer_icon(false, theme)),
            )
    }

    fn fields(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let collection_view = cx.entity();
        let theme = cx.theme();
        let t = *cx.global::<Translator>();
        let color_view = cx.entity();
        let options: Vec<_> = [
            "colorPicker.standard",
            "colorPicker.advanced",
            "colorPicker.noBackground",
            "status.workspace.none",
        ]
        .iter()
        .enumerate()
        .map(|(index, key)| {
            SelectOption::new(index, t.t(key)).disabled(*key == "colorPicker.noBackground")
        })
        .collect();
        let select_view = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap(theme.metrics.control_gap)
            .child(Field::new(
                t.t("colorPicker.hexColor"),
                TextInput::new("text", &self.text).label(t.t("colorPicker.hexColor")),
            ))
            .child(
                Field::new(
                    "Compact, inline",
                    TextInput::new("text-compact", &self.compact_text).compact(),
                )
                .inline()
                .label_width(cx.theme().metrics.space_lg * 4.),
            )
            .child(orbit_ui::controls::FilterInput::new("filter", &self.filter))
            .child(
                orbit_ui::controls::FilterInput::new("filter-disabled", &self.disabled_filter)
                    .disabled(true),
            )
            .child(
                Field::new(
                    "Disabled",
                    TextInput::new("text-disabled", &self.disabled_text).disabled(true),
                )
                .inline(),
            )
            .child(Field::new(
                "Textarea",
                Textarea::new("textarea", &self.long_text),
            ))
            .child(self.rich_text.clone())
            .child(
                Field::new(
                    "Number, hold to repeat",
                    NumberInput::new("number", &self.number)
                        .range(0., 100.)
                        .suffix("%")
                        .stepper(),
                )
                .inline(),
            )
            .child(Field::new("Range", RangeInput::new(&self.range)).inline())
            .child(
                Field::new(
                    format!("Rotation · {}°", self.angle),
                    orbit_ui::controls::AngleDial::new("rotation", "Rotation", self.angle)
                        .on_change(cx.listener(|view, angle: &f64, _, cx| {
                            view.angle = *angle;
                            cx.notify();
                        })),
                )
                .inline(),
            )
            .child(
                Select::new(
                    "stroke-previews",
                    vec![
                        SelectOption::new(0, "Solid").preview_icon("stroke-solid"),
                        SelectOption::new(1, "Short dash").preview_icon("stroke-short"),
                        SelectOption::new(2, "Long dash").preview_icon("stroke-long"),
                    ],
                    self.stroke,
                )
                .compact()
                .label("Stroke preview")
                .on_change({
                    let owner = cx.entity();
                    move |value, _, cx| {
                        owner.update(cx, |view, cx| {
                            view.stroke = value;
                            cx.notify();
                        })
                    }
                }),
            )
            .child(
                CollectionPicker::new(
                    "showcase-collections",
                    "Collections",
                    self.collections
                        .iter()
                        .cloned()
                        .map(|mut option| {
                            option.delete = self.collections.len() > 1;
                            option
                        })
                        .collect(),
                    self.collection.clone(),
                    "Delete this collection?",
                )
                .on_action(move |action, _, cx| {
                    collection_view.update(cx, |this, cx| {
                        match action {
                            CollectionAction::Select(id) => this.collection = id,
                            CollectionAction::Create => {
                                let id = uuid::Uuid::new_v4().to_string();
                                this.collections
                                    .push(CollectionOption::new(&id, "New Collection"));
                                this.collection = id;
                            }
                            CollectionAction::Rename { id, title } => {
                                if let Some(option) =
                                    this.collections.iter_mut().find(|option| option.id == id)
                                {
                                    option.label = title;
                                }
                            }
                            CollectionAction::Duplicate(id) => {
                                if let Some(option) = this
                                    .collections
                                    .iter()
                                    .find(|option| option.id == id)
                                    .cloned()
                                {
                                    let id = uuid::Uuid::new_v4().to_string();
                                    this.collections.push(CollectionOption::new(
                                        &id,
                                        format!("{} Copy", option.label),
                                    ));
                                    this.collection = id;
                                }
                            }
                            CollectionAction::Delete(id) => {
                                this.collections.retain(|option| option.id != id);
                                if this.collection == id {
                                    if let Some(option) = this.collections.first() {
                                        this.collection = option.id.clone();
                                    }
                                }
                            }
                            CollectionAction::Reorder {
                                source,
                                target,
                                after,
                            } => {
                                if let Some(from) = this
                                    .collections
                                    .iter()
                                    .position(|option| option.id == source)
                                {
                                    let option = this.collections.remove(from);
                                    if let Some(to) = this
                                        .collections
                                        .iter()
                                        .position(|option| option.id == target)
                                    {
                                        this.collections.insert(to + usize::from(after), option);
                                    }
                                }
                            }
                        }
                        cx.notify();
                    })
                }),
            )
            .child(
                Field::new("Media progress", RangeInput::new(&self.media_range).scrub()).inline(),
            )
            .child(
                Field::new(
                    "Select",
                    Select::new("select", options.clone(), self.choice)
                        .label("Select")
                        .on_change(move |value, _, cx| {
                            select_view.update(cx, |this, cx| {
                                this.choice = value;
                                cx.notify();
                            })
                        }),
                )
                .inline(),
            )
            .child(
                Field::new(
                    "Disabled select",
                    Select::new("select-disabled", options, 0usize)
                        .compact()
                        .disabled(true),
                )
                .inline(),
            )
            .child(
                Field::new(
                    "Color",
                    ColorPicker::new("color", "Color", self.color.clone())
                        .anchor(gpui::Anchor::BottomLeft)
                        .close_on_selection()
                        .on_change(move |value, _, cx| {
                            color_view.update(cx, |this, cx| {
                                this.color = value;
                                cx.notify();
                            })
                        }),
                )
                .inline(),
            )
            .child(OutputField::new("Output", "1920 x 1080"))
    }

    fn toggles(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (check_view, switch_view) = (cx.entity(), cx.entity());
        let (tab_view, section_view) = (cx.entity(), cx.entity());
        let t = *cx.global::<Translator>();
        let tabs = vec![
            TabItem::new(0usize, t.t("colorPicker.standard")),
            TabItem::new(1usize, t.t("colorPicker.advanced")),
            TabItem::new(2usize, "Disabled").disabled(true),
        ];
        let icon_tabs = vec![
            TabItem::new(0usize, "Search").icon("search"),
            TabItem::new(1usize, "Eye").icon("eye"),
        ];
        let icon_view = cx.entity();
        div()
            .flex()
            .flex_col()
            .gap(theme.metrics.control_gap)
            .child(PanelHeader::new("Section header with contrast").contrast())
            .child(
                CheckboxField::new("check", "Checkbox", self.checked).on_change(
                    move |checked, _, cx| {
                        check_view.update(cx, |this, cx| {
                            this.checked = checked;
                            cx.notify();
                        })
                    },
                ),
            )
            .child(CheckboxField::new("check-bare", "Compact checkbox", true).bare())
            .child(CheckboxField::new("check-mixed", "Some selected", false).mixed(true))
            .child(CheckboxField::new("check-disabled", "Disabled", true).disabled(true))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(theme.metrics.space_md)
                    .child(
                        ToggleSwitch::new("switch", "Switch", self.switched).on_change(
                            move |value, _, cx| {
                                switch_view.update(cx, |this, cx| {
                                    this.switched = value;
                                    cx.notify();
                                })
                            },
                        ),
                    )
                    .child(ToggleSwitch::new("switch-off", "Off", false))
                    .child(ToggleSwitch::new("switch-disabled", "Disabled", true).disabled(true)),
            )
            .child(
                TabList::new("tabs", "Tabs", tabs, self.tab)
                    .equal_width()
                    .on_change(move |tab, _, cx| {
                        tab_view.update(cx, |this, cx| {
                            this.tab = tab;
                            cx.notify();
                        })
                    }),
            )
            .child(
                TabList::new("icon-tabs", "Icon tabs", icon_tabs, self.tab).on_change(
                    move |tab, _, cx| {
                        icon_view.update(cx, |this, cx| {
                            this.tab = tab;
                            cx.notify();
                        })
                    },
                ),
            )
            .child(
                Disclosure::new("section", t.t("errorBoundary.details"), self.open_section)
                    .on_toggle(move |open, _, cx| {
                        section_view.update(cx, |this, cx| {
                            this.open_section = open;
                            cx.notify();
                        })
                    })
                    .child(EmptyState::new(t.t("status.workspace.none"))),
            )
    }

    fn lists(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let selected = self.selected_row;
        div()
            .flex()
            .flex_col()
            .h(px(220.))
            .child(tree_row("tree-folder", 0, Some(true), false, cx).child("Library"))
            .child(tree_row("tree-child", 1, None, true, cx).child("Selected item"))
            .child(tree_row("tree-closed", 1, Some(false), false, cx).child("Closed folder"))
            .child(
                uniform_list(
                    "showcase-list",
                    LIST_ROWS,
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|index| {
                                (if index == 0 {
                                    orbit_ui::controls::list_row_without_hover(
                                        ("row", index),
                                        index == this.selected_row,
                                        cx,
                                    )
                                } else {
                                    list_row(("row", index), index == this.selected_row, cx)
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.selected_row = index;
                                    cx.notify();
                                }))
                                .child(format!("Row {index} of {LIST_ROWS}"))
                            })
                            .collect()
                    }),
                )
                .flex_1()
                .border_t_1()
                .border_color(theme.colors.border_soft),
            )
            .id(("list-area", selected))
    }

    fn overlays(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .flex()
            .flex_wrap()
            .gap(theme.metrics.control_gap)
            .child(
                ActionButton::new("open-dialog", "Open dialog")
                    .variant(ButtonVariant::Secondary)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.dialog = true;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("menu-target")
                    .flex()
                    .items_center()
                    .px(theme.metrics.space_sm)
                    .h(theme.metrics.control_height)
                    .border_1()
                    .border_color(theme.colors.border)
                    .text_size(theme.metrics.font_sm)
                    .child("Right-click for a menu")
                    .on_mouse_down(
                        gpui::MouseButton::Right,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            this.menu = Some(event.position);
                            cx.notify();
                        }),
                    ),
            )
    }

    fn menu_entries(&self, cx: &mut Context<Self>) -> Vec<MenuEntry> {
        let owner = cx.weak_entity();
        vec![
            MenuEntry::Header("Actions".into()),
            MenuItem::new("Preview row")
                .content(
                    div()
                        .flex()
                        .flex_col()
                        .child("Display 1")
                        .child("1920 × 1080"),
                )
                .into(),
            MenuItem::new("Copy").icon("copy").meta("Ctrl+C").into(),
            MenuEntry::SelectableSubmenu {
                item: MenuItem::new("Selected")
                    .checked(true)
                    .on_click(|_, _, _| {}),
                entries: vec![
                    MenuItem::new("Rename").into(),
                    MenuItem::new("Delete").into(),
                ],
            },
            MenuItem::new("Disabled").disabled(true).into(),
            MenuEntry::Control {
                label: "Color".into(),
                control: div()
                    .w(cx.theme().metrics.control_height * 2.)
                    .child(
                        ColorPicker::new("showcase-menu-color", "Color", self.color.clone())
                            .on_change(move |color, _, cx| {
                                owner
                                    .update(cx, |view, cx| {
                                        view.color = color;
                                        cx.notify();
                                    })
                                    .ok();
                            }),
                    )
                    .into_any_element(),
            },
            MenuEntry::Separator,
            MenuEntry::Submenu {
                label: "More".into(),
                disabled: false,
                entries: vec![
                    MenuItem::new("First").into(),
                    MenuEntry::Submenu {
                        label: "Deeper".into(),
                        disabled: false,
                        entries: vec![MenuItem::new("Last").into()],
                    },
                ],
            },
            MenuEntry::Submenu {
                label: "Disabled submenu".into(),
                disabled: true,
                entries: Vec::new(),
            },
        ]
    }
}

impl Render for ShowcaseView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let buttons = Self::buttons(cx);
        let fields = self.fields(cx);
        let toggles = self.toggles(cx);
        let lists = self.lists(cx);
        let overlays = self.overlays(cx);
        let button_section = Self::section("Actions", cx, buttons);
        let field_section = Self::section("Fields", cx, fields);
        let toggle_section = Self::section("Choices and sections", cx, toggles);
        let list_section = Self::section("Lists", cx, lists);
        let overlay_section = Self::section("Dialog and menu", cx, overlays);
        let bar = self.appearance_bar(cx);
        let menu = self.menu.map(|position| {
            let close = cx.entity();
            ContextMenu::new(
                "showcase-menu",
                position,
                self.menu_entries(cx),
                move |_, _, cx| {
                    close.update(cx, |this, cx| {
                        this.menu = None;
                        cx.notify();
                    })
                },
            )
        });
        let dialog = self.dialog.then(|| {
            let close = cx.entity();
            let cancel = cx.entity();
            Dialog::new(
                "showcase-dialog",
                "Dialog title",
                "Close",
                move |_, _, cx| {
                    close.update(cx, |this, cx| {
                        this.dialog = false;
                        cx.notify();
                    })
                },
            )
            .width(px(420.))
            .footer(
                ActionButton::new("dialog-cancel", "Cancel")
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, _, cx| {
                        cancel.update(cx, |this, cx| {
                            this.dialog = false;
                            cx.notify();
                        })
                    }),
            )
            .footer(ActionButton::new("dialog-ok", "Confirm"))
            .child(EmptyState::new("Dialog body"))
        });
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.colors.bg)
            .text_color(theme.colors.text)
            .text_size(theme.metrics.font_md)
            .child(bar)
            .child(
                div()
                    .id("showcase-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_start()
                            .gap(theme.metrics.space_md)
                            .p(theme.metrics.space_md)
                            .child(div().w(px(420.)).child(button_section))
                            .child(div().w(px(420.)).child(field_section))
                            .child(div().w(px(420.)).child(toggle_section))
                            .child(div().w(px(420.)).child(list_section))
                            .child(div().w(px(420.)).child(overlay_section)),
                    ),
            )
            .children(menu)
            .children(dialog)
    }
}
