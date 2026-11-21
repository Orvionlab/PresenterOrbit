//! Layer-wide lists for text layers. The renderer formats the same markers in
//! `theme-text-list.ts`; both are checked against `tests/fixtures/text/list-markers.json`.
//!
//! Every non-empty paragraph of the layer is a list item at one level. Empty paragraphs get no
//! marker and are not counted (an Orbit decision, not compared with ProPresenter). Bullets are
//! fixed characters. A numbered marker is prefix + number + postfix, counting from the start
//! (default 1). Alphabetic numbers run a, b .. z, aa, ab .. like CSS `lower-alpha`, and roman
//! numbers cover 1 to 3999. A number outside the range of its style (only reachable when a list
//! counts past 3999 in roman) is written as plain decimal digits, as CSS `list-style` does.

use serde_json::{Map, Value};

pub(crate) const STYLE_FIELD: &str = "textListStyle";
pub(crate) const PREFIX_FIELD: &str = "textListPrefix";
pub(crate) const POSTFIX_FIELD: &str = "textListPostfix";
pub(crate) const START_FIELD: &str = "textListStart";
const MAX_AFFIX_CHARS: usize = 8;
const MAX_START: i64 = 9999;
const MAX_ROMAN: i64 = 3999;
const STYLE_NAMES: &str =
    "box, check, diamond, disc, circle, square, hyphen, decimal, lower-alpha, upper-alpha, lower-roman or upper-roman";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ListStyle {
    Box,
    Check,
    Diamond,
    Disc,
    Circle,
    Square,
    Hyphen,
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
}

impl ListStyle {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "box" => Self::Box,
            "check" => Self::Check,
            "diamond" => Self::Diamond,
            "disc" => Self::Disc,
            "circle" => Self::Circle,
            "square" => Self::Square,
            "hyphen" => Self::Hyphen,
            "decimal" => Self::Decimal,
            "lower-alpha" => Self::LowerAlpha,
            "upper-alpha" => Self::UpperAlpha,
            "lower-roman" => Self::LowerRoman,
            "upper-roman" => Self::UpperRoman,
            _ => return None,
        })
    }

    fn bullet(self) -> Option<char> {
        match self {
            Self::Box => Some('□'),
            Self::Check => Some('✓'),
            Self::Diamond => Some('◆'),
            Self::Disc => Some('\u{2022}'),
            Self::Circle => Some('\u{25E6}'),
            Self::Square => Some('\u{25AA}'),
            Self::Hyphen => Some('-'),
            _ => None,
        }
    }

    pub(crate) fn is_numbered(self) -> bool {
        self.bullet().is_none()
    }

    /// Alphabetic and roman numbering have no zero.
    fn min_start(self) -> i64 {
        if self == Self::Decimal {
            0
        } else {
            1
        }
    }

    fn max_start(self) -> i64 {
        match self {
            Self::LowerRoman | Self::UpperRoman => MAX_ROMAN,
            _ => MAX_START,
        }
    }
}

/// `number` written in a numbered `style`; numbers outside the range of the style are decimal.
pub(crate) fn format_number(style: ListStyle, number: i64) -> String {
    match style {
        ListStyle::LowerAlpha if number >= 1 => alphabetic(number, b'a'),
        ListStyle::UpperAlpha if number >= 1 => alphabetic(number, b'A'),
        ListStyle::LowerRoman if (1..=MAX_ROMAN).contains(&number) => roman(number).to_lowercase(),
        ListStyle::UpperRoman if (1..=MAX_ROMAN).contains(&number) => roman(number),
        _ => number.to_string(),
    }
}

fn alphabetic(mut number: i64, first: u8) -> String {
    let mut letters = Vec::new();
    while number > 0 {
        number -= 1;
        letters.push(first + (number % 26) as u8);
        number /= 26;
    }
    letters.reverse();
    String::from_utf8(letters).expect("ASCII letters")
}

fn roman(mut number: i64) -> String {
    const NUMERALS: [(i64, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut text = String::new();
    for (value, numeral) in NUMERALS {
        while number >= value {
            text.push_str(numeral);
            number -= value;
        }
    }
    text
}

fn invalid_affix(field: &'static str, value: &Value) -> Option<String> {
    let valid = value.as_str().is_some_and(|text| {
        text.chars().count() <= MAX_AFFIX_CHARS
            && !text.chars().any(|character| {
                character.is_control() || matches!(character, '\u{2028}' | '\u{2029}')
            })
    });
    (!valid).then(|| {
        format!(
            "{field} must be a string of at most {MAX_AFFIX_CHARS} characters without line breaks or tabs."
        )
    })
}

/// The list settings of `object` that are not supported, with the reason for each. The prefix,
/// postfix need a numbered style. Bullet styles preserve their unused starting number.
pub(crate) fn invalid_list_fields(object: &Map<String, Value>) -> Vec<(&'static str, String)> {
    let mut invalid = Vec::new();
    let style = match object.get(STYLE_FIELD) {
        None => None,
        Some(value) if value.as_str() == Some("none") => None,
        Some(value) => match value.as_str().and_then(ListStyle::parse) {
            Some(style) => Some(style),
            None => {
                invalid.push((STYLE_FIELD, format!("{STYLE_FIELD} must be {STYLE_NAMES}.")));
                return invalid;
            }
        },
    };
    for field in [PREFIX_FIELD, POSTFIX_FIELD, START_FIELD] {
        let Some(value) = object.get(field) else {
            continue;
        };
        let reason = match style {
            None => Some(format!("{field} requires {STYLE_FIELD}.")),
            Some(style) if !style.is_numbered() && field != START_FIELD => {
                Some(format!("{field} applies to numbered list styles only."))
            }
            Some(style) if field == START_FIELD => {
                let (min, max) = (style.min_start(), style.max_start());
                (!value.as_i64().is_some_and(|start| (min..=max).contains(&start))).then(|| {
                    format!("{field} must be a whole number between {min} and {max} for this list style.")
                })
            }
            Some(_) => invalid_affix(field, value),
        };
        invalid.extend(reason.map(|reason| (field, reason)));
    }
    invalid
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TextList {
    style: ListStyle,
    prefix: String,
    postfix: String,
    start: i64,
}

impl TextList {
    pub fn start(&self) -> i64 {
        self.start
    }

    pub fn marker(&self, number: i64) -> String {
        match self.style.bullet() {
            Some(bullet) => bullet.to_string(),
            None => format!(
                "{}{}{}",
                self.prefix,
                format_number(self.style, number),
                self.postfix
            ),
        }
    }

    pub fn same_format(&self, other: &Self) -> bool {
        self.style == other.style
            && self.prefix == other.prefix
            && self.postfix == other.postfix
            && self.start == other.start
    }
    /// The list of a validated text style, or `None` while no list style is set.
    pub fn from_style(style: &Map<String, Value>) -> Option<Self> {
        let list_style = style
            .get(STYLE_FIELD)
            .and_then(Value::as_str)
            .and_then(ListStyle::parse)?;
        let text = |field: &str| {
            style
                .get(field)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        Some(Self {
            style: list_style,
            prefix: text(PREFIX_FIELD),
            postfix: text(POSTFIX_FIELD),
            start: style.get(START_FIELD).and_then(Value::as_i64).unwrap_or(1),
        })
    }
}
