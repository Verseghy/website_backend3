//! The value types of content-v1 CSS and their parsers (editor.md §3.1
//! rules 3 and 4, §7.3): numbers rounded to hundredths, lengths in px,
//! sizes in `em`, percentages and colours.
//!
//! Every parser takes one [`Component`] that the declaration reader produced
//! and returns a typed value, or `None` when the component is not one. Each
//! type has exactly one spelling, its `Display`, so a value that is read and
//! written again is unchanged.
//!
//! Numbers are rounded to two decimals before any bound is checked, so a
//! value that passes is exactly the value that is stored, and reading the
//! stored value again gives the same number. The rounding is
//! `round(x × 100)`, half away from zero. For the non-negative numbers that
//! content-v1 accepts, that is what `Math.round(x * 100)` gives, so the
//! admin's TypeScript canonicaliser can round the same way.

use std::fmt;
use std::num::FpCategory;

use super::declarations::{Argument, Component};

/// A non-negative number with at most two decimals, held as hundredths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hundredths(u32);

impl Hundredths {
    /// Zero.
    pub const ZERO: Self = Self(0);

    /// The whole number `value`.
    ///
    /// - `value`: the number; at most `u32::MAX / 100`.
    #[must_use]
    pub const fn whole(value: u32) -> Self {
        Self(value * 100)
    }

    /// The number `hundredths / 100`.
    ///
    /// - `hundredths`: the number times 100.
    #[must_use]
    pub const fn from_hundredths(hundredths: u32) -> Self {
        Self(hundredths)
    }

    /// The number times 100.
    #[must_use]
    pub const fn hundredths(self) -> u32 {
        self.0
    }

    /// `value` rounded to two decimals, half away from zero, or `None` when
    /// it is negative, not finite, or too large to hold.
    ///
    /// - `value`: the number to round. `-0` counts as zero.
    #[must_use]
    pub fn round(value: f64) -> Option<Self> {
        if !value.is_finite() || value < 0.0 {
            return None;
        }
        let scaled = (value * 100.0).round();
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "`scaled` is a whole number from 0 to u32::MAX, checked on the line before"
        )]
        (scaled <= f64::from(u32::MAX)).then_some(Self(scaled as u32))
    }
}

/// Written with at most two decimals and no trailing zeros: `20`, `0.67`,
/// `1.5` (editor.md §7.3, "Numbers").
impl fmt::Display for Hundredths {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (whole, fraction) = (self.0 / 100, self.0 % 100);
        if fraction == 0 {
            write!(f, "{whole}")
        } else if fraction % 10 == 0 {
            write!(f, "{whole}.{}", fraction / 10)
        } else {
            write!(f, "{whole}.{fraction:02}")
        }
    }
}

/// The absolute length units, with how many of each make one inch (CSS
/// Values 4 §6.2: 1in = 96px = 72pt = 6pc = 2.54cm = 25.4mm = 101.6Q).
/// Each becomes px as `value × 96 / per_inch` (editor.md §3.1 rule 4); `px`
/// itself is taken as it is.
pub const LENGTH_UNITS: &[(&str, f64)] = &[
    ("px", 96.0),
    ("pt", 72.0),
    ("pc", 6.0),
    ("in", 1.0),
    ("cm", 2.54),
    ("mm", 25.4),
    ("q", 101.6),
];

/// A length in px, rounded to hundredths: a dimension in one of the
/// [`LENGTH_UNITS`], or a unitless zero (CSS allows a zero length without a
/// unit, and `0 auto` is how the locked image margin is written).
///
/// - `component`: one value component.
#[must_use]
pub fn length_px(component: &Component) -> Option<Hundredths> {
    match component {
        Component::Number(value) if value.classify() == FpCategory::Zero => Some(Hundredths::ZERO),
        Component::Dimension { value, unit } => {
            let &(name, per_inch) = LENGTH_UNITS
                .iter()
                .find(|(name, _)| *name == unit.as_str())?;
            Hundredths::round(if name == "px" {
                *value
            } else {
                value * 96.0 / per_inch
            })
        }
        _ => None,
    }
}

/// A size in `em`, rounded to hundredths (span `font-size`, editor.md §3.3).
///
/// - `component`: one value component.
#[must_use]
pub fn em(component: &Component) -> Option<Hundredths> {
    match component {
        Component::Dimension { value, unit } if unit == "em" => Hundredths::round(*value),
        _ => None,
    }
}

/// A percentage, rounded to hundredths.
///
/// - `component`: one value component.
#[must_use]
pub fn percentage(component: &Component) -> Option<Hundredths> {
    match component {
        Component::Percentage(value) => Hundredths::round(*value),
        _ => None,
    }
}

/// A number without a unit, rounded to hundredths.
///
/// - `component`: one value component.
#[must_use]
pub fn number(component: &Component) -> Option<Hundredths> {
    match component {
        Component::Number(value) => Hundredths::round(*value),
        _ => None,
    }
}

/// A colour (editor.md §3.1 rule 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Colour {
    /// An sRGB colour with its alpha, each channel from 0 to 255.
    Rgba {
        /// The red channel.
        red: u8,
        /// The green channel.
        green: u8,
        /// The blue channel.
        blue: u8,
        /// The alpha channel; 255 is opaque.
        alpha: u8,
    },
    /// `transparent`, kept as the keyword.
    Transparent,
    /// `currentcolor`, kept as the keyword.
    CurrentColor,
}

impl Colour {
    /// An opaque colour.
    ///
    /// - `red`, `green`, `blue`: the channels.
    #[must_use]
    pub const fn opaque(red: u8, green: u8, blue: u8) -> Self {
        Self::Rgba {
            red,
            green,
            blue,
            alpha: u8::MAX,
        }
    }

    /// Whether the colour hides nothing behind it: an sRGB colour with full
    /// alpha. `transparent` and `currentcolor` are not (span `color`,
    /// editor.md §3.3).
    #[must_use]
    pub const fn is_opaque(self) -> bool {
        matches!(self, Self::Rgba { alpha: u8::MAX, .. })
    }
}

/// Lower-case `#rrggbb`, or `#rrggbbaa` when the alpha is below 1; the two
/// keywords as they are.
impl fmt::Display for Colour {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Rgba {
                red,
                green,
                blue,
                alpha: u8::MAX,
            } => write!(f, "#{red:02x}{green:02x}{blue:02x}"),
            Self::Rgba {
                red,
                green,
                blue,
                alpha,
            } => write!(f, "#{red:02x}{green:02x}{blue:02x}{alpha:02x}"),
            Self::Transparent => f.write_str("transparent"),
            Self::CurrentColor => f.write_str("currentcolor"),
        }
    }
}

/// A colour: a hex colour of 3, 4, 6 or 8 digits, a CSS named colour,
/// `transparent`, `currentcolor`, `windowtext` (the system colour Office
/// writes for black text, read as `#000000`), or an `rgb()` or `rgba()` call.
/// Other system colours and every other function are rejected.
///
/// - `component`: one value component.
#[must_use]
pub fn colour(component: &Component) -> Option<Colour> {
    match component {
        Component::Hash(hex) => hex_colour(hex),
        Component::Ident(name) => match name.as_str() {
            "transparent" => Some(Colour::Transparent),
            "currentcolor" => Some(Colour::CurrentColor),
            "windowtext" => Some(Colour::opaque(0, 0, 0)),
            // The 148 names of CSS Color 4 §6.1, from the cssparser crate
            // that tokenized the value.
            named => cssparser::color::parse_named_color(named)
                .ok()
                .map(|(red, green, blue)| Colour::opaque(red, green, blue)),
        },
        Component::Rgb(arguments) => rgb(arguments),
        _ => None,
    }
}

/// The colour of a hex token without its `#`.
fn hex_colour(hex: &str) -> Option<Colour> {
    let digit = |index: usize| -> Option<u8> {
        char::from(*hex.as_bytes().get(index)?)
            .to_digit(16)
            .and_then(|digit| u8::try_from(digit).ok())
    };
    let short = |index: usize| digit(index).map(|value| value * 17);
    let long = |index: usize| Some(digit(index)? * 16 + digit(index + 1)?);
    let [red, green, blue, alpha] = match hex.len() {
        3 => [short(0)?, short(1)?, short(2)?, u8::MAX],
        4 => [short(0)?, short(1)?, short(2)?, short(3)?],
        6 => [long(0)?, long(2)?, long(4)?, u8::MAX],
        8 => [long(0)?, long(2)?, long(4)?, long(6)?],
        _ => return None,
    };
    Some(Colour::Rgba {
        red,
        green,
        blue,
        alpha,
    })
}

/// The colour of an `rgb()` or `rgba()` call (the two names are aliases in
/// CSS Color 4), in its legacy syntax (`rgb(255, 0, 0)`, `rgba(100%, 0%, 0%,
/// 0.5)`: three numbers or three percentages, separated by commas) or its
/// modern syntax (`rgb(255 0 0 / 50%)`).
///
/// Channels are numbers from 0 to 255 or percentages from 0 to 100, and the
/// alpha value is a number from 0 to 1 or a percentage. A value outside its
/// range is rejected rather than clamped, as browsers would, because no
/// content-v1 writer produces one. Each channel is rounded to a whole
/// number (a percentage as `p × 255 / 100`).
///
/// - `arguments`: the call's arguments, as the declaration reader read them.
#[must_use]
pub fn rgb(arguments: &[Argument]) -> Option<Colour> {
    use Argument::{Comma, Number, Percentage, Slash};
    let (channels, alpha) = match *arguments {
        [r, Comma, g, Comma, b] => ([r, g, b], None),
        [r, Comma, g, Comma, b, Comma, a] => ([r, g, b], Some(a)),
        [r, g, b] if !arguments.contains(&Comma) => ([r, g, b], None),
        [r, g, b, Slash, a] if !arguments.contains(&Comma) => ([r, g, b], Some(a)),
        _ => return None,
    };
    let legacy = arguments.contains(&Comma);
    let all_numbers = channels.iter().all(|c| matches!(c, Number(_)));
    let all_percentages = channels.iter().all(|c| matches!(c, Percentage(_)));
    if legacy && !all_numbers && !all_percentages {
        return None;
    }
    let channel = |argument: Argument| match argument {
        Number(value) if (0.0..=255.0).contains(&value) => to_byte(value),
        Percentage(value) if (0.0..=100.0).contains(&value) => to_byte(value * 255.0 / 100.0),
        _ => None,
    };
    let alpha = match alpha {
        None => u8::MAX,
        Some(Number(value)) if (0.0..=1.0).contains(&value) => to_byte(value * 255.0)?,
        Some(Percentage(value)) if (0.0..=100.0).contains(&value) => {
            to_byte(value * 255.0 / 100.0)?
        }
        Some(_) => return None,
    };
    Some(Colour::Rgba {
        red: channel(channels[0])?,
        green: channel(channels[1])?,
        blue: channel(channels[2])?,
        alpha,
    })
}

/// `value` rounded to a whole number, half away from zero, when it lies
/// from 0 to 255.
fn to_byte(value: f64) -> Option<u8> {
    let rounded = value.round();
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "`rounded` is a whole number from 0 to 255, checked first"
    )]
    (0.0..=255.0).contains(&rounded).then_some(rounded as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::rich_text::css::declarations::read_value;
    use proptest::prelude::*;

    fn component(text: &str) -> Component {
        match read_value(text).as_deref() {
            Some([component]) => component.clone(),
            other => panic!("{text}: {other:?}"),
        }
    }

    fn px(text: &str) -> Option<String> {
        length_px(&component(text)).map(|value| value.to_string())
    }

    fn hex(text: &str) -> Option<String> {
        colour(&component(text)).map(|value| value.to_string())
    }

    #[test]
    fn hundredths_are_written_without_trailing_zeros() {
        let written = |hundredths| Hundredths::from_hundredths(hundredths).to_string();
        assert_eq!(written(2000), "20");
        assert_eq!(written(67), "0.67");
        assert_eq!(written(150), "1.5");
        assert_eq!(written(105), "1.05");
        assert_eq!(written(0), "0");
        assert_eq!(written(5), "0.05");
        assert_eq!(Hundredths::whole(3), Hundredths::from_hundredths(300));
        assert_eq!(Hundredths::whole(3).hundredths(), 300);
    }

    #[test]
    fn rounding_is_half_away_from_zero_and_refuses_what_it_cannot_hold() {
        assert_eq!(
            Hundredths::round(0.125),
            Some(Hundredths::from_hundredths(13))
        );
        assert_eq!(
            Hundredths::round(1.005),
            Some(Hundredths::from_hundredths(100)),
            "1.005 is 1.00499… as an f64"
        );
        assert_eq!(Hundredths::round(20.004), Some(Hundredths::whole(20)));
        assert_eq!(Hundredths::round(-0.0), Some(Hundredths::ZERO));
        assert_eq!(Hundredths::round(-0.001), None);
        assert_eq!(Hundredths::round(f64::NAN), None);
        assert_eq!(Hundredths::round(f64::INFINITY), None);
        assert_eq!(Hundredths::round(1e30), None);
        assert_eq!(
            Hundredths::round(42_949_672.95),
            Some(Hundredths::from_hundredths(u32::MAX))
        );
        assert_eq!(Hundredths::round(42_949_672.96), None);
    }

    #[test]
    fn lengths_become_px() {
        assert_eq!(px("10px").as_deref(), Some("10"));
        assert_eq!(px("10PX").as_deref(), Some("10"));
        assert_eq!(px("15.0pt").as_deref(), Some("20"));
        assert_eq!(px(".5pt").as_deref(), Some("0.67"));
        assert_eq!(px("0.75pt").as_deref(), Some("1"));
        assert_eq!(px("1.0cm").as_deref(), Some("37.8"));
        assert_eq!(px("10mm").as_deref(), Some("37.8"));
        assert_eq!(px("40q").as_deref(), Some("37.8"));
        assert_eq!(px("1in").as_deref(), Some("96"));
        assert_eq!(px("1pc").as_deref(), Some("16"));
        assert_eq!(px("1.234px").as_deref(), Some("1.23"));
        assert_eq!(px("0").as_deref(), Some("0"));
        assert_eq!(px("-0px").as_deref(), Some("0"));
        for rejected in [
            "-1px", "1", "1em", "1rem", "1vw", "10%", "1e30px", "auto", "#fff",
        ] {
            assert_eq!(px(rejected), None, "{rejected}");
        }
    }

    #[test]
    fn sizes_percentages_and_numbers() {
        let size = |text| em(&component(text)).map(|v| v.to_string());
        assert_eq!(size("1.25em").as_deref(), Some("1.25"));
        assert_eq!(size("1.256EM").as_deref(), Some("1.26"));
        assert_eq!(size("1px"), None);
        assert_eq!(size("-1em"), None);
        assert_eq!(percentage(&component("100%")), Some(Hundredths::whole(100)));
        assert_eq!(percentage(&component("100")), None);
        assert_eq!(number(&component("16")), Some(Hundredths::whole(16)));
        assert_eq!(number(&component("16px")), None);
        assert_eq!(number(&component("-1")), None);
        // 0.015 × 100 is 1.5 in double precision and rounds to 2; from
        // cssparser's f32 it would be 1.49999… and round to 1.
        assert_eq!(
            number(&component("0.015")),
            Some(Hundredths::from_hundredths(2))
        );
        assert_eq!(
            length_px(&component("0.015px")),
            Some(Hundredths::from_hundredths(2))
        );
        assert_eq!(
            percentage(&component("0.015%")),
            Some(Hundredths::from_hundredths(2))
        );
    }

    #[test]
    fn colours_have_one_spelling() {
        assert_eq!(hex("#C00000").as_deref(), Some("#c00000"));
        assert_eq!(hex("#abc").as_deref(), Some("#aabbcc"));
        assert_eq!(hex("#abcd").as_deref(), Some("#aabbccdd"));
        assert_eq!(hex("#11223344").as_deref(), Some("#11223344"));
        assert_eq!(hex("#112233ff").as_deref(), Some("#112233"));
        assert_eq!(hex("black").as_deref(), Some("#000000"));
        assert_eq!(hex("WhiteSmoke").as_deref(), Some("#f5f5f5"));
        assert_eq!(hex("rebeccapurple").as_deref(), Some("#663399"));
        assert_eq!(hex("windowtext").as_deref(), Some("#000000"));
        assert_eq!(hex("transparent").as_deref(), Some("transparent"));
        assert_eq!(hex("currentColor").as_deref(), Some("currentcolor"));
        for rejected in [
            "#ab",
            "#abcde",
            "#ggg",
            "#1234567",
            "buttontext",
            "canvas",
            "none",
            "inherit",
            "1px",
            "#é00",
        ] {
            assert_eq!(hex(rejected), None, "{rejected}");
        }
        // CSS Color 4 §6.1 names 148 colours; every one is read.
        assert_eq!(cssparser::color::all_named_colors().count(), 148);
        for (name, (red, green, blue)) in cssparser::color::all_named_colors() {
            assert_eq!(
                colour(&component(name)),
                Some(Colour::opaque(red, green, blue)),
                "{name}"
            );
        }
    }

    #[test]
    fn rgb_calls_in_both_syntaxes() {
        assert_eq!(hex("rgb(192, 0, 0)").as_deref(), Some("#c00000"));
        assert_eq!(hex("rgba(192,0,0,1)").as_deref(), Some("#c00000"));
        assert_eq!(hex("rgba(0, 0, 0, 0.5)").as_deref(), Some("#00000080"));
        assert_eq!(hex("rgb(0, 0, 0, 0)").as_deref(), Some("#00000000"));
        assert_eq!(hex("rgb(100%, 50%, 0%)").as_deref(), Some("#ff8000"));
        assert_eq!(hex("rgb(255 128 0)").as_deref(), Some("#ff8000"));
        assert_eq!(hex("rgb(255 50% 0 / 25%)").as_deref(), Some("#ff800040"));
        assert_eq!(hex("rgb(127.5, 0, 0)").as_deref(), Some("#800000"));
        // 0.7 × 255 is 178.5 in double precision, which rounds to 179 (b3).
        // From cssparser's f32 it would be 178.49999… and round to 178 (b2).
        assert_eq!(hex("rgba(0, 0, 0, 0.7)").as_deref(), Some("#000000b3"));
        assert_eq!(hex("rgba(0, 0, 0, 0.9)").as_deref(), Some("#000000e6"));
        assert_eq!(hex("rgb(0 0 0 / 70%)").as_deref(), Some("#000000b3"));
        // The range is checked on the number as read, not on a rounded one.
        for rejected in [
            "rgb(256, 0, 0)",
            "rgb(255.001, 0, 0)",
            "rgb(-1, 0, 0)",
            "rgb(-0.001, 0, 0)",
            "rgb(101%, 0%, 0%)",
            "rgb(100.001%, 0%, 0%)",
            "rgba(0, 0, 0, 1.5)",
            "rgba(0, 0, 0, 1.001)",
            "rgba(0, 0, 0, -0.001)",
            "rgba(0, 0, 0, 101%)",
            "rgba(0, 0, 0, 100.001%)",
            "rgb(100%, 0, 0)",
            "rgb(0, 0)",
            "rgb(0 0 0 0)",
            "rgb(0, 0 0)",
            "rgb(0 0 0, 1)",
            "rgb(0, 0, 0 / 1)",
            "rgb(0 0 0 / 1 / 1)",
            "rgb()",
        ] {
            assert_eq!(hex(rejected), None, "{rejected}");
        }
    }

    #[test]
    fn opacity() {
        assert!(Colour::opaque(1, 2, 3).is_opaque());
        assert!(
            !Colour::Rgba {
                red: 1,
                green: 2,
                blue: 3,
                alpha: 254
            }
            .is_opaque()
        );
        assert!(!Colour::Transparent.is_opaque());
        assert!(!Colour::CurrentColor.is_opaque());
    }

    proptest! {
        #[test]
        fn hundredths_read_back_as_written(hundredths in 0u32..100_000_000) {
            let value = Hundredths::from_hundredths(hundredths);
            let text = value.to_string();
            prop_assert!(!text.ends_with('0') || !text.contains('.'), "{}", text);
            prop_assert_eq!(Hundredths::round(text.parse::<f64>().unwrap()), Some(value));
        }

        #[test]
        fn rounding_is_monotonic(a in 0.0f64..1e6, b in 0.0f64..1e6) {
            let (low, high) = if a <= b { (a, b) } else { (b, a) };
            prop_assert!(Hundredths::round(low) <= Hundredths::round(high));
        }

        #[test]
        fn px_values_read_back_as_written(hundredths in 0u32..10_000_000) {
            let written = format!("{}px", Hundredths::from_hundredths(hundredths));
            prop_assert_eq!(length_px(&component(&written)), Some(Hundredths::from_hundredths(hundredths)));
        }

        #[test]
        fn points_are_four_thirds_of_a_pixel(points in 0u32..100_000) {
            let expected = Hundredths::round(f64::from(points) * 96.0 / 72.0);
            prop_assert_eq!(length_px(&component(&format!("{points}pt"))), expected);
        }

        #[test]
        fn negative_lengths_are_rejected(value in 0.001f64..1e6, unit in prop::sample::select(&["px", "pt", "cm", "em"][..])) {
            let text = format!("-{value}{unit}");
            prop_assert_eq!(length_px(&component(&text)), None);
            prop_assert_eq!(em(&component(&text)), None);
        }

        #[test]
        fn hex_colours_read_back_as_written(red: u8, green: u8, blue: u8, alpha: u8) {
            let colour = Colour::Rgba { red, green, blue, alpha };
            let written = colour.to_string();
            prop_assert_eq!(written.len(), if alpha == u8::MAX { 7 } else { 9 });
            prop_assert_eq!(super::colour(&component(&written)), Some(colour));
            prop_assert_eq!(super::colour(&component(&written.to_uppercase())), Some(colour));
            let call = format!("rgba({red}, {green}, {blue}, {})", f64::from(alpha) / 255.0);
            prop_assert_eq!(super::colour(&component(&call)), Some(colour));
            prop_assert_eq!(colour.is_opaque(), alpha == u8::MAX);
            prop_assert_eq!(Colour::opaque(red, green, blue), Colour::Rgba { red, green, blue, alpha: u8::MAX });
        }

        #[test]
        fn whole_numbers_are_a_hundred_hundredths(value in 0u32..=u32::MAX / 100) {
            prop_assert_eq!(Hundredths::whole(value).hundredths(), value * 100);
            prop_assert_eq!(Hundredths::whole(value).to_string(), value.to_string());
        }

        #[test]
        fn sizes_percentages_and_numbers_read_back_as_written(hundredths in 0u32..10_000_000) {
            let value = Hundredths::from_hundredths(hundredths);
            prop_assert_eq!(em(&component(&format!("{value}em"))), Some(value));
            prop_assert_eq!(em(&component(&format!("{value}px"))), None);
            prop_assert_eq!(percentage(&component(&format!("{value}%"))), Some(value));
            prop_assert_eq!(number(&component(&value.to_string())), Some(value));
            prop_assert_eq!(number(&component(&format!("{value}%"))), None);
        }

        #[test]
        fn named_colours_match_in_any_ascii_case(index in 0usize..148, upper in prop::collection::vec(any::<bool>(), 20)) {
            let named: Vec<_> = cssparser::color::all_named_colors().collect();
            let (name, (red, green, blue)) = named[index % named.len()];
            let mixed: String = name.chars().zip(upper.iter().cycle()).map(|(c, &up)| if up { c.to_ascii_uppercase() } else { c }).collect();
            prop_assert_eq!(super::colour(&component(&mixed)), Some(Colour::opaque(red, green, blue)));
        }

        #[test]
        fn rgb_accepts_exactly_the_channel_range(value in -10.0f64..300.0) {
            let accepted = super::colour(&component(&format!("rgb({value}, 0, 0)")));
            prop_assert_eq!(accepted.is_some(), (0.0..=255.0).contains(&value));
        }
    }
}
