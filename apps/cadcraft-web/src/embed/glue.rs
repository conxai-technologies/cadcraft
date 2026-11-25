//! Embed mode's glue: what each host message does to the app, and which app changes are reported
//! back to the host page.

use cadcraft_doc::Handle;
use cadcraft_ui_egui::docking::Panel;
use cadcraft_ui_egui::{CadApp, UiState};
use serde_json::{Value, json};

use super::Incoming;
use super::policy::{self, HostFlag};
use super::protocol::{Bounds, EmbedCommand, HighlightBox, HostMessage, LayerInfo, PanelName, ToolState, ViewerMessage, ZoomTarget};

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
        | HostMessage::Panel { .. }
        | HostMessage::Highlight { .. } => return None,
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
    ui.show_toolsets_strip = false;
    ui.show_viewcube = false;
    ui.start_tab = false;
    ui.show_status_bar = true;
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

fn opened(app: &CadApp, name: &str, saved_view: bool) -> ViewerMessage {
    let (extents, units, layers) = match app.session.state() {
        Ok(state) => (
            finite(&state.doc.extents(&state.space)),
            units(state.doc.header.i64("INSUNITS", 0)),
            state.doc.layers.iter().map(|l| LayerInfo { name: l.name.clone(), on: l.on }).collect(),
        ),
        Err(_) => (None, units(0), Vec::new()),
    };
    ViewerMessage::Opened { name: name.to_string(), extents, units: units.to_string(), layers, saved_view }
}

/// The part of the tool state that depends on the selection and the drawing. One pass over the
/// groups' members, so a large selection in a drawing with many groups stays cheap.
fn selection_tools(app: &CadApp, selection: &[Handle]) -> (bool, bool, bool) {
    let drawing = app.session.doc().ok();
    let is_insert = |h: &Handle| drawing.and_then(|d| d.entity(*h)).is_some_and(|e| matches!(e.kind, cadcraft_doc::EntityKind::Insert(_)));
    let can_ungroup = !selection.is_empty()
        && drawing.is_some_and(|d| {
            let grouped: std::collections::HashSet<Handle> = d.groups.iter().flat_map(|g| g.members.iter().copied()).collect();
            selection.iter().any(|h| grouped.contains(h))
        });
    (selection.iter().any(is_insert), selection.len() >= 2, can_ungroup)
}

fn error(message: impl Into<String>) -> Vec<ViewerMessage> {
    vec![ViewerMessage::Error { message: message.into(), fatal: false }]
}

/// What the host page was last told, so each change is reported once.
#[derive(Default)]
pub struct Glue {
    selection: Option<Vec<Handle>>,
    state: Option<ToolState>,
    /// The document and revision the selection-dependent tool state was worked out for.
    examined: Option<(u64, u64)>,
    selection_tools: (bool, bool, bool),
    /// The next selection change was asked for by the host page.
    host_selected: bool,
    /// Set while the host page's call runs: the command policy lets it through (`policy.rs`).
    host: HostFlag,
    /// The boxes `cadcraft.highlight` asked for, painted over the drawing (`overlay.rs`).
    highlight: Vec<HighlightBox>,
    /// The viewer's own Layers and Properties panels (`panels.rs`).
    panels: super::panels::Panels,
    /// Clicks in highlight boxes, reported with the next [`Self::events`].
    clicked: Vec<ViewerMessage>,
}

impl Glue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Once, when the app exists: installs the command policy (the user only views, selects and
    /// explores; `policy.rs`), and the palettes start closed (the host page opens them).
    pub fn start(&mut self, app: &mut CadApp) {
        policy::install(&mut app.session, &self.host);
        let _host = self.host.scope();
        // The app's own palettes (with their editing tools and dock) stay closed for good: the
        // host page's Layers and Properties buttons show the viewer's panels (`panels.rs`).
        for panel in [Panel::Layers, Panel::Properties] {
            if app.ui.docking.layout.contains(&panel)
                && let Err(e) = app.run("ui.dock", json!({"operation": "close", "panel": panel.id()}))
            {
                log::warn!("embed: could not close the {} palette: {e}", panel.id());
            }
        }
    }

    /// Draws the open panels beside the drawing; call before the app draws its frame.
    pub fn show_panels(&mut self, ui: &mut egui::Ui, app: &mut CadApp) {
        self.panels.show(ui, app, &self.host);
    }

    /// A primary click at `pos` (screen), after the app handled it. Inside a highlight box that
    /// has an id it is reported to the host, wherever in the box it lands. On empty space the
    /// selection window the click opened is closed again; on an object, the object is selected
    /// as usual and the box is reported after that selection.
    pub fn canvas_click(&mut self, app: &mut CadApp, pos: egui::Pos2) {
        let Some(xf) = app.canvas.xf else { return };
        let model = app.session.state().is_ok_and(|s| s.space == cadcraft_doc::Space::Model);
        if !model || !xf.rect.contains(pos) {
            return;
        }
        let Some(id) = super::overlay::box_at(&xf, &self.highlight, pos).map(str::to_string) else { return };
        if app.session.pending_window.is_some() {
            app.session.cancel();
        }
        self.clicked.push(ViewerMessage::HighlightClicked { id });
    }

    fn open(&mut self, app: &mut CadApp, name: &str, bytes: Option<Vec<u8>>, refused: Option<String>) -> Vec<ViewerMessage> {
        if let Some(reason) = refused {
            return error(reason);
        }
        let Some(bytes) = bytes else { return error(format!("Open: {name} came without its bytes")) };
        if let Some(e) = CadApp::open_size_error(name, bytes.len() as u64) {
            return error(e);
        }
        let before = app.session.docs.len();
        let data = cadcraft_engine::cmd::file::base64_encode(&bytes);
        // The engine decodes its own copy: ours is not kept beside it while the drawing is read.
        drop(bytes);
        let result = match app.run("open", json!({"data": data, "name": name})) {
            Ok(result) => result,
            Err(e) => return error(e),
        };
        // One drawing at a time: the ones that were open go, oldest first, which leaves the new
        // one (opened last) as the only document.
        for _ in 0..before {
            if let Err(e) = app.run("close", json!({"index": 0})) {
                log::warn!("embed: could not close the previous drawing: {e}");
                break;
            }
        }
        app.ui.start_tab = false;
        // A drawing that was saved with a view opens on it; any other is fitted to the window.
        let saved_view = result.get("saved_view").and_then(Value::as_bool).unwrap_or(false);
        app.canvas.zoom_pending = !saved_view;
        // The new drawing starts with nothing selected; the host learns that from `opened`.
        self.selection = Some(Vec::new());
        self.host_selected = false;
        self.highlight.clear();
        self.clicked.clear();
        self.panels.reset();
        vec![opened(app, name, saved_view)]
    }

    /// Does what one host message asks. The replies are the messages that answer it directly
    /// (an error, a command's result, `opened`); changes it causes are reported by [`Self::events`].
    pub fn apply(&mut self, app: &mut CadApp, incoming: Incoming) -> Vec<ViewerMessage> {
        // Everything below is the host page's doing: the policy lets it run (opening, closing).
        let _host = self.host.scope();
        let Incoming { message, bytes, refused } = incoming;
        match &message {
            HostMessage::Open { name } => return self.open(app, name, bytes, refused),
            HostMessage::Theme { dark, background, selection } => {
                app.ui.theme = if *dark { cadcraft_ui_egui::theme::ThemePref::Dark } else { cadcraft_ui_egui::theme::ThemePref::Light };
                let colour = |c: &[u8; 3]| egui::Color32::from_rgb(c[0], c[1], c[2]);
                app.options.model_background = background.as_ref().map(colour);
                app.options.selection_color = selection.as_ref().map(colour);
                return Vec::new();
            }
            HostMessage::Panel { name, open } => {
                self.panels.set(*name == PanelName::Layers, *open);
                return Vec::new();
            }
            HostMessage::Highlight { boxes, zoom } => {
                self.highlight.clone_from(boxes);
                let extents = app.session.state().ok().and_then(|s| finite(&s.doc.extents(&s.space)));
                if *zoom && let Some(window) = super::overlay::zoom_window(boxes, extents) {
                    // As `cadcraft.zoomTo` with bounds, and it too wins over a pending fit.
                    app.canvas.zoom_pending = false;
                    if let Some((id, params)) = command_for(&HostMessage::ZoomTo(ZoomTarget::Bounds(window)))
                        && let Err(e) = app.run(id, params)
                    {
                        log::warn!("embed: zooming to the highlight failed: {e}");
                    }
                }
                return Vec::new();
            }
            HostMessage::Command { id: EmbedCommand::Pan, request_id } => {
                // The hand pointer, not the PAN command: that one asks for two points.
                if app.session.running.is_some() {
                    app.session.cancel();
                }
                app.canvas.hand_pan = true;
                return vec![ViewerMessage::CommandDone { request_id: request_id.clone(), ok: true, handles: Vec::new(), error: None }];
            }
            HostMessage::Command { id: EmbedCommand::Select, request_id } => {
                app.canvas.hand_pan = false;
                // With no command running, cancel would clear the selection, which is not what
                // switching back to the pointer means.
                if app.session.running.is_some() {
                    app.session.cancel();
                }
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
        // A view the host asks for wins over the fit that is still pending from an open.
        if matches!(message, HostMessage::ZoomExtents | HostMessage::ZoomBy { .. } | HostMessage::ZoomTo(_)) {
            app.canvas.zoom_pending = false;
        }
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

    /// The boxes to paint over the drawing.
    pub fn highlights(&self) -> &[HighlightBox] {
        &self.highlight
    }

    /// Called once per frame: the selection and tool state, each reported when it changed.
    pub fn events(&mut self, app: &CadApp) -> Vec<ViewerMessage> {
        let mut out = Vec::new();
        let document = app.session.state().ok();
        let selection: &[Handle] = document.map_or(&[], |d| d.selection.as_slice());
        let selection_changed = self.selection.as_deref() != Some(selection);
        if selection_changed {
            // The first frame only records the starting point: nothing changed yet.
            if self.selection.is_some() {
                out.push(ViewerMessage::Selection {
                    handles: selection.iter().map(|h| h.hex()).collect(),
                    bounds: bounds_of(app, selection),
                    from_host: self.host_selected,
                });
            }
            self.selection = Some(selection.to_vec());
        }
        self.host_selected = false;
        // The drawing is only examined again when it or the selection changed, not every frame.
        let examined = document.map(|d| (d.uid, d.revision));
        if selection_changed || self.examined != examined {
            self.selection_tools = selection_tools(app, selection);
            self.examined = examined;
        }
        let state = ToolState {
            can_explode: self.selection_tools.0,
            can_group: self.selection_tools.1,
            can_ungroup: self.selection_tools.2,
            layers_open: self.panels.layers_open,
            properties_open: self.panels.properties_open,
        };
        if self.state.as_ref() != Some(&state) {
            // After the selection: a click in a box may have cleared it first.
            out.append(&mut self.clicked);
            out.push(ViewerMessage::State(state.clone()));
            self.state = Some(state);
        }
        // After the selection: a click in a box may have cleared it first.
        out.append(&mut self.clicked);
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
        Incoming { message, bytes: None, refused: None }
    }

    /// The sample drawing as DXF bytes: a drawing the app wrote itself is the simplest valid one.
    /// Writing it out is not for the user: the test asks as the host.
    fn sample_dxf(app: &mut CadApp, glue: &Glue) -> Vec<u8> {
        let _host = glue.host.scope();
        let written = app.run("document.bytes", json!({"name": "plan.dxf"})).expect("the sample is written as DXF");
        cadcraft_engine::cmd::file::base64_decode(written["data"].as_str().expect("data")).expect("base64")
    }

    fn open(name: &str, bytes: Vec<u8>) -> Incoming {
        Incoming { message: HostMessage::Open { name: name.into() }, bytes: Some(bytes), refused: None }
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
        assert!(!ui.show_toolsets_strip);
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
        let host = glue.host.scope();
        let block = app.run("block", json!({"name": "EMBEDTEST", "base": [0.0, 0.0], "handles": parts})).expect("block");
        let insert = block["insert"].as_str().expect("the block left a reference").to_string();
        drop(host);
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
        let bytes = sample_dxf(&mut app, &glue);
        let replies = glue.apply(&mut app, open("plan.dxf", bytes));
        assert!(matches!(&replies[..], [ViewerMessage::Opened { name, extents: Some(_), layers, .. }] if name == "plan.dxf" && !layers.is_empty()));
        assert_eq!(app.session.docs.len(), 1);
    }

    #[test]
    fn two_opens_in_a_row_report_each_drawing_and_keep_the_last() {
        let (mut app, mut glue) = sample_app();
        let bytes = sample_dxf(&mut app, &glue);
        let first = glue.apply(&mut app, open("a.dxf", bytes.clone()));
        let second = glue.apply(&mut app, open("b.dxf", bytes));
        assert!(matches!(&first[..], [ViewerMessage::Opened { name, .. }] if name == "a.dxf"));
        assert!(matches!(&second[..], [ViewerMessage::Opened { name, .. }] if name == "b.dxf"));
        assert_eq!(app.session.docs.len(), 1);
        assert_eq!(app.session.state().expect("a drawing is open").title, "b.dxf");
    }

    #[test]
    fn a_file_that_cannot_be_opened_is_an_error_and_the_open_drawing_stays() {
        let (mut app, mut glue) = sample_app();
        let title = app.session.state().expect("the sample is open").title.clone();
        let documents = app.session.docs.len();
        for refused in [
            open("notes.dxf", b"not a drawing".to_vec()),
            Incoming { message: HostMessage::Open { name: "plan.dxf".into() }, bytes: None, refused: None },
            Incoming { message: HostMessage::Open { name: "huge.dxf".into() }, bytes: None, refused: Some("huge.dxf is too large".into()) },
        ] {
            let replies = glue.apply(&mut app, refused);
            assert!(matches!(&replies[..], [ViewerMessage::Error { fatal: false, message }] if !message.is_empty()));
            assert_eq!(app.session.docs.len(), documents);
            assert_eq!(app.session.state().expect("still open").title, title);
        }
        let too_large = glue.apply(
            &mut app,
            Incoming { message: HostMessage::Open { name: "huge.dxf".into() }, bytes: None, refused: Some("huge.dxf is too large".into()) },
        );
        assert_eq!(too_large, vec![ViewerMessage::Error { message: "huge.dxf is too large".into(), fatal: false }]);
    }

    #[test]
    fn a_zoom_the_host_sends_after_open_is_not_replaced_by_zoom_extents() {
        let (mut app, mut glue) = sample_app();
        let bytes = sample_dxf(&mut app, &glue);
        glue.apply(&mut app, open("plan.dxf", bytes));
        glue.apply(&mut app, message(HostMessage::ZoomBy { factor: 2.0 }));
        assert!(!app.canvas.zoom_pending);
    }

    #[test]
    fn the_select_pointer_keeps_the_selection() {
        let (mut app, mut glue) = sample_app();
        let first = handles_of(&mut app).into_iter().next().expect("the sample has entities");
        glue.apply(&mut app, message(HostMessage::Select { handles: vec![first] }));
        let done = glue.apply(&mut app, message(HostMessage::Command { id: EmbedCommand::Select, request_id: "s".into() }));
        assert!(matches!(&done[..], [ViewerMessage::CommandDone { ok: true, .. }]));
        assert_eq!(app.session.selection().len(), 1);
    }

    #[test]
    fn the_pan_pointer_drags_the_view_and_the_select_pointer_picks_again() {
        let (mut app, mut glue) = sample_app();
        let done = glue.apply(&mut app, message(HostMessage::Command { id: EmbedCommand::Pan, request_id: "p".into() }));
        assert!(matches!(&done[..], [ViewerMessage::CommandDone { ok: true, .. }]));
        // No point prompts: the hand drags the view for as long as it is the pointer.
        assert!(app.canvas.hand_pan);
        assert!(app.session.running.is_none());
        glue.apply(&mut app, message(HostMessage::Command { id: EmbedCommand::Select, request_id: "s".into() }));
        assert!(!app.canvas.hand_pan);
    }

    #[test]
    fn a_click_on_empty_space_in_a_highlight_box_is_reported_once() {
        let (mut app, mut glue) = sample_app();
        let _ = glue.events(&app);
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0));
        app.canvas.xf = Some(cadcraft_ui_egui::canvas::Xf { rect, center: cadcraft_geom::Vec2::new(0.0, 0.0), scale: 1.0, twist: 0.0 });
        app.canvas.hover = None;
        let boxed = |bounds, id: &str| HighlightBox { bounds, color: None, id: Some(id.to_string()) };
        let boxes = vec![boxed([-100.0, -100.0, 100.0, 100.0], "outer"), boxed([-10.0, -10.0, 10.0, 10.0], "inner")];
        glue.apply(&mut app, message(HostMessage::Highlight { boxes, zoom: false }));
        // The view's centre is the middle of the canvas: inside both boxes, the smaller one wins.
        glue.canvas_click(&mut app, rect.center());
        assert!(matches!(&glue.events(&app)[..], [ViewerMessage::HighlightClicked { id }] if id == "inner"));
        assert!(glue.events(&app).is_empty());
        glue.canvas_click(&mut app, rect.center() + egui::vec2(60.0, 0.0));
        assert!(matches!(&glue.events(&app)[..], [ViewerMessage::HighlightClicked { id }] if id == "outer"));
        // Outside every box, and outside the drawing area: nothing.
        glue.canvas_click(&mut app, rect.center() + egui::vec2(180.0, 0.0));
        glue.canvas_click(&mut app, egui::pos2(900.0, 900.0));
        assert!(glue.events(&app).is_empty());
    }

    /// One frame of the shell: the viewer's panels, then the app.
    fn frame(app: &mut CadApp, glue: &mut Glue) {
        let ctx = egui::Context::default();
        for _ in 0..2 {
            let raw =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                app.logic(ui.ctx());
                glue.show_panels(ui, app);
                app.ui(ui);
            });
            out.textures_delta.clear();
        }
    }

    #[test]
    fn the_viewers_own_panels_take_room_beside_the_drawing_and_the_palettes_stay_closed() {
        let (mut app, mut glue) = sample_app();
        frame(&mut app, &mut glue);
        let full = app.canvas.rect.expect("the drawing was drawn").width();
        let first = handles_of(&mut app).into_iter().next().expect("the sample has entities");
        glue.apply(&mut app, message(HostMessage::Select { handles: vec![first] }));
        glue.apply(&mut app, message(HostMessage::Panel { name: PanelName::Layers, open: true }));
        glue.apply(&mut app, message(HostMessage::Panel { name: PanelName::Properties, open: true }));
        frame(&mut app, &mut glue);
        let beside = app.canvas.rect.expect("the drawing was drawn").width();
        assert!(beside < full - 200.0, "{beside} of {full}");
        assert!(!app.ui.docking.layout.contains(&Panel::Layers) && !app.ui.docking.layout.contains(&Panel::Properties));
        // Reading the properties is not an edit and leaves the selection alone.
        assert_eq!(app.session.selection().len(), 1);
        glue.apply(&mut app, message(HostMessage::Panel { name: PanelName::Layers, open: false }));
        glue.apply(&mut app, message(HostMessage::Panel { name: PanelName::Properties, open: false }));
        frame(&mut app, &mut glue);
        assert_eq!(app.canvas.rect.expect("the drawing was drawn").width(), full);
    }

    #[test]
    fn tool_state_follows_grouping_without_a_selection_change() {
        let (mut app, mut glue) = sample_app();
        let two: Vec<String> = handles_of(&mut app).into_iter().take(2).collect();
        glue.apply(&mut app, message(HostMessage::Select { handles: two.clone() }));
        assert!(glue.events(&app).iter().any(|e| matches!(e, ViewerMessage::State(state) if state.can_group && !state.can_ungroup)));
        // Grouping through the engine leaves the selection as it is: only the drawing changed.
        app.run("group", json!({"handles": two})).expect("group");
        let events = glue.events(&app);
        assert!(events.iter().any(|e| matches!(e, ViewerMessage::State(state) if state.can_ungroup)), "{events:?}");
        // Nothing changed since: nothing is reported again.
        assert!(glue.events(&app).is_empty());
    }

    #[test]
    fn theme_sets_mode_and_canvas_colours() {
        let (mut app, mut glue) = sample_app();
        glue.apply(&mut app, message(HostMessage::Theme { dark: false, background: Some([255, 255, 255]), selection: None }));
        assert_eq!(app.ui.theme, cadcraft_ui_egui::theme::ThemePref::Light);
        assert_eq!(app.options.model_background, Some(egui::Color32::from_rgb(255, 255, 255)));
        assert_eq!(app.options.selection_color, None);
    }

    fn entity_count(app: &CadApp) -> usize {
        app.session.doc().map(cadcraft_doc::Drawing::entity_count).unwrap_or_default()
    }

    #[test]
    fn the_user_cannot_draw_or_open_but_the_host_can_open() {
        let (mut app, mut glue) = sample_app();
        let bytes = sample_dxf(&mut app, &glue);
        let (count, documents) = (entity_count(&app), app.session.docs.len());
        // Typed (the canvas sends keys to the command line), started from a menu or shortcut, or
        // run with parameters: refused, and the drawing is as it was.
        let _ = app.cmdline("line 0,0 10,10 ");
        let _ = app.cmdline("l 0,0 10,10 ");
        app.start("line");
        assert!(app.run("line", json!({"points": [[0, 0], [10, 10]]})).is_err());
        assert!(app.session.running.is_none());
        assert_eq!(entity_count(&app), count);
        app.start("open");
        app.start("ui.open");
        app.start("ui.sample");
        app.start("close");
        let data = cadcraft_engine::cmd::file::base64_encode(&bytes);
        assert!(app.run("open", json!({"data": data, "name": "mine.dxf"})).is_err());
        assert_eq!(app.session.docs.len(), documents);
        // Selecting and zooming are the user's.
        let first = handles_of(&mut app).into_iter().next().expect("the sample has entities");
        app.run("select", json!({"handles": [first]})).expect("select");
        app.run("zoom.extents", json!({})).expect("zoom");
        // The host page opens drawings.
        let replies = glue.apply(&mut app, open("plan.dxf", bytes));
        assert!(matches!(&replies[..], [ViewerMessage::Opened { name, .. }] if name == "plan.dxf"));
        assert_eq!(app.session.docs.len(), 1);
    }

    /// The viewer's own interface (status bar, palettes, canvas) runs nothing the policy refuses
    /// while the user only looks: no refusal every frame, no part of it silently dead.
    #[test]
    fn ordinary_frames_run_nothing_the_policy_refuses() {
        let (mut app, mut glue) = sample_app();
        restrict_ui(&mut app.ui);
        glue.apply(&mut app, message(HostMessage::Panel { name: PanelName::Layers, open: true }));
        glue.apply(&mut app, message(HostMessage::Panel { name: PanelName::Properties, open: true }));
        let first = handles_of(&mut app).into_iter().next().expect("the sample has entities");
        glue.apply(&mut app, message(HostMessage::Select { handles: vec![first] }));
        let ctx = egui::Context::default();
        for _ in 0..3 {
            let raw =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                app.logic(ui.ctx());
                restrict_ui(&mut app.ui);
                app.ui(ui);
            });
            out.textures_delta.clear();
            let _ = glue.events(&app);
        }
        let refused: Vec<&String> = app.session.log.iter().filter(|l| l.contains("is not available here")).collect();
        assert!(refused.is_empty(), "{refused:?}");
    }

    #[test]
    fn a_failing_host_call_leaves_the_user_restricted() {
        let (mut app, mut glue) = sample_app();
        let replies = glue.apply(&mut app, open("notes.dxf", b"not a drawing".to_vec()));
        assert!(matches!(&replies[..], [ViewerMessage::Error { .. }]));
        assert!(!glue.host.active());
        assert!(app.run("line", json!({"points": [[0, 0], [10, 10]]})).is_err());
        assert!(app.run("open", json!({"data": "", "name": "x.dxf"})).is_err());
    }

    #[test]
    fn highlight_is_kept_zoomed_to_and_cleared_by_open() {
        let (mut app, mut glue) = sample_app();
        let boxes = vec![HighlightBox { bounds: [10.0, 10.0, 20.0, 10.0], color: None, id: None }];
        let replies = glue.apply(&mut app, message(HostMessage::Highlight { boxes: boxes.clone(), zoom: true }));
        assert!(replies.is_empty());
        assert_eq!(glue.highlights(), &boxes[..]);
        // A line (no height): the view is fitted around it, not collapsed onto it.
        let view = app.session.state().expect("a drawing").view();
        assert!(view.height.is_finite() && view.height > 0.0, "{view:?}");
        assert!((view.center.x - 15.0).abs() < 1e-6 && (view.center.y - 10.0).abs() < 1e-6, "{view:?}");
        assert!(view.height < 20.0, "fitted to the line: {view:?}");
        // An empty list clears.
        glue.apply(&mut app, message(HostMessage::Highlight { boxes: boxes.clone(), zoom: false }));
        glue.apply(&mut app, message(HostMessage::Highlight { boxes: vec![], zoom: true }));
        assert!(glue.highlights().is_empty());
        // Opening a drawing clears them too.
        glue.apply(&mut app, message(HostMessage::Highlight { boxes, zoom: false }));
        let bytes = sample_dxf(&mut app, &glue);
        glue.apply(&mut app, open("plan.dxf", bytes));
        assert!(glue.highlights().is_empty());
    }
}
