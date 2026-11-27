//! A cell's GridCraft style (the `style` object `cell.get` returns) as
//! the [`CellStyle`] quadraui's [`quadraui::CellGrid`] paints: bold,
//! italic, underline, an explicit font colour, a solid fill and the
//! horizontal alignment.
//!
//! Only [`gridcraft_engine::model::Color::Rgb`] resolves to a paintable
//! colour; `Auto` and a theme-tinted `Theme(..)` come back `None` (inherit
//! the grid's default) — resolving a theme tint needs the workbook's
//! theme palette, which has no `Session::execute` query to read it back
//! through.
//!
//! A filled cell with no font colour of its own gets one picked for
//! contrast against the fill. The grid's default foreground is the
//! *terminal's* (or window theme's), which is light on a dark theme — so a
//! pale fill a workbook authored on a white sheet would otherwise paint
//! white-on-pale and be unreadable.

use quadraui::{CellAlign, CellStyle, Color};
use serde_json::Value;

/// `style` (`cell.get`'s `"style"` field, or `None` for an unstyled cell)
/// as a [`CellStyle`].
pub fn from_engine(style: Option<&Value>) -> CellStyle {
    let font = style.and_then(|s| s.get("font"));
    let bold = font.and_then(|f| f.get("bold")).and_then(Value::as_bool);
    let italic = font
        .and_then(|f| f.get("italic"))
        .and_then(Value::as_bool)
        .filter(|on| *on);
    let underline = font
        .and_then(|f| f.get("underline"))
        .and_then(Value::as_str)
        .map(|u| u != "None")
        .filter(|on| *on);
    let fg = font.and_then(|f| f.get("color")).and_then(rgb);
    let bg = style
        .and_then(|s| s.get("fill"))
        .filter(|fill| fill.get("pattern").and_then(Value::as_str) == Some("Solid"))
        .and_then(|fill| fill.get("fg"))
        .and_then(rgb);
    let align = style
        .and_then(|s| s.get("align"))
        .and_then(|a| a.get("h"))
        .and_then(Value::as_str)
        .and_then(|h| match h {
            "Left" => Some(CellAlign::Left),
            "Center" | "CenterAcross" => Some(CellAlign::Center),
            "Right" => Some(CellAlign::Right),
            _ => None,
        });
    let fg = fg.or_else(|| bg.map(contrasting_fg));
    CellStyle {
        bold,
        italic,
        underline,
        fg,
        bg,
        align,
    }
}

/// `{"Rgb": 0xRRGGBB}` as a [`Color`]; anything else (`"Auto"`, a theme
/// colour) as `None`.
fn rgb(color: &Value) -> Option<Color> {
    let rgb = color.get("Rgb").and_then(Value::as_u64)?;
    Some(Color::rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8))
}

/// Near-black on a light fill, white on a dark one — by the fill's
/// relative luminance (sRGB weights), the same split a spreadsheet's own
/// "Automatic" font colour makes.
pub fn contrasting_fg(bg: Color) -> Color {
    let luma = 0.2126 * f32::from(bg.r) + 0.7152 * f32::from(bg.g) + 0.0722 * f32::from(bg.b);
    if luma > 140.0 {
        Color::rgb(20, 20, 20)
    } else {
        Color::rgb(255, 255, 255)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_unstyled_cell_inherits_everything() {
        assert_eq!(from_engine(None), CellStyle::default());
    }

    #[test]
    fn font_fill_and_alignment_map_across() {
        let style = json!({
            "font": {"bold": true, "italic": true, "underline": "Single", "color": {"Rgb": 0xFF0000}},
            "fill": {"pattern": "Solid", "fg": {"Rgb": 0x1F4E79}},
            "align": {"h": "Center"},
        });
        let s = from_engine(Some(&style));
        assert_eq!(s.bold, Some(true));
        assert_eq!(s.italic, Some(true));
        assert_eq!(s.underline, Some(true));
        assert_eq!(s.fg, Some(Color::rgb(255, 0, 0)));
        assert_eq!(s.bg, Some(Color::rgb(0x1F, 0x4E, 0x79)));
        assert_eq!(s.align, Some(CellAlign::Center));
    }

    #[test]
    fn a_fill_with_no_font_colour_gets_a_readable_one() {
        let pale = json!({"fill": {"pattern": "Solid", "fg": {"Rgb": 0xD9E8F5}}});
        assert_eq!(from_engine(Some(&pale)).fg, Some(Color::rgb(20, 20, 20)));
        let dark = json!({"fill": {"pattern": "Solid", "fg": {"Rgb": 0x1F4E79}}});
        assert_eq!(from_engine(Some(&dark)).fg, Some(Color::rgb(255, 255, 255)));
    }

    #[test]
    fn theme_and_auto_colours_inherit() {
        let style = json!({"font": {"color": "Auto"}, "fill": {"pattern": "Solid", "fg": {"Theme": [4, 0]}}});
        let s = from_engine(Some(&style));
        assert_eq!(s.fg, None);
        assert_eq!(s.bg, None);
    }
}
