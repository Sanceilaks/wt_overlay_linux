use std::{collections::HashSet, fmt, str::FromStr};

pub const DEFAULT_MAX_TEXT_NODES: usize = 128;
pub const DEFAULT_MAX_TEXT_CHARS: usize = 512;
pub const MIN_FONT_SIZE: f32 = 6.0;
pub const MAX_FONT_SIZE: f32 = 256.0;
pub const MIN_BLINK_HZ: f32 = 0.1;
pub const MAX_BLINK_HZ: f32 = 20.0;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Slot {
    CrosshairTop,
    CrosshairRight,
    CrosshairBottom,
    CrosshairLeft,
    ScreenTopLeft,
    ScreenTopCenter,
    ScreenTopRight,
    ScreenLeftTop,
    ScreenLeftMiddle,
    ScreenLeftBottom,
    ScreenRightTop,
    ScreenRightMiddle,
    ScreenRightBottom,
    ScreenBottomLeft,
    ScreenBottomCenter,
    ScreenBottomRight,
}

impl Slot {
    pub const ALL: [Self; 16] = [
        Self::CrosshairTop,
        Self::CrosshairRight,
        Self::CrosshairBottom,
        Self::CrosshairLeft,
        Self::ScreenTopLeft,
        Self::ScreenTopCenter,
        Self::ScreenTopRight,
        Self::ScreenLeftTop,
        Self::ScreenLeftMiddle,
        Self::ScreenLeftBottom,
        Self::ScreenRightTop,
        Self::ScreenRightMiddle,
        Self::ScreenRightBottom,
        Self::ScreenBottomLeft,
        Self::ScreenBottomCenter,
        Self::ScreenBottomRight,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CrosshairTop => "crosshair-top",
            Self::CrosshairRight => "crosshair-right",
            Self::CrosshairBottom => "crosshair-bottom",
            Self::CrosshairLeft => "crosshair-left",
            Self::ScreenTopLeft => "screen-top-left",
            Self::ScreenTopCenter => "screen-top-center",
            Self::ScreenTopRight => "screen-top-right",
            Self::ScreenLeftTop => "screen-left-top",
            Self::ScreenLeftMiddle => "screen-left-middle",
            Self::ScreenLeftBottom => "screen-left-bottom",
            Self::ScreenRightTop => "screen-right-top",
            Self::ScreenRightMiddle => "screen-right-middle",
            Self::ScreenRightBottom => "screen-right-bottom",
            Self::ScreenBottomLeft => "screen-bottom-left",
            Self::ScreenBottomCenter => "screen-bottom-center",
            Self::ScreenBottomRight => "screen-bottom-right",
        }
    }
}

impl fmt::Display for Slot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Slot {
    type Err = UnknownSlot;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|slot| slot.as_str() == value)
            .ok_or_else(|| UnknownSlot(value.to_owned()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownSlot(pub String);

impl fmt::Display for UnknownSlot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown HUD slot {:?}", self.0)
    }
}

impl std::error::Error for UnknownSlot {}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

impl Rgba {
    pub const WHITE: Self = Self::from_u8(255, 255, 255, 255);
    pub const BLACK: Self = Self::from_u8(0, 0, 0, 255);
    pub const TRANSPARENT: Self = Self::from_u8(0, 0, 0, 0);

    pub fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Result<Self, InvalidColor> {
        let color = Self {
            red,
            green,
            blue,
            alpha,
        };
        color.validate()?;
        Ok(color)
    }

    pub const fn from_u8(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        const SCALE: f32 = 1.0 / 255.0;
        Self {
            red: red as f32 * SCALE,
            green: green as f32 * SCALE,
            blue: blue as f32 * SCALE,
            alpha: alpha as f32 * SCALE,
        }
    }

    pub fn validate(self) -> Result<(), InvalidColor> {
        for (name, value) in [
            ("red", self.red),
            ("green", self.green),
            ("blue", self.blue),
            ("alpha", self.alpha),
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(InvalidColor { name, value });
            }
        }
        Ok(())
    }
}

impl FromStr for Rgba {
    type Err = ParseColorError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let hex = value
            .strip_prefix('#')
            .ok_or(ParseColorError::MissingHash)?;
        if hex.len() != 6 && hex.len() != 8 {
            return Err(ParseColorError::InvalidLength(hex.len()));
        }
        if !hex.is_ascii() {
            return Err(ParseColorError::InvalidHex);
        }

        let component = |start| {
            u8::from_str_radix(&hex[start..start + 2], 16).map_err(|_| ParseColorError::InvalidHex)
        };
        let red = component(0)?;
        let green = component(2)?;
        let blue = component(4)?;
        let alpha = if hex.len() == 8 { component(6)? } else { 255 };
        Ok(Self::from_u8(red, green, blue, alpha))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InvalidColor {
    pub name: &'static str,
    pub value: f32,
}

impl fmt::Display for InvalidColor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "color component {} must be finite and between 0 and 1, got {}",
            self.name, self.value
        )
    }
}

impl std::error::Error for InvalidColor {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseColorError {
    MissingHash,
    InvalidLength(usize),
    InvalidHex,
}

impl fmt::Display for ParseColorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHash => formatter.write_str("color must start with '#'"),
            Self::InvalidLength(length) => write!(
                formatter,
                "color must contain 6 or 8 hexadecimal digits, got {length}"
            ),
            Self::InvalidHex => formatter.write_str("color contains a non-hexadecimal digit"),
        }
    }
}

impl std::error::Error for ParseColorError {}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FontWeight {
    #[default]
    Normal,
    Bold,
}

impl FontWeight {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Bold => "bold",
        }
    }
}

impl FromStr for FontWeight {
    type Err = UnknownFontWeight;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "normal" => Ok(Self::Normal),
            "bold" => Ok(Self::Bold),
            _ => Err(UnknownFontWeight(value.to_owned())),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownFontWeight(pub String);

impl fmt::Display for UnknownFontWeight {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "unknown font weight {:?}", self.0)
    }
}

impl std::error::Error for UnknownFontWeight {}

#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub foreground: Rgba,
    pub font_size: f32,
    pub weight: FontWeight,
    pub shadow: Option<Rgba>,
    pub blink_hz: Option<f32>,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            foreground: Rgba::WHITE,
            font_size: 22.0,
            weight: FontWeight::Normal,
            shadow: Some(Rgba::from_u8(0, 0, 0, 204)),
            blink_hz: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextNode {
    pub id: String,
    pub slot: Slot,
    pub text: String,
    pub style: TextStyle,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextScene {
    pub revision: u64,
    pub nodes: Vec<TextNode>,
}

impl TextScene {
    pub fn validate(&self) -> Result<(), SceneValidationError> {
        self.validate_with_limits(SceneLimits::default())
    }

    pub fn validate_with_limits(&self, limits: SceneLimits) -> Result<(), SceneValidationError> {
        if self.nodes.len() > limits.max_nodes {
            return Err(SceneValidationError::TooManyNodes {
                actual: self.nodes.len(),
                maximum: limits.max_nodes,
            });
        }

        let mut ids = HashSet::with_capacity(self.nodes.len());
        for node in &self.nodes {
            if node.id.is_empty() {
                return Err(SceneValidationError::EmptyId);
            }
            if !ids.insert(node.id.as_str()) {
                return Err(SceneValidationError::DuplicateId(node.id.clone()));
            }
            let length = node.text.chars().count();
            if length > limits.max_text_chars {
                return Err(SceneValidationError::TextTooLong {
                    id: node.id.clone(),
                    actual: length,
                    maximum: limits.max_text_chars,
                });
            }
            validate_style(&node.id, &node.style)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SceneLimits {
    pub max_nodes: usize,
    pub max_text_chars: usize,
}

impl Default for SceneLimits {
    fn default() -> Self {
        Self {
            max_nodes: DEFAULT_MAX_TEXT_NODES,
            max_text_chars: DEFAULT_MAX_TEXT_CHARS,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SceneValidationError {
    TooManyNodes {
        actual: usize,
        maximum: usize,
    },
    EmptyId,
    DuplicateId(String),
    TextTooLong {
        id: String,
        actual: usize,
        maximum: usize,
    },
    InvalidForeground {
        id: String,
        source: InvalidColor,
    },
    InvalidShadow {
        id: String,
        source: InvalidColor,
    },
    InvalidFontSize {
        id: String,
        value: f32,
    },
    InvalidBlinkFrequency {
        id: String,
        value: f32,
    },
}

fn validate_style(id: &str, style: &TextStyle) -> Result<(), SceneValidationError> {
    style
        .foreground
        .validate()
        .map_err(|source| SceneValidationError::InvalidForeground {
            id: id.to_owned(),
            source,
        })?;
    if let Some(shadow) = style.shadow {
        shadow
            .validate()
            .map_err(|source| SceneValidationError::InvalidShadow {
                id: id.to_owned(),
                source,
            })?;
    }
    if !style.font_size.is_finite() || !(MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&style.font_size) {
        return Err(SceneValidationError::InvalidFontSize {
            id: id.to_owned(),
            value: style.font_size,
        });
    }
    if let Some(blink_hz) = style.blink_hz
        && (!blink_hz.is_finite() || !(MIN_BLINK_HZ..=MAX_BLINK_HZ).contains(&blink_hz))
    {
        return Err(SceneValidationError::InvalidBlinkFrequency {
            id: id.to_owned(),
            value: blink_hz,
        });
    }
    Ok(())
}

impl fmt::Display for SceneValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyNodes { actual, maximum } => {
                write!(formatter, "scene has {actual} nodes, maximum is {maximum}")
            }
            Self::EmptyId => formatter.write_str("text node ID must not be empty"),
            Self::DuplicateId(id) => write!(formatter, "duplicate text node ID {id:?}"),
            Self::TextTooLong {
                id,
                actual,
                maximum,
            } => write!(
                formatter,
                "text node {id:?} has {actual} characters, maximum is {maximum}"
            ),
            Self::InvalidForeground { id, source } => {
                write!(
                    formatter,
                    "text node {id:?} has invalid foreground: {source}"
                )
            }
            Self::InvalidShadow { id, source } => {
                write!(formatter, "text node {id:?} has invalid shadow: {source}")
            }
            Self::InvalidFontSize { id, value } => write!(
                formatter,
                "text node {id:?} font size must be finite and in {MIN_FONT_SIZE}..={MAX_FONT_SIZE}, got {value}"
            ),
            Self::InvalidBlinkFrequency { id, value } => write!(
                formatter,
                "text node {id:?} blink frequency must be finite and in {MIN_BLINK_HZ}..={MAX_BLINK_HZ}, got {value}"
            ),
        }
    }
}

impl std::error::Error for SceneValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str) -> TextNode {
        TextNode {
            id: id.to_owned(),
            slot: Slot::CrosshairRight,
            text: "IAS 320".to_owned(),
            style: TextStyle::default(),
        }
    }

    #[test]
    fn every_slot_round_trips_through_its_script_name() {
        for slot in Slot::ALL {
            assert_eq!(slot.as_str().parse(), Ok(slot));
        }
        assert!("arbitrary-position".parse::<Slot>().is_err());
    }

    #[test]
    fn parses_rgb_and_rgba_colors() {
        assert_eq!("#ff8000".parse(), Ok(Rgba::from_u8(255, 128, 0, 255)));
        assert_eq!("#10203040".parse(), Ok(Rgba::from_u8(16, 32, 48, 64)));
        assert!("ff8000".parse::<Rgba>().is_err());
        assert!("#xyzxyz".parse::<Rgba>().is_err());
        assert!("#aéaaa".parse::<Rgba>().is_err());
    }

    #[test]
    fn rejects_duplicate_ids_and_counts_unicode_characters() {
        let duplicated = TextScene {
            revision: 1,
            nodes: vec![node("ias"), node("ias")],
        };
        assert_eq!(
            duplicated.validate(),
            Err(SceneValidationError::DuplicateId("ias".to_owned()))
        );

        let mut unicode = node("warning");
        unicode.text = "УГОЛ".to_owned();
        let scene = TextScene {
            revision: 2,
            nodes: vec![unicode],
        };
        assert!(
            scene
                .validate_with_limits(SceneLimits {
                    max_nodes: 1,
                    max_text_chars: 4,
                })
                .is_ok()
        );
    }

    #[test]
    fn rejects_non_finite_and_out_of_range_style_values() {
        let mut invalid = node("aoa");
        invalid.style.foreground.red = f32::NAN;
        assert!(matches!(
            (TextScene {
                revision: 1,
                nodes: vec![invalid]
            })
            .validate(),
            Err(SceneValidationError::InvalidForeground { .. })
        ));

        let mut invalid = node("aoa");
        invalid.style.blink_hz = Some(0.0);
        assert!(matches!(
            (TextScene {
                revision: 1,
                nodes: vec![invalid]
            })
            .validate(),
            Err(SceneValidationError::InvalidBlinkFrequency { .. })
        ));
    }
}
