//! A host's command policy (`Session::set_command_policy`): an embedding host decides which
//! commands may run, and every way of starting one asks it first — programmatic execution, the
//! interactive start (typed names and aliases), grip edits and AutoLISP.

use cadcraft_engine::{Input, Session};
use serde_json::{Value, json};

/// A session with one line, and a policy that allows only what `allow` names.
fn session_allowing(allow: &'static [&'static str]) -> (Session, String) {
    let mut s = Session::new();
    let line = s.execute("line", &json!({ "points": [[0, 0], [10, 0]] })).expect("line");
    let handle = line["handles"][0].as_str().expect("a handle").to_string();
    s.set_command_policy(Some(Box::new(move |id: &str, _: &Value| allow.contains(&id))));
    (s, handle)
}

fn entities(s: &Session) -> usize {
    s.doc().map(cadcraft_engine::doc::Drawing::entity_count).unwrap_or_default()
}

#[test]
fn a_refused_command_is_an_error_and_leaves_the_drawing_alone() {
    let (mut s, _) = session_allowing(&["zoom.extents"]);
    let revision = s.state().expect("a drawing").revision;
    let refused = s.execute("circle", &json!({ "center": [0, 0], "radius": 5 }));
    let message = refused.expect_err("CIRCLE is refused").to_string();
    assert!(message.contains("CIRCLE"), "{message}");
    assert_eq!(entities(&s), 1);
    assert_eq!(s.state().expect("a drawing").revision, revision);
    assert!(s.state().expect("a drawing").undo.len() == 1, "no undo step for the refused command");
    // An allowed command still runs, and an unknown one is still unknown.
    s.execute("zoom.extents", &json!({})).expect("allowed");
    assert!(matches!(s.execute("nosuchcommand", &json!({})), Err(cadcraft_engine::EngineError::UnknownCommand(_))));
}

#[test]
fn a_refused_command_does_not_start_by_name_or_alias() {
    let (mut s, _) = session_allowing(&[]);
    for typed in ["line", "LINE", "l", "_.line", "'circle"] {
        assert!(s.start(typed).is_err(), "{typed} started");
        assert!(s.running.is_none(), "{typed} is running");
    }
    // Typed at the command line (with its points) it is refused too, and nothing is drawn.
    assert!(s.cmdline("line 0,0 5,5 ").is_err());
    assert!(s.cmdline("line {\"points\": [[0, 0], [5, 5]]}").is_err());
    assert_eq!(entities(&s), 1);
}

#[test]
fn a_refused_command_does_not_cancel_the_running_one() {
    let (mut s, _) = session_allowing(&["pan"]);
    s.start("pan").expect("pan starts");
    assert!(s.start("erase").is_err());
    assert_eq!(s.running.as_ref().map(|r| r.id.as_str()), Some("pan"));
}

#[test]
fn the_policy_sees_the_parameters() {
    let mut s = Session::new();
    s.execute("layer.new", &json!({ "name": "A" })).expect("layer");
    s.set_command_policy(Some(Box::new(|id: &str, p: &Value| id == "layer.set" && p.get("color").is_none())));
    s.execute("layer.set", &json!({ "name": "A", "on": false })).expect("visibility is allowed");
    assert!(s.execute("layer.set", &json!({ "name": "A", "color": 3 })).is_err());
    let a = s.doc().expect("a drawing").layers.iter().find(|l| l.name == "A").expect("layer A");
    assert!(!a.on);
    assert_ne!(a.color, cadcraft_engine::doc::color::Color::from_aci(3));
}

#[test]
fn grip_edits_ask_the_policy() {
    let (mut s, handle) = session_allowing(&[]);
    let h = cadcraft_engine::doc::Handle::parse_hex(&handle).expect("a handle");
    assert!(s.start_grip_edit(&[(h, 1)]).is_err());
    assert!(s.running.is_none());
    // Allowed, the same grip starts an edit.
    s.set_command_policy(Some(Box::new(|id: &str, _: &Value| id == "grip")));
    s.start_grip_edit(&[(h, 1)]).expect("grip edit");
    assert_eq!(s.running.as_ref().map(|r| r.id.as_str()), Some("grip"));
}

#[test]
fn lisp_asks_the_policy() {
    let (mut s, _) = session_allowing(&[]);
    assert!(s.lisp_eval("(setq cc-policy 1)", false, true, Vec::new()).is_err());
    assert!(matches!(s.lisp_var("CC-POLICY"), cadcraft_engine::lisp::value::Value::Nil));
    // Typed at the command line, the refusal is shown and nothing runs.
    let _ = s.cmdline("(setq cc-policy 2)");
    assert!(matches!(s.lisp_var("CC-POLICY"), cadcraft_engine::lisp::value::Value::Nil));
}

#[test]
fn enter_does_not_repeat_a_command_the_policy_now_refuses() {
    let mut s = Session::new();
    s.start("line").expect("line");
    s.cancel();
    s.set_command_policy(Some(Box::new(|_: &str, _: &Value| false)));
    let _ = s.input(Input::Enter);
    assert!(s.running.is_none());
}

#[test]
fn without_a_policy_everything_runs_as_before() {
    let (mut s, _) = session_allowing(&[]);
    s.set_command_policy(None);
    s.execute("circle", &json!({ "center": [0, 0], "radius": 5 })).expect("circle");
    s.start("line").expect("line");
    assert_eq!(entities(&s), 2);
    assert!(s.command_allowed("anything", &Value::Null));
}
