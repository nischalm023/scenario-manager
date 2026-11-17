//! Within `crucible-viewer`, `palette` is THE COLOURS THE VIEWER CHOOSES
//! ITSELF: the ground a floor is drawn on and the lines drawn over it (spec 03
//! section 3.6's `set_palette`), each kept as the CSS a canvas strokes with
//! and the four numbers a GPU clears to.
//!
//! It exists because a page's colours do not reach a canvas by themselves:
//! not its tokens, and under `(forced-colors: active)` not the system colours
//! either (reviews/accessibility.md finding 4). The page resolves whichever
//! apply and hands them in, and this takes a system colour as it takes any
//! other. The cells keep the renderer's own colours (`worldview-render`'s
//! `grid.wgsl`, desvis's palette); only what this crate draws follows a page.

/// One colour, as given and as numbers.
#[derive(Clone, Debug, PartialEq)]
pub struct Colour {
    /// The text it was given, which a 2D canvas reads exactly as the page did.
    pub css: String,
    /// Red, green, blue and alpha from 0 to 1, still sRGB-encoded: what a
    /// clear of a surface that is not sRGB writes as it stands.
    pub rgba: [f64; 4],
}

/// What each colour is for.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    /// The ground under and around the cells, and the halo either side of
    /// every line, which keeps a line's contrast whatever it crosses.
    pub floor: Colour,
    /// The frame's axes and names, and the centre cross.
    pub lines: Colour,
    /// What the agent highlighted.
    pub agent: Colour,
    /// The one selected thing.
    pub selection: Colour,
}

impl Default for Palette {
    /// desvis's own ground, so a viewer no page has coloured opens as a
    /// recording's always has, and spec 02's light tokens for the lines.
    fn default() -> Self {
        let of = |css: &str| Colour::parse(css).expect("the defaults parse");
        Self { floor: of("#f3f5f8"), lines: of("#181d27"), agent: of("#6941c6"), selection: of("#0a4a5e") }
    }
}

impl Colour {
    /// Read a CSS colour in the forms a computed style or a 2D canvas writes
    /// one: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb(…)` or `rgba(…)`,
    /// with commas or with spaces and a slash. A name, `currentColor` or a
    /// system keyword is resolved by the page first, so it is refused here.
    pub fn parse(css: &str) -> Result<Self, String> {
        let text = css.trim().to_ascii_lowercase();
        let rgba = match (text.strip_prefix('#'), text.strip_suffix(')')) {
            (Some(hex), _) => hexadecimal(hex),
            (None, Some(call)) => call.split_once('(').and_then(|(name, args)| functional(name.trim(), args)),
            (None, None) => None,
        };
        let rgba = rgba.ok_or_else(|| {
            format!("`{css}` is not a colour the viewer reads; give #rrggbb or rgb(…) as a computed style writes it")
        })?;
        Ok(Self { css: css.trim().to_owned(), rgba })
    }
}

fn hexadecimal(hex: &str) -> Option<[f64; 4]> {
    let digits: Vec<u32> = hex.chars().map(|digit| digit.to_digit(16)).collect::<Option<_>>()?;
    let channels: Vec<u32> = match digits.len() {
        3 | 4 => digits.iter().map(|digit| digit * 17).collect(),
        6 | 8 => digits.chunks(2).map(|pair| pair[0] * 16 + pair[1]).collect(),
        _ => return None,
    };
    let alpha = channels.get(3).map_or(1.0, |&alpha| f64::from(alpha) / 255.0);
    Some([f64::from(channels[0]) / 255.0, f64::from(channels[1]) / 255.0, f64::from(channels[2]) / 255.0, alpha])
}

fn functional(name: &str, args: &str) -> Option<[f64; 4]> {
    if name != "rgb" && name != "rgba" {
        return None;
    }
    let (colour, alpha) = match args.split_once('/') {
        Some((colour, alpha)) => (colour, Some(alpha)),
        None => (args, None),
    };
    let mut parts: Vec<&str> = colour.split([',', ' ']).filter(|part| !part.trim().is_empty()).collect();
    let alpha = match (alpha, parts.len()) {
        (Some(alpha), 3) => alpha,
        (None, 4) => parts.pop()?,
        (None, 3) => "1",
        _ => return None,
    };
    let channel = |part: &str| match part.trim().strip_suffix('%') {
        Some(percent) => percent.parse::<f64>().ok().map(|value| value / 100.0),
        None => part.trim().parse::<f64>().ok().map(|value| value / 255.0),
    };
    let alpha = match alpha.trim().strip_suffix('%') {
        Some(percent) => percent.parse::<f64>().ok()? / 100.0,
        None => alpha.trim().parse::<f64>().ok()?,
    };
    let [r, g, b] = [channel(parts[0])?, channel(parts[1])?, channel(parts[2])?];
    Some([r, g, b, alpha].map(|value| value.clamp(0.0, 1.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The forms a computed style and a canvas write a colour in all read,
    /// including the opaque system colours a forced-colours page resolves.
    #[test]
    fn computed_and_canvas_forms_read_and_keywords_are_refused() {
        let rgba = |css: &str| Colour::parse(css).unwrap().rgba;
        assert_eq!(rgba("#6941c6"), [105.0 / 255.0, 65.0 / 255.0, 198.0 / 255.0, 1.0]);
        assert_eq!(rgba("#fff"), [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(rgba("#00000080")[3], 128.0 / 255.0);
        assert_eq!(rgba("rgb(0, 102, 204)"), [0.0, 0.4, 0.8, 1.0]);
        assert_eq!(rgba("rgba(0, 0, 0, 0.5)"), [0.0, 0.0, 0.0, 0.5]);
        assert_eq!(rgba("rgb(255 255 255 / 25%)"), [1.0, 1.0, 1.0, 0.25]);
        assert_eq!(Colour::parse(" #EEF0F3 ").unwrap().css, "#EEF0F3", "the text a canvas is given is the page's own");
        for refused in ["Highlight", "currentColor", "#12345", "hsl(0 0% 0%)", "rgb(1, 2)", ""] {
            assert!(Colour::parse(refused).is_err(), "{refused}");
        }
    }
}
