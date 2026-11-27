//! Terminal screenshots of the TUI, rendered through quadraui's own
//! `TuiDriver` (the production render path into a headless ratatui
//! `TestBackend`) and written as SVG — one `<rect>` per background run and
//! one `<text>` per foreground run, the same shape `quadraui-gallery`'s
//! `capture.rs::write_tui_svg` emits, so its SVG→PNG rasteriser works on
//! these unchanged.
//!
//! `#[ignore]`d: this writes files rather than asserting anything. Run it
//! with
//!
//! ```text
//! QUADRACALC_SCREENSHOTS=docs/screenshots cargo test -p quadracalc-core \
//!     --test screenshots -- --ignored
//! ```
//!
//! from the workspace root. Without `QUADRACALC_SCREENSHOTS` it prints
//! each screen as plain text instead.

use std::fmt::Write as _;
use std::path::PathBuf;

use quadraui::tui::testing::{Color, Modifier, TuiDriver};
use quadraui::{Key, Modifiers, NamedKey, UiEvent};

use quadracalc_core::CalcApp;

const W: u16 = 100;
const H: u16 = 30;
const CELL_W: u32 = 9;
const CELL_H: u32 = 18;

fn ansi_rgb(c: Color) -> Option<(u8, u8, u8)> {
    const BASIC16: [(u8, u8, u8); 16] = [
        (12, 12, 12),
        (205, 49, 49),
        (13, 188, 121),
        (229, 229, 16),
        (36, 114, 200),
        (188, 63, 188),
        (17, 168, 205),
        (204, 204, 204),
        (102, 102, 102),
        (241, 76, 76),
        (35, 209, 139),
        (245, 245, 67),
        (59, 142, 234),
        (214, 112, 214),
        (41, 184, 219),
        (242, 242, 242),
    ];
    Some(match c {
        Color::Reset => return None,
        Color::Black => BASIC16[0],
        Color::Red => BASIC16[1],
        Color::Green => BASIC16[2],
        Color::Yellow => BASIC16[3],
        Color::Blue => BASIC16[4],
        Color::Magenta => BASIC16[5],
        Color::Cyan => BASIC16[6],
        Color::Gray => BASIC16[7],
        Color::DarkGray => BASIC16[8],
        Color::LightRed => BASIC16[9],
        Color::LightGreen => BASIC16[10],
        Color::LightYellow => BASIC16[11],
        Color::LightBlue => BASIC16[12],
        Color::LightMagenta => BASIC16[13],
        Color::LightCyan => BASIC16[14],
        Color::White => BASIC16[15],
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(i) if i < 16 => BASIC16[i as usize],
        Color::Indexed(i) if i >= 232 => {
            let v = 8 + (i - 232) * 10;
            (v, v, v)
        }
        Color::Indexed(i) => {
            let i = i - 16;
            let level = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
            (level(i / 36), level((i / 6) % 6), level(i % 6))
        }
    })
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn svg(driver: &TuiDriver<CalcApp>) -> String {
    let (px_w, px_h) = (u32::from(W) * CELL_W, u32::from(H) * CELL_H);
    let mut svg = String::new();
    let _ = writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{px_w}\" height=\"{px_h}\" \
         viewBox=\"0 0 {px_w} {px_h}\" font-family=\"'SF Mono','DejaVu Sans Mono',monospace\" \
         font-size=\"{}\" xml:space=\"preserve\">",
        CELL_H - 4
    );
    svg.push_str("<rect width=\"100%\" height=\"100%\" fill=\"#101010\"/>\n");
    for y in 0..H {
        let row = driver.styled_row(y);
        let mut x = 0;
        while x < row.len() {
            let bg = row[x].1.bg;
            let mut end = x + 1;
            while end < row.len() && row[end].1.bg == bg {
                end += 1;
            }
            if let Some((r, g, b)) = ansi_rgb(bg) {
                let _ = writeln!(
                    svg,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{CELL_H}\" fill=\"#{r:02x}{g:02x}{b:02x}\"/>",
                    x as u32 * CELL_W,
                    u32::from(y) * CELL_H,
                    (end - x) as u32 * CELL_W,
                );
            }
            x = end;
        }
        let mut x = 0;
        while x < row.len() {
            let s0 = row[x].1;
            let mut end = x + 1;
            while end < row.len() && row[end].1.fg == s0.fg && row[end].1.modifiers == s0.modifiers
            {
                end += 1;
            }
            let text: String = row[x..end].iter().map(|(c, _)| *c).collect();
            if !text.trim().is_empty() {
                let (r, g, b) = ansi_rgb(s0.fg).unwrap_or((230, 230, 230));
                let weight = if s0.modifiers.contains(Modifier::BOLD) {
                    " font-weight=\"bold\""
                } else {
                    ""
                };
                let _ = writeln!(
                    svg,
                    "<text x=\"{}\" y=\"{}\" textLength=\"{}\" lengthAdjust=\"spacingAndGlyphs\" \
                     fill=\"#{r:02x}{g:02x}{b:02x}\"{weight}>{}</text>",
                    x as u32 * CELL_W,
                    u32::from(y) * CELL_H + CELL_H - 4,
                    (end - x) as u32 * CELL_W,
                    xml_escape(&text),
                );
            }
            x = end;
        }
    }
    svg.push_str("</svg>\n");
    svg
}

fn capture(driver: &TuiDriver<CalcApp>, name: &str) {
    match std::env::var_os("QUADRACALC_SCREENSHOTS") {
        Some(dir) => {
            let mut dir = PathBuf::from(dir);
            if dir.is_relative() {
                // `cargo test` runs from the crate dir; resolve against
                // the workspace root instead.
                dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../..")
                    .join(dir);
            }
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(format!("{name}.svg")), svg(driver)).unwrap();
            std::fs::write(dir.join(format!("{name}.txt")), driver.screen()).unwrap();
        }
        None => println!("=== {name} ===\n{}", driver.screen()),
    }
}

fn key(key: Key, modifiers: Modifiers) -> UiEvent {
    UiEvent::KeyPressed {
        key,
        modifiers,
        repeat: false,
    }
}

fn ctrl_char(c: char) -> UiEvent {
    key(
        Key::Char(c),
        Modifiers {
            ctrl: true,
            ..Default::default()
        },
    )
}

fn shift() -> Modifiers {
    Modifiers {
        shift: true,
        ..Default::default()
    }
}

fn driver() -> TuiDriver<CalcApp> {
    TuiDriver::new(CalcApp::with_sample().unwrap(), W, H)
}

fn alt(c: char) -> UiEvent {
    key(
        Key::Char(c),
        Modifiers {
            alt: true,
            ..Default::default()
        },
    )
}

fn at(d: &TuiDriver<CalcApp>, needle: &str) -> (f32, f32) {
    d.find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not on screen\n{}", d.screen()))
}

fn go(d: &mut TuiDriver<CalcApp>, down: usize, right: usize) {
    for _ in 0..down {
        d.press_named(NamedKey::Down);
    }
    for _ in 0..right {
        d.press_named(NamedKey::Right);
    }
}

#[test]
#[ignore = "writes screenshots; run explicitly"]
fn capture_screenshots() {
    // Startup: the sample sheet as it first opens.
    let d = driver();
    capture(&d, "01-startup");

    // A selected range: the status bar's Average/Count/Sum.
    let mut d = driver();
    go(&mut d, 10, 3);
    for _ in 0..11 {
        d.dispatch(key(Key::Named(NamedKey::Down), shift()));
    }
    capture(&d, "02-selection");

    // Editing a formula in place: the cell and the formula bar share
    // one edit.
    let mut d = driver();
    go(&mut d, 10, 8);
    d.press_named(NamedKey::F(2));
    capture(&d, "03-edit-formula");

    // Find across the sheet.
    let mut d = driver();
    d.dispatch(ctrl_char('f'));
    for c in "yes".chars() {
        d.type_char(c);
    }
    d.press_named(NamedKey::Enter);
    d.press_named(NamedKey::Enter);
    capture(&d, "04-find");

    // Find and replace: replace mode looks in what was typed, so the
    // quoted "no" is the IF's own label, and Replace All rewrites it in
    // every Over target? formula.
    let mut d = driver();
    d.dispatch(ctrl_char('h'));
    for c in "\"no\"".chars() {
        d.type_char(c);
    }
    d.press_named(NamedKey::Tab);
    for c in "\"not yet\"".chars() {
        d.type_char(c);
    }
    d.dispatch(key(
        Key::Named(NamedKey::Enter),
        Modifiers {
            ctrl: true,
            ..Default::default()
        },
    ));
    capture(&d, "05-replace");

    // The grid's right-click menu.
    let mut d = driver();
    let (x, y) = at(&d, "$4,525.00");
    d.dispatch(UiEvent::MouseDown {
        widget: None,
        button: quadraui::MouseButton::Right,
        position: quadraui::Point::new(x, y),
        modifiers: Modifiers::default(),
    });
    capture(&d, "06-context-menu");

    // The menu bar, Home › Number open.
    let mut d = driver();
    d.dispatch(alt('h'));
    let (x, y) = at(&d, "Number");
    d.click(x, y);
    capture(&d, "07-menu");

    // The command palette, filtered.
    let mut d = driver();
    d.type_char(':');
    for c in "freeze".chars() {
        d.type_char(c);
    }
    capture(&d, "08-palette");

    // Scrolled down and right with the wheel: column A stays frozen.
    let mut d = driver();
    let (x, y) = at(&d, "Hosting");
    d.dispatch(UiEvent::Scroll {
        widget: None,
        delta: quadraui::ScrollDelta::new(2.0, -3.0),
        position: quadraui::Point::new(x, y),
    });
    capture(&d, "09-frozen-panes");
}
