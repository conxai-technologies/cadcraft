//! Embed mode's own Layers and Properties panels, drawn by the shell beside the drawing in
//! place of the app's palettes. A viewer needs less than the palettes offer: layers are listed,
//! selected (several at once) and shown, hidden, locked or unlocked; the selection's properties
//! are read, not edited. Nothing here creates, renames, recolours or deletes anything, and there
//! is no dock to move, float or collapse: a panel is shown or it is not.

use std::collections::BTreeSet;

use cadcraft_ui_egui::CadApp;
use cadcraft_ui_egui::icons::{self, Icon};
use cadcraft_ui_egui::theme::{self, Tokens};
use egui::{RichText, ScrollArea, Sense, vec2};
use serde_json::{Value, json};

use super::policy::HostFlag;

/// Width the panel opens at, and how far the user may drag its edge (points).
const DEFAULT_WIDTH: f32 = 320.0;
const WIDTH_RANGE: std::ops::RangeInclusive<f32> = 240.0..=640.0;
/// Above this many selected objects only their number is shown: reading every property of a
/// whole drawing each time the selection changes would stall the frame.
const MAX_OBJECTS: usize = 500;
/// The value shown for a property the selected objects do not share.
const VARIES: &str = "Varies";

/// One layer as the list shows it.
#[derive(Debug, Clone, PartialEq)]
struct LayerRow {
    name: String,
    on: bool,
    locked: bool,
    color: cadcraft_color::Color,
}

/// What a click in the layer list asks for.
#[derive(Debug, Clone, PartialEq)]
enum LayerAction {
    /// Set `field` (`on` or `locked`) of these layers.
    Set { names: Vec<String>, field: &'static str, value: bool },
}

/// The selection's properties as rows, with what they were read for.
#[derive(Default)]
struct Properties {
    key: Option<(u64, u64, Vec<cadcraft_doc::Handle>)>,
    title: String,
    rows: Vec<(String, String)>,
}

#[derive(Default)]
pub struct Panels {
    pub layers_open: bool,
    pub properties_open: bool,
    /// Selected layer names, lower-cased (layer names do not differ by case).
    selected: BTreeSet<String>,
    /// The row a Shift+click extends from.
    anchor: Option<usize>,
    properties: Properties,
}

/// The layer selection after a click on row `index`: plain picks only that row, Ctrl (Cmd)
/// toggles it, Shift selects from the anchor to it. Returns the new anchor.
fn click_row(selected: &mut BTreeSet<String>, anchor: Option<usize>, names: &[String], index: usize, toggle: bool, extend: bool) -> Option<usize> {
    let Some(name) = names.get(index) else { return anchor };
    let key = name.to_lowercase();
    if extend && let Some(from) = anchor.filter(|a| *a < names.len()) {
        let (a, b) = if from <= index { (from, index) } else { (index, from) };
        if !toggle {
            selected.clear();
        }
        selected.extend(names.iter().skip(a).take(b - a + 1).map(|n| n.to_lowercase()));
        return anchor;
    }
    if toggle {
        if !selected.remove(&key) {
            selected.insert(key);
        }
    } else {
        selected.clear();
        selected.insert(key);
    }
    Some(index)
}

/// `linetypeScale` as `Linetype scale`.
fn label_of(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    for (i, c) in key.chars().enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else if c.is_uppercase() {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else if c == '_' {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

/// A property value as text: numbers without float noise, lists joined with commas.
fn text_of(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => if *b { "Yes" } else { "No" }.to_string(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() => {
                let text = format!("{f:.4}");
                text.trim_end_matches('0').trim_end_matches('.').to_string()
            }
            _ => n.to_string(),
        },
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().map(text_of).collect::<Vec<_>>().join(", "),
        Value::Object(_) => value.to_string(),
    }
}

/// One object's properties as `(label, value)` rows; a nested object's fields follow its name
/// (`Start x`).
fn flatten(prefix: &str, value: &Value, out: &mut Vec<(String, String)>, depth: usize) {
    match value {
        Value::Object(fields) if depth < 3 => {
            for (key, value) in fields {
                let name = if prefix.is_empty() { key.clone() } else { format!("{prefix} {key}") };
                flatten(&name, value, out, depth + 1);
            }
        }
        other => out.push((label_of(prefix), text_of(other))),
    }
}

/// The rows for the selected `objects` (the `properties` command's result): one object's own
/// rows, or for several the rows they all have, with [`VARIES`] where the values differ.
fn rows_of(objects: &[Value]) -> Vec<(String, String)> {
    let mut all = objects.iter().map(|object| {
        let mut rows = Vec::new();
        flatten("", object, &mut rows, 0);
        rows
    });
    let Some(mut rows) = all.next() else { return Vec::new() };
    // Nothing to read in an empty value, and the box around an object is not a property of it.
    rows.retain(|(label, value)| !value.is_empty() && !label.starts_with("Bounds"));
    for other in all {
        rows.retain_mut(|(label, value)| match other.iter().find(|(l, _)| l == label) {
            Some((_, v)) => {
                if v != value {
                    *value = VARIES.to_string();
                }
                true
            }
            None => false,
        });
    }
    rows
}

/// What the panel is headed with: the object's type, or how many are selected.
fn title_of(objects: &[Value]) -> String {
    match objects {
        [] => "Nothing selected".to_string(),
        [one] => one.get("type").and_then(Value::as_str).unwrap_or("Object").to_string(),
        many => format!("{} objects", many.len()),
    }
}

impl Panels {
    pub fn set(&mut self, layers: bool, open: bool) {
        if layers {
            self.layers_open = open;
        } else {
            self.properties_open = open;
        }
    }

    /// A new drawing: its layers are other layers.
    pub fn reset(&mut self) {
        self.selected.clear();
        self.anchor = None;
        self.properties = Properties::default();
    }

    /// Draws the open panels in one column on the right. Call before the app draws its frame, so
    /// the drawing takes the room that is left.
    pub fn show(&mut self, ui: &mut egui::Ui, app: &mut CadApp, host: &HostFlag) {
        if !self.layers_open && !self.properties_open {
            return;
        }
        let t = Tokens::get();
        let frame = egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::same(8));
        egui::Panel::right("embed_panels").default_size(DEFAULT_WIDTH).size_range(WIDTH_RANGE).resizable(true).frame(frame).show(ui, |ui| {
            // Long layer names and values are scrolled to, never cut or wrapped mid-word.
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            let both = self.layers_open && self.properties_open;
            let height = ui.available_height();
            if self.layers_open {
                let share = if both { (height * 0.45).max(120.0) } else { height };
                ui.allocate_ui(vec2(ui.available_width(), share), |ui| {
                    ui.set_min_height(share);
                    self.layers(ui, app, host);
                });
            }
            if both {
                ui.separator();
            }
            if self.properties_open {
                self.properties(ui, app, host);
            }
        });
    }

    fn layers(&mut self, ui: &mut egui::Ui, app: &mut CadApp, host: &HostFlag) {
        let t = Tokens::get();
        let rows: Vec<LayerRow> = match app.session.doc() {
            Ok(d) => d.layers.iter().map(|l| LayerRow { name: l.name.clone(), on: l.on, locked: l.locked, color: l.color }).collect(),
            Err(_) => Vec::new(),
        };
        let names: Vec<String> = rows.iter().map(|r| r.name.clone()).collect();
        // Layers that are gone (another drawing, an undo) leave the selection.
        self.selected.retain(|key| names.iter().any(|n| n.to_lowercase() == *key));
        let chosen: Vec<String> = names.iter().filter(|n| self.selected.contains(&n.to_lowercase())).cloned().collect();
        let mut action = None;
        ui.label(RichText::new("Layers").font(theme::body()).strong().color(t.text));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            let any = !chosen.is_empty();
            for (label, field, value) in [("Show", "on", true), ("Hide", "on", false), ("Lock", "locked", true), ("Unlock", "locked", false)] {
                if ui.add_enabled(any, egui::Button::new(label)).clicked() {
                    action = Some(LayerAction::Set { names: chosen.clone(), field, value });
                }
            }
            if ui.button(if chosen.len() == names.len() && any { "Select none" } else { "Select all" }).clicked() {
                if chosen.len() == names.len() {
                    self.selected.clear();
                } else {
                    self.selected = names.iter().map(|n| n.to_lowercase()).collect();
                }
            }
        });
        ui.label(RichText::new(format!("{} of {} selected", chosen.len(), names.len())).font(theme::small()).color(t.text_dim));
        ui.add_space(2.0);
        ScrollArea::both().id_salt("embed-layers").auto_shrink([false, false]).show(ui, |ui| {
            for (index, row) in rows.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 0.0);
                    if icons::button(ui, if row.on { Icon::Bulb } else { Icon::BulbOff }, 18.0, if row.on { "Hide" } else { "Show" }, false).clicked()
                    {
                        action = Some(LayerAction::Set { names: vec![row.name.clone()], field: "on", value: !row.on });
                    }
                    let lock = if row.locked { Icon::Lock } else { Icon::Unlock };
                    if icons::button(ui, lock, 18.0, if row.locked { "Unlock" } else { "Lock" }, false).clicked() {
                        action = Some(LayerAction::Set { names: vec![row.name.clone()], field: "locked", value: !row.locked });
                    }
                    let (swatch, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                    cadcraft_ui_egui::layers::swatch(ui.painter(), swatch, row.color);
                    let selected = self.selected.contains(&row.name.to_lowercase());
                    let text = RichText::new(&row.name).font(theme::body()).color(if row.on { t.text } else { t.text_dim });
                    let response = ui.selectable_label(selected, text);
                    if response.clicked() {
                        let mods = ui.input(|i| i.modifiers);
                        self.anchor = click_row(&mut self.selected, self.anchor, &names, index, mods.command || mods.ctrl, mods.shift);
                    }
                });
            }
        });
        if let Some(LayerAction::Set { names, field, value }) = action {
            // The panel is the viewer's own control, not something the user typed: it may lock
            // layers, which the user's policy otherwise refuses.
            let _host = host.scope();
            for name in names {
                if let Err(e) = app.run("layer.set", json!({"name": name, field: value})) {
                    log::warn!("embed: could not change layer {name}: {e}");
                }
            }
        }
    }

    /// Reads the selection's properties again when the selection or the drawing changed.
    fn refresh(&mut self, app: &mut CadApp, host: &HostFlag) {
        let key = app.session.state().ok().map(|d| (d.uid, d.revision, d.selection.clone()));
        if self.properties.key == key {
            return;
        }
        let count = key.as_ref().map_or(0, |k| k.2.len());
        let objects: Vec<Value> = if count == 0 || count > MAX_OBJECTS {
            Vec::new()
        } else {
            let _host = host.scope();
            let handles: Vec<String> = key.iter().flat_map(|k| k.2.iter().map(|h| h.hex())).collect();
            app.run("properties", json!({"handles": handles}))
                .ok()
                .and_then(|value| value.get("objects").and_then(Value::as_array).cloned())
                .unwrap_or_default()
        };
        self.properties.title = if count > MAX_OBJECTS { format!("{count} objects") } else { title_of(&objects) };
        self.properties.rows = rows_of(&objects);
        self.properties.key = key;
    }

    fn properties(&mut self, ui: &mut egui::Ui, app: &mut CadApp, host: &HostFlag) {
        let t = Tokens::get();
        self.refresh(app, host);
        ui.label(RichText::new("Properties").font(theme::body()).strong().color(t.text));
        ui.label(RichText::new(&self.properties.title).font(theme::small()).color(t.text_dim));
        ui.add_space(2.0);
        ScrollArea::both().id_salt("embed-properties").auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("embed-properties-grid").num_columns(2).spacing(vec2(12.0, 4.0)).striped(true).show(ui, |ui| {
                for (label, value) in &self.properties.rows {
                    ui.label(RichText::new(label).font(theme::body()).color(t.text_dim));
                    ui.label(RichText::new(value).font(theme::body()).color(t.text));
                    ui.end_row();
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        ["0", "Walls", "Doors", "Text"].map(String::from).to_vec()
    }

    #[test]
    fn a_plain_click_selects_one_layer_and_ctrl_adds_or_removes() {
        let mut selected = BTreeSet::new();
        let anchor = click_row(&mut selected, None, &names(), 1, false, false);
        assert_eq!(selected.iter().collect::<Vec<_>>(), ["walls"]);
        let anchor = click_row(&mut selected, anchor, &names(), 3, true, false);
        assert_eq!(selected.iter().collect::<Vec<_>>(), ["text", "walls"]);
        let anchor = click_row(&mut selected, anchor, &names(), 1, true, false);
        assert_eq!(selected.iter().collect::<Vec<_>>(), ["text"]);
        click_row(&mut selected, anchor, &names(), 0, false, false);
        assert_eq!(selected.iter().collect::<Vec<_>>(), ["0"]);
    }

    #[test]
    fn shift_selects_the_range_from_the_last_clicked_layer() {
        let mut selected = BTreeSet::new();
        let anchor = click_row(&mut selected, None, &names(), 3, false, false);
        let anchor = click_row(&mut selected, anchor, &names(), 1, false, true);
        assert_eq!(selected.iter().collect::<Vec<_>>(), ["doors", "text", "walls"]);
        assert_eq!(anchor, Some(3));
        // A row that is not there changes nothing.
        assert_eq!(click_row(&mut selected, anchor, &names(), 9, false, false), anchor);
        assert_eq!(selected.len(), 3);
    }

    #[test]
    fn one_object_shows_its_properties_with_readable_names_and_numbers() {
        let rows = rows_of(&[json!({"type": "Line", "linetypeScale": 1.0, "start": {"x": 33979.000001, "y": 2}, "closed": false})]);
        assert!(rows.contains(&("Type".to_string(), "Line".to_string())), "{rows:?}");
        assert!(rows.contains(&("Linetype scale".to_string(), "1".to_string())), "{rows:?}");
        assert!(rows.contains(&("Start x".to_string(), "33979".to_string())), "{rows:?}");
        assert!(rows.contains(&("Start y".to_string(), "2".to_string())), "{rows:?}");
        assert!(rows.contains(&("Closed".to_string(), "No".to_string())), "{rows:?}");
        let rows = rows_of(&[json!({"type": "Line", "annotativeScales": [], "bounds": [0, 0, 1, 1], "note": ""})]);
        assert_eq!(rows, [("Type".to_string(), "Line".to_string())]);
    }

    #[test]
    fn several_objects_show_what_they_share_and_mark_what_differs() {
        let objects = [json!({"type": "Line", "layer": "Walls", "length": 2.5}), json!({"type": "Circle", "layer": "Walls", "radius": 1})];
        let rows = rows_of(&objects);
        assert_eq!(rows, [("Layer".to_string(), "Walls".to_string()), ("Type".to_string(), VARIES.to_string())]);
        assert_eq!(title_of(&objects), "2 objects");
        assert_eq!(title_of(&objects[..1]), "Line");
        assert_eq!(title_of(&[]), "Nothing selected");
        assert!(rows_of(&[]).is_empty());
    }
}
