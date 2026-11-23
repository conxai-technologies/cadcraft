//! Embed mode's command policy: what the user may run in the viewer. They view, select, measure,
//! show and hide layers, explode and group; nothing they type, click or press creates, edits,
//! opens, saves or exports anything else. The host page's own calls (`glue.rs`) run in a
//! [`HostScope`] and are not limited: the host opens and closes drawings.
//!
//! Installed on the session (`Session::set_command_policy`), so it covers every way a command
//! starts: typed names and aliases, menus, shortcuts, context menus, grips, AutoLISP and the
//! UI-only commands (`run_ui_command`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cadcraft_engine::Session;
use serde_json::Value;

/// The commands the user may run, by id. Exact ids: a prefix such as `view` would also let in
/// the named-view and viewport edits.
pub const ALLOWED: &[&str] = &[
    // View and navigation. LAYOUT.SET, MSPACE and PSPACE are the status bar's layout tabs and a
    // double click into a layout viewport.
    "zoom",
    "zoom.all",
    "zoom.dynamic",
    "zoom.extents",
    "zoom.in",
    "zoom.object",
    "zoom.out",
    "zoom.previous",
    "zoom.realtime",
    "zoom.window",
    "pan",
    "pan.up",
    "pan.down",
    "pan.left",
    "pan.right",
    "regen",
    "regenall",
    "redraw",
    "view.get",
    "layout.set",
    "mspace",
    "pspace",
    // Selection.
    "select",
    "selectall",
    "selectsimilar",
    "ai_deselect",
    // Inquiry: reads the drawing, changes nothing.
    "list",
    "dist",
    "id",
    "area",
    "measuregeom",
    "drawing.inspect",
    "entities",
    "properties",
    // Undo and redo.
    "u",
    "undo",
    "redo",
    "mredo",
    // What the host page's bar offers on the selection.
    "explode",
    "burst",
    "group",
    "ungroup",
    // Layer visibility: on and off, freeze and thaw, isolate. LAYER.SET only with those
    // parameters (see `layer_visibility`).
    "layon",
    "layoff",
    "layfrz",
    "laythw",
    "layiso",
    "layuniso",
    "layer.set",
    // The palettes the host opens and closes.
    "ui.dock",
    "ui.panel.layers",
    "ui.panel.properties",
];

/// The command a typed name stands for, as the engine reads it: `'ZOOM` (transparent), `_PAN` and
/// `.LINE` (English and built-in names) and aliases (`Z`). A name with a dash keeps it when that
/// is a command of its own (`-GROUP`), as the engine's start does.
pub fn command_id(name: &str) -> String {
    let name = name.trim().trim_start_matches('\'').trim_start_matches(['_', '.']).to_ascii_lowercase();
    let dashed = cadcraft_engine::cmd::resolve_alias(&name);
    if cadcraft_engine::find_command(&dashed).is_some() { dashed } else { cadcraft_engine::cmd::resolve_alias(name.trim_start_matches('-')) }
}

/// LAYER.SET changes visibility only: a layer name and `on` / `frozen` flags, nothing else
/// (colour, linetype, lock, rename and the rest are refused).
fn layer_visibility(params: &Value) -> bool {
    let Some(fields) = params.as_object() else { return false };
    fields.get("name").is_some_and(Value::is_string)
        && fields.iter().all(|(key, value)| key == "name" || (matches!(key.as_str(), "on" | "frozen") && value.is_boolean()))
        && fields.len() > 1
}

/// Whether the user may run `name` (a command id or a typed name) with `params`.
pub fn user_may_run(name: &str, params: &Value) -> bool {
    let id = command_id(name);
    match id.as_str() {
        "layer.set" => layer_visibility(params),
        id => ALLOWED.contains(&id),
    }
}

/// Set while the host page's own call runs: the policy lets it through. Shared with the policy
/// closure on the session.
#[derive(Clone, Default)]
pub struct HostFlag(Arc<AtomicBool>);

impl HostFlag {
    /// Whether a host call is running.
    pub fn active(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// Marks the calls made until the scope is dropped as the host's. Dropping restores what was
    /// there before, so an early return or a nested scope cannot leave the flag set.
    pub fn scope(&self) -> HostScope {
        let before = self.0.swap(true, Ordering::Relaxed);
        HostScope { flag: self.clone(), before }
    }
}

pub struct HostScope {
    flag: HostFlag,
    before: bool,
}

impl Drop for HostScope {
    fn drop(&mut self) {
        self.flag.0.store(self.before, Ordering::Relaxed);
    }
}

/// Installs the policy on the session: the host's calls run, the user's only when allowed.
pub fn install(session: &mut Session, host: &HostFlag) {
    let host = host.clone();
    session.set_command_policy(Some(Box::new(move |id: &str, params: &Value| host.active() || user_may_run(id, params))));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_allowed_id_is_a_command() {
        for id in ALLOWED {
            let known = cadcraft_engine::find_command(id).is_some() || cadcraft_ui_egui::menus::is_ui_command(id);
            assert!(known, "{id} is not a command: renamed upstream?");
        }
    }

    #[test]
    fn the_user_may_view_select_measure_and_explode() {
        for name in [
            "zoom",
            "Z",
            "'zoom",
            "zoom.extents",
            "zoom.window",
            "pan",
            "P",
            "_pan",
            "'pan",
            "regen",
            "re",
            "redraw",
            "layout.set",
            "select",
            "selectall",
            "ai_deselect",
            "list",
            "li",
            "dist",
            "di",
            "id",
            "area",
            "measuregeom",
            "drawing.inspect",
            "entities",
            "properties",
            "u",
            "undo",
            "redo",
            "explode",
            "x",
            "burst",
            "group",
            "g",
            "ungroup",
            "layoff",
            "layfrz",
            "layiso",
            "layuniso",
            "layon",
            "laythw",
            "ui.dock",
            "ui.panel.layers",
            "ui.panel.properties",
        ] {
            assert!(user_may_run(name, &Value::Null), "{name} is refused");
        }
    }

    #[test]
    fn the_user_may_not_draw_edit_open_save_or_customise() {
        for name in [
            // Drawing and editing, by id and alias.
            "line",
            "l",
            "_.line",
            "'line",
            "circle",
            "c",
            "pline",
            "erase",
            "e",
            "erase.selection",
            "copy",
            "co",
            "move",
            "m",
            "rotate",
            "scale",
            "stretch",
            "mirror",
            "offset",
            "trim",
            "hatch",
            "text",
            "mtext",
            "textedit",
            "dimlinear",
            "block",
            "insert",
            "properties.set",
            "matchprop",
            "chprop",
            "pasteclip",
            "cutclip",
            "copyclip",
            "oops",
            "-group",
            "groupedit",
            "count",
            "counttable",
            // Layers beyond visibility.
            "layer",
            "la",
            "layer.new",
            "layer.delete",
            "layer.current",
            "laylck",
            "layulk",
            "laymrg",
            "laydel",
            "layerp",
            "layerstate.restore",
            // Views and viewports that change the drawing.
            "view",
            "view.set",
            "vports",
            "mview",
            // Files.
            "open",
            "new",
            "qsave",
            "save",
            "saveas",
            "close",
            "closeall",
            "export",
            "exportpdf",
            "plot",
            "publish",
            "wblock",
            "document.bytes",
            "recover",
            // Block editor, xrefs, images.
            "bedit",
            "refedit",
            "xattach",
            "xref",
            "attach",
            "imageattach",
            // LISP, scripts, settings, customisation.
            "lisp",
            "appload",
            "script",
            "setvar",
            "options",
            "units",
            "aliasedit",
            "grip",
            "grid",
            "osnap",
            // UI commands.
            "ui.open",
            "ui.saveas",
            "ui.sample",
            "ui.start",
            "ui.quit",
            "ui.newtemplate",
            "ui.dialog.layers",
            "ui.dialog.find",
            "ui.shortcuts",
            "ui.toggle.toolbar",
            "ui.toggle.cmdline",
            "ui.toggle.toolsets",
            "ui.resetpalettes",
            // Unknown names (and LISP typed as the first word).
            "nosuchcommand",
            "(entmake",
            "",
        ] {
            assert!(!user_may_run(name, &Value::Null), "{name} is allowed");
        }
    }

    #[test]
    fn layer_set_is_visibility_only() {
        for params in [json!({"name": "A", "on": false}), json!({"name": "A", "frozen": true}), json!({"name": "A", "on": true, "frozen": false})] {
            assert!(user_may_run("layer.set", &params), "{params}");
        }
        for params in [
            Value::Null,
            json!({}),
            json!({"name": "A"}),
            json!({"on": false}),
            json!({"name": "A", "locked": true}),
            json!({"name": "A", "color": 3}),
            json!({"name": "A", "on": false, "color": 3}),
            json!({"name": "A", "newName": "B"}),
            json!({"name": "A", "on": "no"}),
            json!({"name": 7, "on": false}),
        ] {
            assert!(!user_may_run("layer.set", &params), "{params}");
        }
    }

    #[test]
    fn the_host_scope_lets_everything_through_and_is_reset() {
        let host = HostFlag::default();
        let mut session = Session::new();
        install(&mut session, &host);
        assert!(!session.command_allowed("open", &Value::Null));
        {
            let _host = host.scope();
            assert!(session.command_allowed("open", &Value::Null));
            {
                let _nested = host.scope();
            }
            assert!(session.command_allowed("close", &Value::Null), "a nested scope keeps the outer one");
        }
        assert!(!session.command_allowed("open", &Value::Null));
        // An early return out of a scope resets it too.
        let early = |host: &HostFlag| -> Result<(), String> {
            let _host = host.scope();
            Err("failed".to_string())?;
            Ok(())
        };
        assert!(early(&host).is_err());
        assert!(!host.active());
    }
}
