//! One shared "pick one of N, then run a command with the chosen value"
//! dialog host for every GridCraft format command the Home tab's palette
//! entry would otherwise show disabled with a "needs …" reason: this app
//! has no input surface for a required `name`/`size`/`color`/`preset`/
//! `edge`/`format` parameter.
//!
//! Hosts [`quadraui::MessageDialogController`] — the cross-backend
//! "show a `Dialog`, resolve on Enter/click, Escape cancels" composition
//! quadraui already ships — over a short built-in list of choices per
//! command id, and maps whichever choice is picked back onto the params
//! `CalcApp::handle` runs the command with. One composition covers every
//! wired id instead of a bespoke dialog per command: font name and size
//! pick from a short built-in list (no live system font enumeration),
//! fill/font color pick from a fixed theme-color grid (no color wheel),
//! borders/draw-border pick from a handful of named presets, and number
//! format picks from a handful of named codes — all the same
//! "list of labelled buttons, Enter/click resolves" shape.
//!
//! [`palette`](crate::palette) treats every id [`choices_for`] knows
//! about as enabled (it can supply the parameter, just not up front),
//! and returns [`crate::palette::Outcome::Pick`] instead of `Run` so
//! `app.rs` opens a [`FormatPicker`] in place of running the command
//! directly.

use quadraui::{
    Backend, Dialog, Handled, Key, MessageDialogButton, MessageDialogController,
    MessageDialogOptions, NamedKey, UiEvent, WidgetId,
};
use serde_json::{Value, json};

/// One selectable choice: its button label and the params merged into
/// the command run when it's picked. `range`/`cell` is never one of
/// these — like every other palette-run command, the picker's caller
/// (`app.rs`) already resolves that from the current selection via
/// `CalcApp::sync_selection` before executing.
struct Choice {
    label: &'static str,
    params: Value,
}

/// `Some` (dialog title, choices) for every command id this module can
/// supply a parameter for; `None` for everything else, including ids the
/// palette's own `unsuppliable_params` heuristic would accept as-is.
fn spec_for(id: &str) -> Option<(&'static str, Vec<Choice>)> {
    let choices = match id {
        "home.fontName" => ("Font", font_names()),
        "home.fontSize" => ("Font Size", font_sizes()),
        "home.fontColor" => ("Font Color", colors("color", Some(("Automatic", "auto")))),
        "home.fillColor" => ("Fill Color", colors("color", Some(("No Fill", "none")))),
        "home.borders" => ("Borders", border_presets()),
        "home.drawBorder" => ("Draw Border", draw_border_edges()),
        "home.numberFormat" => ("Number Format", number_formats()),
        _ => return None,
    };
    Some(choices)
}

/// Whether [`spec_for`] knows `id` — all [`crate::palette`] needs, since
/// it never looks at the choices themselves.
pub fn has_picker(id: &str) -> bool {
    spec_for(id).is_some()
}

fn font_names() -> Vec<Choice> {
    [
        "Calibri",
        "Arial",
        "Times New Roman",
        "Courier New",
        "Georgia",
        "Verdana",
    ]
    .into_iter()
    .map(|name| Choice {
        label: name,
        params: json!({ "name": name }),
    })
    .collect()
}

fn font_sizes() -> Vec<Choice> {
    [8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 18.0, 24.0, 36.0, 48.0]
        .into_iter()
        .map(|size| Choice {
            label: match size {
                8.0 => "8",
                9.0 => "9",
                10.0 => "10",
                11.0 => "11",
                12.0 => "12",
                14.0 => "14",
                18.0 => "18",
                24.0 => "24",
                36.0 => "36",
                _ => "48",
            },
            params: json!({ "size": size }),
        })
        .collect()
}

/// A fixed theme-color grid, plus one command-specific "clear it" entry
/// (`extra`'s label and the engine's own `color_param` value for it —
/// `"auto"` for font color, `"none"` for fill color, both of which
/// `color_param` reads identically as [`gridcraft_engine`]'s automatic
/// color).
fn colors(field: &str, extra: Option<(&'static str, &'static str)>) -> Vec<Choice> {
    let mut choices: Vec<Choice> = extra
        .into_iter()
        .map(|(label, value)| Choice {
            label,
            params: json!({ field: value }),
        })
        .collect();
    choices.extend(
        [
            ("Black", "#000000"),
            ("White", "#FFFFFF"),
            ("Red", "#C00000"),
            ("Orange", "#ED7D31"),
            ("Yellow", "#FFC000"),
            ("Green", "#70AD47"),
            ("Blue", "#4472C4"),
            ("Purple", "#7030A0"),
        ]
        .into_iter()
        .map(|(label, hex)| Choice {
            label,
            params: json!({ field: hex }),
        }),
    );
    choices
}

fn border_presets() -> Vec<Choice> {
    [
        ("All", "all"),
        ("Outline", "outside"),
        ("Thick Box", "thickOutside"),
        ("Inside", "inside"),
        ("None", "none"),
    ]
    .into_iter()
    .map(|(label, preset)| Choice {
        label,
        params: json!({ "preset": preset }),
    })
    .collect()
}

fn draw_border_edges() -> Vec<Choice> {
    [
        ("Top Edge", "top"),
        ("Bottom Edge", "bottom"),
        ("Left Edge", "left"),
        ("Right Edge", "right"),
        ("Outline", "outline"),
        ("All (Grid)", "grid"),
    ]
    .into_iter()
    .map(|(label, edge)| Choice {
        label,
        params: json!({ "edge": edge }),
    })
    .collect()
}

fn number_formats() -> Vec<Choice> {
    let named = [
        ("General", "General"),
        ("Number", "Number"),
        ("Currency", "Currency"),
        ("Percentage", "Percentage"),
        ("Short Date", "Short Date"),
        ("Long Date", "Long Date"),
    ]
    .into_iter()
    .map(|(label, format)| Choice {
        label,
        params: json!({ "format": format }),
    });
    let coded = [
        ("Number, no decimals", "0"),
        ("Currency, no decimals", "\"$\"#,##0"),
        ("Percentage, no decimals", "0%"),
    ]
    .into_iter()
    .map(|(label, code)| Choice {
        label,
        params: json!({ "code": code }),
    });
    named.chain(coded).collect()
}

/// What picking (or cancelling) did, mirroring [`crate::palette::Outcome`]'s
/// own shape so `app.rs` routes both the same way.
pub enum PickerOutcome {
    /// Keyboard focus moved among the choices — redraw, picker stays open.
    Changed,
    /// Escape, a click outside the dialog, or the explicit Cancel choice:
    /// close the picker with no further action.
    Closed,
    /// A choice was confirmed: run this id with these params (merged
    /// with whatever `range`/`cell` the caller resolves) and close the
    /// picker.
    Run(&'static str, Value),
    /// The event meant nothing to the picker.
    Ignored,
}

/// Open, modal picker state for one command id's [`spec_for`] choices.
/// `app.rs` holds one of these in `Some` exactly like
/// [`crate::palette::CommandPalette`], feeding it every event while
/// it's open.
pub struct FormatPicker {
    command_id: &'static str,
    controller: MessageDialogController,
    /// `choices[i].1` is the params for `ids[i]` (same index as the
    /// `MessageDialogButton`s `controller` was built from, before the
    /// trailing Cancel button); `cancel_id` is that Cancel button's id.
    ids: Vec<WidgetId>,
    params: Vec<Value>,
    cancel_id: WidgetId,
}

impl FormatPicker {
    /// `None` when `id` has no [`spec_for`] entry — the caller
    /// (`app.rs`) only calls this after `palette::Outcome::Pick`, which
    /// only ever carries an id [`has_picker`] already confirmed `Some`
    /// for, so this should never actually return `None` in practice.
    pub fn open(id: &'static str) -> Option<Self> {
        let (title, choices) = spec_for(id)?;
        let ids: Vec<WidgetId> = (0..choices.len())
            .map(|i| WidgetId::new(format!("quadracalc-format-picker:{id}:{i}")))
            .collect();
        let cancel_id = WidgetId::new(format!("quadracalc-format-picker:{id}:cancel"));
        let mut buttons: Vec<MessageDialogButton> = ids
            .iter()
            .zip(&choices)
            .enumerate()
            .map(|(i, (&wid, choice))| MessageDialogButton {
                id: wid,
                label: choice.label.to_string(),
                is_default: i == 0,
                is_cancel: false,
                spec: None,
            })
            .collect();
        buttons.push(MessageDialogButton {
            id: cancel_id,
            label: "Cancel".to_string(),
            is_default: false,
            is_cancel: true,
            spec: None,
        });
        let controller = MessageDialogController::new(MessageDialogOptions {
            title: title.to_string(),
            body: String::new(),
            buttons,
            severity: None,
        })
        .with_vertical_buttons(true);
        Some(Self {
            command_id: id,
            controller,
            ids,
            params: choices.into_iter().map(|c| c.params).collect(),
            cancel_id,
        })
    }

    /// The dialog descriptor this picker is currently showing — mostly
    /// useful to tests; `app.rs` paints through [`Self::render`]
    /// instead, since [`MessageDialogController`] computes its own
    /// backend-specific [`quadraui::DialogLayout`] from private fields
    /// `app.rs` has no way to recompute itself.
    pub fn dialog(&self) -> &Dialog {
        self.controller.dialog()
    }

    /// Paint this frame's dialog, centred in `backend`'s viewport.
    /// Called directly from `AppLogic::render`, the same contract every
    /// other quadraui `compose` controller (`MessageDialogController`'s
    /// own module doc: "Call this from `AppLogic::render`") documents —
    /// unlike `CommandPalette`, a hand-rolled composition `app.rs` owns
    /// end to end, this one is quadraui's, with no public way to get its
    /// `DialogLayout` out to push through `ScreenLayout` instead.
    pub fn render(&self, backend: &mut dyn Backend) {
        self.controller.render(backend);
    }

    /// Feed one event to the picker. Up/Down are remapped to Left/Right
    /// first — [`MessageDialogController`]'s own keyboard model cycles
    /// button focus on Left/Right/Tab (it's written for a horizontal
    /// button row), but every choice list here renders
    /// `vertical_buttons`, where Up/Down reads as the natural "next
    /// choice" key.
    pub fn handle_event(&mut self, event: &UiEvent, backend: &dyn Backend) -> PickerOutcome {
        let remapped = remap_vertical_nav(event);
        let event = remapped.as_ref().unwrap_or(event);
        match self.controller.handle_event(event, backend) {
            Handled::Event(chosen) => {
                if chosen == self.cancel_id {
                    return PickerOutcome::Closed;
                }
                match self.ids.iter().position(|&wid| wid == chosen) {
                    Some(i) => PickerOutcome::Run(self.command_id, self.params[i].clone()),
                    // Not one of ours — shouldn't happen, but closing is
                    // safer than silently running the wrong params.
                    None => PickerOutcome::Closed,
                }
            }
            Handled::Consumed => PickerOutcome::Changed,
            Handled::Ignored => PickerOutcome::Ignored,
        }
    }
}

/// `Some(Left/Right-shaped copy of event)` for an Up/Down `KeyPressed`,
/// `None` for everything else (the caller falls back to the original
/// event unchanged).
fn remap_vertical_nav(event: &UiEvent) -> Option<UiEvent> {
    let UiEvent::KeyPressed {
        key: Key::Named(named),
        modifiers,
        repeat,
    } = event
    else {
        return None;
    };
    let mapped = match named {
        NamedKey::Up => NamedKey::Left,
        NamedKey::Down => NamedKey::Right,
        _ => return None,
    };
    Some(UiEvent::KeyPressed {
        key: Key::Named(mapped),
        modifiers: *modifiers,
        repeat: *repeat,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use quadraui::Modifiers;

    #[test]
    fn every_wired_id_has_at_least_one_choice_plus_a_default() {
        for id in [
            "home.fontName",
            "home.fontSize",
            "home.fontColor",
            "home.fillColor",
            "home.borders",
            "home.drawBorder",
            "home.numberFormat",
        ] {
            let (_, choices) = spec_for(id).unwrap_or_else(|| panic!("{id} has no picker"));
            assert!(!choices.is_empty(), "{id} has no choices");
        }
    }

    #[test]
    fn unrelated_ids_have_no_picker() {
        assert!(!has_picker("home.bold"));
        assert!(!has_picker("file.open"));
    }

    #[test]
    fn opening_seeds_a_dialog_with_one_button_per_choice_plus_cancel() {
        let picker = FormatPicker::open("home.fontSize").unwrap();
        // 10 sizes + Cancel.
        assert_eq!(picker.dialog().buttons.len(), 11);
        assert_eq!(picker.dialog().title.spans[0].text, "Font Size");
    }

    #[test]
    fn modifiers_round_trip_through_remap_vertical_nav() {
        let shift = Modifiers {
            shift: true,
            ..Default::default()
        };
        let down = UiEvent::KeyPressed {
            key: Key::Named(NamedKey::Down),
            modifiers: shift,
            repeat: true,
        };
        let remapped = remap_vertical_nav(&down).unwrap();
        assert_eq!(
            remapped,
            UiEvent::KeyPressed {
                key: Key::Named(NamedKey::Right),
                modifiers: shift,
                repeat: true,
            }
        );
    }
}
