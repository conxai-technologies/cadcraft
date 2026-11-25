//! The host's command policy (`Session::set_command_policy`) covers the UI-only commands too
//! (`run_ui_command`: dialogs, file pickers, CLOSE), and a host can hide the Tool Sets strip.

use cadcraft_engine::Session;
use cadcraft_ui_egui::{CadApp, Services};
use serde_json::{Value, json};

/// A frame of the whole interface, as the app runs it.
fn frame(app: &mut CadApp, ctx: &egui::Context) {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
}

fn refusing_all() -> CadApp {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.session.set_command_policy(Some(Box::new(|_: &str, _: &Value| false)));
    app
}

#[test]
fn a_refused_ui_command_does_nothing_and_says_so() {
    let mut app = refusing_all();
    let toolbar = app.ui.show_toolbar;
    assert!(app.run("ui.toggle.toolbar", json!({})).is_err());
    app.start("ui.toggle.toolbar");
    let _ = app.cmdline("ui.toggle.toolbar");
    assert_eq!(app.ui.show_toolbar, toolbar);
    assert!(app.session.log.iter().any(|l| l.contains("UI.TOGGLE.TOOLBAR is not available")), "{:?}", app.session.log);
    // The sample, CLOSE (Cmd+W) and the open picker are UI commands as well.
    let title = app.session.state().expect("Drawing1").title.clone();
    for id in ["ui.sample", "close", "ui.open", "closeall"] {
        app.start(id);
    }
    assert_eq!(app.session.docs.len(), 1);
    assert_eq!(app.session.state().expect("still open").title, title);
}

#[test]
fn an_allowed_ui_command_runs() {
    let mut app = CadApp::new(Session::new(), Services::default());
    app.session.set_command_policy(Some(Box::new(|id: &str, _: &Value| id == "ui.toggle.toolbar")));
    let toolbar = app.ui.show_toolbar;
    app.run("ui.toggle.toolbar", json!({})).expect("allowed");
    assert_ne!(app.ui.show_toolbar, toolbar);
}

#[test]
fn the_tool_sets_strip_can_be_hidden() {
    let left = |strip: bool| {
        let ctx = egui::Context::default();
        let mut app = CadApp::new(Session::new(), Services::default());
        app.ui.show_toolsets = false;
        app.ui.show_toolsets_strip = strip;
        frame(&mut app, &ctx);
        frame(&mut app, &ctx);
        app.canvas.rect.expect("the canvas was laid out").left()
    };
    assert!(cadcraft_ui_egui::UiState::default().show_toolsets_strip, "shown by default, as upstream");
    let (shown, hidden) = (left(true), left(false));
    assert!(shown - hidden >= 20.0, "the strip takes its width from the canvas: {shown} vs {hidden}");
}
