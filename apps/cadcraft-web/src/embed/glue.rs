//! Embed mode's glue: what each host message does to the app, and which app changes are reported
//! back to the host page.

use cadcraft_doc::Handle;
use cadcraft_ui_egui::docking::Panel;
use cadcraft_ui_egui::{CadApp, UiState};
use serde_json::{Value, json};

use super::Incoming;
use super::protocol::{Bounds, EmbedCommand, HostMessage, LayerInfo, PanelName, ToolState, ViewerMessage, ZoomTarget};

/// The engine command a host message runs, with its parameters. `None` for the messages that are
/// not engine commands (open, theme, panels, pointer modes).
pub fn command_for(message: &HostMessage) -> Option<(&'static str, Value)> {
    Some(match message {
        HostMessage::ZoomExtents => ("zoom.extents", json!({})),
        HostMessage::ZoomBy { factor } => ("zoom", json!({"mode": "realtime", "factor": factor})),
        HostMessage::ZoomTo(ZoomTarget::Bounds([x0, y0, x1, y1])) => ("zoom", json!({"mode": "window", "p1": [x0, y0], "p2": [x1, y1]})),
        HostMessage::ZoomTo(ZoomTarget::Handles(handles)) => ("zoom.object", json!({"handles": handles})),
        HostMessage::Select { handles } if handles.is_empty() => ("select", json!({"clear": true})),
        HostMessage::Select { handles } => ("select", json!({"handles": handles})),
        HostMessage::Command { id: EmbedCommand::Explode, .. } => ("explode", json!({})),
        HostMessage::Command { id: EmbedCommand::Group, .. } => ("group", json!({})),
        HostMessage::Command { id: EmbedCommand::Ungroup, .. } => ("ungroup", json!({})),
        HostMessage::Command { id: EmbedCommand::Pan | EmbedCommand::Select, .. }
        | HostMessage::Open { .. }
        | HostMessage::Theme { .. }
        | HostMessage::Panel { .. } => return None,
    })
}

/// Hides the editor's own chrome: the host page's bar carries the tools. Applied every frame,
/// because a shortcut or a context menu can switch a part back on.
pub fn restrict_ui(ui: &mut UiState) {
    ui.show_toolbar = false;
    ui.in_window_menu = false;
    ui.show_file_tabs = false;
    ui.show_command_line = false;
    ui.show_toolsets = false;
    ui.show_viewcube = false;
    ui.start_tab = false;
    ui.show_status_bar = true;
}

fn panel(name: PanelName) -> Panel {
    match name {
        PanelName::Layers => Panel::Layers,
        PanelName::Properties => Panel::Properties,
    }
}

/// The handles of `wanted` that name an entity of the open drawing. A stale link (an entity that
/// was deleted or exploded) is skipped so the rest still apply.
fn known(app: &CadApp, wanted: &[String]) -> Vec<String> {
    let Ok(drawing) = app.session.doc() else { return Vec::new() };
    wanted.iter().filter(|h| Handle::parse_hex(h).is_some_and(|h| drawing.entity(h).is_some())).cloned().collect()
}

/// The box around `handles` in drawing units, or `None` when none of them has an extent.
fn bounds_of(app: &CadApp, handles: &[Handle]) -> Option<Bounds> {
    let drawing = app.session.doc().ok()?;
    let all = handles
        .iter()
        .filter_map(|h| drawing.entity(*h))
        .fold(cadcraft_geom::Bounds2::EMPTY, |all, entity| all.union(&cadcraft_doc::entity_bounds(drawing, entity, 0)));
    finite(&all)
}

fn finite(b: &cadcraft_geom::Bounds2) -> Option<Bounds> {
    let out = [b.min.x, b.min.y, b.max.x, b.max.y];
    (!b.is_empty() && out.iter().all(|v| v.is_finite())).then_some(out)
}

/// DXF `$INSUNITS` as the short name the host shows.
fn units(code: i64) -> &'static str {
    match code {
        1 => "in",
        2 => "ft",
        4 => "mm",
        5 => "cm",
        6 => "m",
        _ => "unitless",
    }
}

fn opened(app: &CadApp, name: &str) -> ViewerMessage {
    let (extents, units, layers) = match app.session.state() {
        Ok(state) => (
            finite(&state.doc.extents(&state.space)),
            units(state.doc.header.i64("INSUNITS", 0)),
            state.doc.layers.iter().map(|l| LayerInfo { name: l.name.clone(), on: l.on }).collect(),
        ),
        Err(_) => (None, units(0), Vec::new()),
    };
    ViewerMessage::Opened { name: name.to_string(), extents, units: units.to_string(), layers }
}

fn tool_state(app: &CadApp) -> ToolState {
    let selection = app.session.selection();
    let drawing = app.session.doc().ok();
    let is_insert = |h: &Handle| drawing.and_then(|d| d.entity(*h)).is_some_and(|e| matches!(e.kind, cadcraft_doc::EntityKind::Insert(_)));
    let grouped = |h: &Handle| drawing.is_some_and(|d| d.groups.iter().any(|g| g.members.contains(h)));
    let layout = &app.ui.docking.layout;
    ToolState {
        can_explode: selection.iter().any(is_insert),
        can_group: selection.len() >= 2,
        can_ungroup: selection.iter().any(grouped),
        layers_open: layout.contains(&Panel::Layers),
        properties_open: layout.contains(&Panel::Properties),
    }
}

fn error(message: impl Into<String>) -> Vec<ViewerMessage> {
    vec![ViewerMessage::Error { message: message.into(), fatal: false }]
}

/// What the host page was last told, so each change is reported once.
#[derive(Default)]
pub struct Glue {
    selection: Option<Vec<Handle>>,
    state: Option<ToolState>,
    /// The next selection change was asked for by the host page.
    host_selected: bool,
}

impl Glue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Once, when the app exists: the palettes start closed (the host page opens them).
    pub fn start(&mut self, app: &mut CadApp) {
        for name in [PanelName::Layers, PanelName::Properties] {
            self.set_panel(app, name, false);
        }
    }

    fn set_panel(&mut self, app: &mut CadApp, name: PanelName, open: bool) {
        let panel = panel(name);
        if app.ui.docking.layout.contains(&panel) == open {
            return;
        }
        let operation = if open { "open" } else { "close" };
        if let Err(e) = app.run("ui.dock", json!({"operation": operation, "panel": panel.id()})) {
            log::warn!("embed: could not {operation} the {} panel: {e}", panel.id());
        }
    }

    fn open(&mut self, app: &mut CadApp, name: &str, bytes: Option<Vec<u8>>) -> Vec<ViewerMessage> {
        let Some(bytes) = bytes else { return error(format!("Open: {name} came without its bytes")) };
        if let Some(e) = CadApp::open_size_error(name, bytes.len() as u64) {
            return error(e);
        }
        let before = app.session.docs.len();
        let data = cadcraft_engine::cmd::file::base64_encode(&bytes);
        if let Err(e) = app.run("open", json!({"data": data, "name": name})) {
            return error(e);
        }
        // One drawing at a time: the ones that were open go, oldest first, which leaves the new
        // one (opened last) as the only document.
        for _ in 0..before {
            if let Err(e) = app.run("close", json!({"index": 0, "discard": true})) {
                log::warn!("embed: could not close the previous drawing: {e}");
                break;
            }
        }
        app.ui.start_tab = false;
        app.canvas.zoom_pending = true;
        // The new drawing starts with nothing selected; the host learns that from `opened`.
        self.selection = Some(Vec::new());
        self.host_selected = false;
        vec![opened(app, name)]
    }

    /// Does what one host message asks. The replies are the messages that answer it directly
    /// (an error, a command's result, `opened`); changes it causes are reported by [`Self::events`].
    pub fn apply(&mut self, app: &mut CadApp, incoming: Incoming) -> Vec<ViewerMessage> {
        let Incoming { message, bytes } = incoming;
        match &message {
            HostMessage::Open { name } => return self.open(app, name, bytes),
            HostMessage::Theme { dark, background, selection } => {
                app.ui.theme = if *dark { cadcraft_ui_egui::theme::ThemePref::Dark } else { cadcraft_ui_egui::theme::ThemePref::Light };
                let colour = |c: &[u8; 3]| egui::Color32::from_rgb(c[0], c[1], c[2]);
                app.options.model_background = background.as_ref().map(colour);
                app.options.selection_color = selection.as_ref().map(colour);
                return Vec::new();
            }
            HostMessage::Panel { name, open } => {
                self.set_panel(app, *name, *open);
                return Vec::new();
            }
            HostMessage::Command { id: EmbedCommand::Pan, request_id } => {
                app.start("pan");
                return vec![ViewerMessage::CommandDone { request_id: request_id.clone(), ok: true, handles: Vec::new(), error: None }];
            }
            HostMessage::Command { id: EmbedCommand::Select, request_id } => {
                app.session.cancel();
                return vec![ViewerMessage::CommandDone { request_id: request_id.clone(), ok: true, handles: Vec::new(), error: None }];
            }
            _ => {}
        }
        // Handles that no longer exist are dropped first, so one stale link does not fail the rest.
        let message = match message {
            HostMessage::Select { handles } => HostMessage::Select { handles: known(app, &handles) },
            HostMessage::ZoomTo(ZoomTarget::Handles(handles)) => {
                let handles = known(app, &handles);
                if handles.is_empty() {
                    return Vec::new();
                }
                HostMessage::ZoomTo(ZoomTarget::Handles(handles))
            }
            other => other,
        };
        let Some((id, params)) = command_for(&message) else { return Vec::new() };
        // The entity commands act on the selection; with none they would do nothing and say so
        // only on the hidden command line.
        if let HostMessage::Command { request_id, .. } = &message
            && app.session.selection().is_empty()
        {
            let error = Some("Nothing is selected".to_string());
            return vec![ViewerMessage::CommandDone { request_id: request_id.clone(), ok: false, handles: Vec::new(), error }];
        }
        let result = app.run(id, params);
        match message {
            HostMessage::Select { .. } => {
                self.host_selected = true;
                Vec::new()
            }
            HostMessage::Command { request_id, .. } => vec![match result {
                Ok(value) => {
                    let handles =
                        value.get("handles").and_then(Value::as_array).map(|a| a.iter().filter_map(|h| h.as_str().map(str::to_string)).collect());
                    ViewerMessage::CommandDone { request_id, ok: true, handles: handles.unwrap_or_default(), error: None }
                }
                Err(e) => ViewerMessage::CommandDone { request_id, ok: false, handles: Vec::new(), error: Some(e) },
            }],
            _ => {
                if let Err(e) = result {
                    log::warn!("embed: {id} failed: {e}");
                }
                Vec::new()
            }
        }
    }

    /// Called once per frame: the selection and tool state, each reported when it changed.
    pub fn events(&mut self, app: &CadApp) -> Vec<ViewerMessage> {
        let mut out = Vec::new();
        let selection = app.session.selection();
        if self.selection.as_ref() != Some(&selection) {
            // The first frame only records the starting point: nothing changed yet.
            if self.selection.is_some() {
                out.push(ViewerMessage::Selection {
                    handles: selection.iter().map(|h| h.hex()).collect(),
                    bounds: bounds_of(app, &selection),
                    from_host: self.host_selected,
                });
            }
            self.selection = Some(selection);
        }
        self.host_selected = false;
        let state = tool_state(app);
        if self.state.as_ref() != Some(&state) {
            out.push(ViewerMessage::State(state.clone()));
            self.state = Some(state);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::protocol::{EmbedCommand, PanelName, ZoomTarget};
    use cadcraft_engine::Session;
    use cadcraft_ui_egui::{CadApp, Services};
    use serde_json::json;

    /// An app showing the sample drawing, with the embed start-up applied.
    fn sample_app() -> (CadApp, Glue) {
        // The engine reads and writes files through hooks the app installs at start-up.
        cadcraft_engine::cmd::file::set_io(cadcraft_engine::cmd::file::IoHooks {
            read: |b, name| cadcraft_io::read(b, name).map_err(|e| e.to_string()),
            write: |d, name| cadcraft_io::write(d, name).map_err(|e| e.to_string()),
            plot: None,
        });
        let mut app = CadApp::new(Session::new(), Services::default());
        app.run("ui.sample", json!({})).expect("the sample drawing opens");
        let mut glue = Glue::new();
        glue.start(&mut app);
        // The first frame reports the state the host starts from.
        let _ = glue.events(&app);
        (app, glue)
    }

    /// Handles of the entities in the open drawing.
    fn handles_of(app: &mut CadApp) -> Vec<String> {
        let drawing = app.run("drawing.inspect", json!({"limit": 100000})).expect("inspect");
        let entities = drawing["entities"].as_array().expect("entities");
        entities.iter().filter_map(|e| e["handle"].as_str().map(str::to_string)).collect()
    }

    fn message(message: HostMessage) -> Incoming {
        Incoming { message, bytes: None }
    }

    fn selections(events: &[ViewerMessage]) -> Vec<&ViewerMessage> {
        events.iter().filter(|e| matches!(e, ViewerMessage::Selection { .. })).collect()
    }

    #[test]
    fn host_messages_map_to_commands() {
        assert_eq!(command_for(&HostMessage::ZoomExtents), Some(("zoom.extents", json!({}))));
        assert_eq!(command_for(&HostMessage::ZoomBy { factor: 1.25 }), Some(("zoom", json!({"mode": "realtime", "factor": 1.25}))));
        assert_eq!(
            command_for(&HostMessage::ZoomTo(ZoomTarget::Bounds([1.0, 2.0, 3.0, 4.0]))),
            Some(("zoom", json!({"mode": "window", "p1": [1.0, 2.0], "p2": [3.0, 4.0]})))
        );
        assert_eq!(command_for(&HostMessage::Select { handles: vec!["1B7".into()] }), Some(("select", json!({"handles": ["1B7"]}))));
        assert_eq!(command_for(&HostMessage::Select { handles: vec![] }), Some(("select", json!({"clear": true}))));
        assert_eq!(command_for(&HostMessage::ZoomTo(ZoomTarget::Handles(vec!["2C".into()]))), Some(("zoom.object", json!({"handles": ["2C"]}))));
        assert_eq!(command_for(&HostMessage::Command { id: EmbedCommand::Explode, request_id: "r".into() }), Some(("explode", json!({}))));
        assert_eq!(command_for(&HostMessage::Command { id: EmbedCommand::Pan, request_id: "r".into() }), None);
        assert_eq!(command_for(&HostMessage::Open { name: "a.dxf".into() }), None);
        assert_eq!(command_for(&HostMessage::Panel { name: PanelName::Layers, open: true }), None);
    }

    #[test]
    fn restricted_ui_hides_editor_chrome() {
        let mut ui = cadcraft_ui_egui::UiState::default();
        restrict_ui(&mut ui);
        assert!(!ui.show_toolbar && !ui.in_window_menu && !ui.show_file_tabs && !ui.show_command_line && !ui.show_toolsets && !ui.start_tab);
        assert!(!ui.show_viewcube);
        assert!(ui.show_status_bar);
    }

    #[test]
    fn selection_event_is_sent_once_per_change_and_skips_unknown_handles() {
        let (mut app, mut glue) = sample_app();
        let first = handles_of(&mut app).into_iter().next().expect("the sample has entities");

        let replies = glue.apply(&mut app, message(HostMessage::Select { handles: vec![first.clone(), "FFFFFFFF".into()] }));
        assert!(replies.is_empty());
        let events = glue.events(&app);
        let sent = selections(&events);
        assert_eq!(sent.len(), 1);
        match sent[0] {
            ViewerMessage::Selection { handles, bounds, from_host } => {
                assert_eq!(handles, &vec![first]);
                assert!(bounds.is_some());
                assert!(*from_host);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(selections(&glue.events(&app)).is_empty());

        glue.apply(&mut app, message(HostMessage::Select { handles: vec![] }));
        let events = glue.events(&app);
        assert_eq!(selections(&events), vec![&ViewerMessage::Selection { handles: vec![], bounds: None, from_host: true }]);
    }

    #[test]
    fn a_selection_the_user_makes_is_reported_as_theirs() {
        let (mut app, mut glue) = sample_app();
        let first = handles_of(&mut app).into_iter().next().expect("the sample has entities");
        app.run("select", json!({"handles": [first]})).expect("select");
        let events = glue.events(&app);
        assert!(matches!(selections(&events)[..], [ViewerMessage::Selection { from_host: false, .. }]));
    }

    #[test]
    fn explode_reports_new_handles_and_needs_a_selection() {
        let (mut app, mut glue) = sample_app();
        let nothing = glue.apply(&mut app, message(HostMessage::Command { id: EmbedCommand::Explode, request_id: "r0".into() }));
        assert!(
            matches!(&nothing[..], [ViewerMessage::CommandDone { request_id, ok: false, error: Some(error), .. }] if request_id == "r0" && !error.is_empty())
        );

        // Turn two entities into a block: the reference that replaces them is what gets exploded.
        let parts: Vec<String> = handles_of(&mut app).into_iter().take(2).collect();
        let block = app.run("block", json!({"name": "EMBEDTEST", "base": [0.0, 0.0], "handles": parts})).expect("block");
        let insert = block["insert"].as_str().expect("the block left a reference").to_string();
        glue.apply(&mut app, message(HostMessage::Select { handles: vec![insert.clone()] }));
        let events = glue.events(&app);
        assert!(events.iter().any(|e| matches!(e, ViewerMessage::State(state) if state.can_explode)));
        let done = glue.apply(&mut app, message(HostMessage::Command { id: EmbedCommand::Explode, request_id: "r1".into() }));
        assert!(
            matches!(&done[..], [ViewerMessage::CommandDone { request_id, ok: true, handles, error: None }] if request_id == "r1" && !handles.is_empty())
        );
    }

    #[test]
    fn panels_open_and_close_and_the_state_follows() {
        let (mut app, mut glue) = sample_app();
        glue.apply(&mut app, message(HostMessage::Panel { name: PanelName::Layers, open: true }));
        assert!(glue.events(&app).iter().any(|e| matches!(e, ViewerMessage::State(state) if state.layers_open && !state.properties_open)));
        glue.apply(&mut app, message(HostMessage::Panel { name: PanelName::Layers, open: false }));
        assert!(glue.events(&app).iter().any(|e| matches!(e, ViewerMessage::State(state) if !state.layers_open)));
    }

    #[test]
    fn open_replaces_the_drawing_and_reports_it() {
        let (mut app, mut glue) = sample_app();
        let dxf = app.run("document.bytes", json!({"name": "plan.dxf"})).ok();
        // A drawing the app wrote itself is the simplest valid DXF to open.
        let Some(bytes) = dxf.and_then(|v| v["data"].as_str().map(str::to_string)).and_then(|d| cadcraft_engine::cmd::file::base64_decode(&d)) else {
            panic!("the sample drawing could not be written as DXF");
        };
        let replies = glue.apply(&mut app, Incoming { message: HostMessage::Open { name: "plan.dxf".into() }, bytes: Some(bytes) });
        assert!(matches!(&replies[..], [ViewerMessage::Opened { name, extents: Some(_), layers, .. }] if name == "plan.dxf" && !layers.is_empty()));
        assert_eq!(app.session.docs.len(), 1);
    }

    #[test]
    fn a_file_that_is_not_a_drawing_is_an_error_and_the_viewer_stays_usable() {
        let (mut app, mut glue) = sample_app();
        let replies =
            glue.apply(&mut app, Incoming { message: HostMessage::Open { name: "notes.dxf".into() }, bytes: Some(b"not a drawing".to_vec()) });
        assert!(matches!(&replies[..], [ViewerMessage::Error { fatal: false, message }] if !message.is_empty()));
        let missing = glue.apply(&mut app, Incoming { message: HostMessage::Open { name: "plan.dxf".into() }, bytes: None });
        assert!(matches!(&missing[..], [ViewerMessage::Error { fatal: false, .. }]));
        assert!(glue.apply(&mut app, message(HostMessage::ZoomExtents)).is_empty());
    }
}
