//! The embed messages: what the host page may send and what the viewer reports. Pure, so it is
//! tested natively. Type names are `cadcraft.<name>` (upstream's host mode uses `cadcraft:<name>`).

use serde_json::{Value, json};

/// `[x0, y0, x1, y1]` in drawing units, minimum corner first.
pub type Bounds = [f64; 4];

/// The capabilities `cadcraft.ready` advertises.
pub const CAPABILITIES: [&str; 7] = ["open", "zoom", "select", "selection", "command", "panel", "theme"];

/// True when the page query has the whole parameter `embed=1`.
pub fn embed_requested(query: &str) -> bool {
    query.trim_start_matches('?').split('&').any(|p| p == "embed=1")
}

#[derive(Debug, Clone, PartialEq)]
pub enum ZoomTarget {
    Bounds(Bounds),
    Handles(Vec<String>),
}

/// The commands the host may run on the selection or the pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedCommand {
    Explode,
    Group,
    Ungroup,
    Pan,
    Select,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelName {
    Layers,
    Properties,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostMessage {
    /// The drawing's bytes travel beside the message; the bridge takes them from the event.
    Open {
        name: String,
    },
    Theme {
        dark: bool,
        background: Option<[u8; 3]>,
        selection: Option<[u8; 3]>,
    },
    ZoomExtents,
    ZoomBy {
        factor: f64,
    },
    ZoomTo(ZoomTarget),
    Select {
        handles: Vec<String>,
    },
    Command {
        id: EmbedCommand,
        request_id: String,
    },
    Panel {
        name: PanelName,
        open: bool,
    },
}

/// A handle as the wire carries it: ASCII hex digits, upper-cased. `None` for anything else.
fn handle(v: &Value) -> Option<String> {
    let s = v.as_str()?.trim();
    (!s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit())).then(|| s.to_ascii_uppercase())
}

/// A list of handles; one bad entry rejects the list.
fn handles(v: &Value) -> Option<Vec<String>> {
    v.as_array()?.iter().map(handle).collect()
}

/// Four finite numbers, returned minimum corner first.
fn bounds(v: &Value) -> Option<Bounds> {
    let a = v.as_array()?;
    if a.len() != 4 {
        return None;
    }
    let mut n = [0.0; 4];
    for (slot, item) in n.iter_mut().zip(a) {
        *slot = item.as_f64().filter(|f| f.is_finite())?;
    }
    Some([n[0].min(n[2]), n[1].min(n[3]), n[0].max(n[2]), n[1].max(n[3])])
}

/// `#rrggbb`.
fn colour(v: &Value) -> Option<[u8; 3]> {
    let hex = v.as_str()?.strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let part = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    Some([part(0)?, part(2)?, part(4)?])
}

/// An optional colour: absent is fine, present and malformed rejects the message.
fn optional_colour(data: &Value, key: &str) -> Option<Option<[u8; 3]>> {
    match data.get(key) {
        None | Some(Value::Null) => Some(None),
        Some(v) => colour(v).map(Some),
    }
}

/// Reads one message from the host page. `None` for anything that is not a well-formed embed
/// message: such messages are dropped without a reply.
pub fn parse_host_message(data: &Value) -> Option<HostMessage> {
    let kind = data.get("type")?.as_str()?.strip_prefix("cadcraft.")?;
    Some(match kind {
        "open" => HostMessage::Open { name: data.get("name")?.as_str()?.to_string() },
        "theme" => {
            let dark = match data.get("mode")?.as_str()? {
                "dark" => true,
                "light" => false,
                _ => return None,
            };
            HostMessage::Theme { dark, background: optional_colour(data, "background")?, selection: optional_colour(data, "selection")? }
        }
        "zoomExtents" => HostMessage::ZoomExtents,
        "zoomBy" => HostMessage::ZoomBy { factor: data.get("factor")?.as_f64().filter(|f| f.is_finite() && *f > 0.0)? },
        "zoomTo" => HostMessage::ZoomTo(match (data.get("bounds"), data.get("handles")) {
            (Some(b), _) => ZoomTarget::Bounds(bounds(b)?),
            (None, Some(h)) => ZoomTarget::Handles(handles(h)?),
            (None, None) => return None,
        }),
        "select" => HostMessage::Select { handles: handles(data.get("handles")?)? },
        "command" => {
            let id = match data.get("id")?.as_str()? {
                "explode" => EmbedCommand::Explode,
                "group" => EmbedCommand::Group,
                "ungroup" => EmbedCommand::Ungroup,
                "pan" => EmbedCommand::Pan,
                "select" => EmbedCommand::Select,
                _ => return None,
            };
            HostMessage::Command { id, request_id: data.get("requestId")?.as_str()?.to_string() }
        }
        "panel" => {
            let name = match data.get("name")?.as_str()? {
                "layers" => PanelName::Layers,
                "properties" => PanelName::Properties,
                _ => return None,
            };
            HostMessage::Panel { name, open: data.get("open")?.as_bool()? }
        }
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayerInfo {
    pub name: String,
    pub on: bool,
}

/// What decides which host tools are enabled.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolState {
    pub can_explode: bool,
    pub can_group: bool,
    pub can_ungroup: bool,
    pub layers_open: bool,
    pub properties_open: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ViewerMessage {
    Ready {
        version: String,
    },
    /// `saved_view`: the drawing opened on the view it was saved with, not fitted to the window.
    Opened {
        name: String,
        extents: Option<Bounds>,
        units: String,
        layers: Vec<LayerInfo>,
        saved_view: bool,
    },
    Error {
        message: String,
        fatal: bool,
    },
    Selection {
        handles: Vec<String>,
        bounds: Option<Bounds>,
        from_host: bool,
    },
    CommandDone {
        request_id: String,
        ok: bool,
        handles: Vec<String>,
        error: Option<String>,
    },
    State(ToolState),
}

impl ViewerMessage {
    /// The message as the host page receives it.
    pub fn to_json(&self) -> Value {
        match self {
            Self::Ready { version } => json!({"type": "cadcraft.ready", "version": version, "capabilities": CAPABILITIES}),
            Self::Opened { name, extents, units, layers, saved_view } => json!({
                "savedView": saved_view,
                "type": "cadcraft.opened",
                "name": name,
                "extents": extents,
                "units": units,
                "layers": layers.iter().map(|l| json!({"name": l.name, "on": l.on})).collect::<Vec<_>>(),
            }),
            Self::Error { message, fatal } => json!({"type": "cadcraft.error", "message": message, "fatal": fatal}),
            Self::Selection { handles, bounds, from_host } => json!({
                "type": "cadcraft.selection",
                "handles": handles,
                "bounds": bounds,
                "count": handles.len(),
                "source": if *from_host { "host" } else { "user" },
            }),
            Self::CommandDone { request_id, ok, handles, error } => {
                let mut message = json!({"type": "cadcraft.commandDone", "requestId": request_id, "ok": ok, "handles": handles});
                if let (Some(error), Some(fields)) = (error, message.as_object_mut()) {
                    fields.insert("error".to_string(), json!(error));
                }
                message
            }
            Self::State(state) => json!({
                "type": "cadcraft.state",
                "canExplode": state.can_explode,
                "canGroup": state.can_group,
                "canUngroup": state.can_ungroup,
                "panels": {"layers": state.layers_open, "properties": state.properties_open},
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn embed_flag_needs_the_exact_value() {
        assert!(embed_requested("?embed=1"));
        assert!(embed_requested("?webgl&embed=1"));
        assert!(!embed_requested(""));
        assert!(!embed_requested("?embed"));
        assert!(!embed_requested("?embed=0"));
        assert!(!embed_requested("?xembed=1"));
    }

    #[test]
    fn parses_each_host_message() {
        assert_eq!(parse_host_message(&json!({"type":"cadcraft.open","name":"plan.dxf"})), Some(HostMessage::Open { name: "plan.dxf".into() }));
        assert_eq!(parse_host_message(&json!({"type":"cadcraft.zoomExtents"})), Some(HostMessage::ZoomExtents));
        assert_eq!(parse_host_message(&json!({"type":"cadcraft.zoomBy","factor":1.25})), Some(HostMessage::ZoomBy { factor: 1.25 }));
        assert_eq!(
            parse_host_message(&json!({"type":"cadcraft.zoomTo","bounds":[3.0,4.0,1.0,2.0]})),
            Some(HostMessage::ZoomTo(ZoomTarget::Bounds([1.0, 2.0, 3.0, 4.0])))
        );
        assert_eq!(
            parse_host_message(&json!({"type":"cadcraft.zoomTo","handles":["1b7","2C"]})),
            Some(HostMessage::ZoomTo(ZoomTarget::Handles(vec!["1B7".into(), "2C".into()])))
        );
        assert_eq!(parse_host_message(&json!({"type":"cadcraft.select","handles":[]})), Some(HostMessage::Select { handles: vec![] }));
        assert_eq!(
            parse_host_message(&json!({"type":"cadcraft.command","id":"explode","requestId":"r1"})),
            Some(HostMessage::Command { id: EmbedCommand::Explode, request_id: "r1".into() })
        );
        assert_eq!(
            parse_host_message(&json!({"type":"cadcraft.panel","name":"layers","open":true})),
            Some(HostMessage::Panel { name: PanelName::Layers, open: true })
        );
        assert_eq!(
            parse_host_message(&json!({"type":"cadcraft.theme","mode":"dark","background":"#101418"})),
            Some(HostMessage::Theme { dark: true, background: Some([0x10, 0x14, 0x18]), selection: None })
        );
    }

    #[test]
    fn drops_malformed_messages() {
        for bad in [
            json!(null),
            json!("cadcraft.open"),
            json!({"name":"x"}),
            json!({"type":"cadcraft:open","name":"x"}),
            json!({"type":"cadcraft.open"}),
            json!({"type":"cadcraft.zoomBy","factor":0}),
            json!({"type":"cadcraft.zoomBy","factor":-2}),
            json!({"type":"cadcraft.zoomBy","factor":"big"}),
            json!({"type":"cadcraft.zoomTo"}),
            json!({"type":"cadcraft.zoomTo","bounds":[1,2,3]}),
            json!({"type":"cadcraft.zoomTo","bounds":[1,2,3,"x"]}),
            json!({"type":"cadcraft.select","handles":"1B7"}),
            json!({"type":"cadcraft.select","handles":["1B7","zz"]}),
            json!({"type":"cadcraft.command","id":"line","requestId":"r"}),
            json!({"type":"cadcraft.command","id":"explode"}),
            json!({"type":"cadcraft.panel","name":"files","open":true}),
            json!({"type":"cadcraft.theme","mode":"sepia"}),
            json!({"type":"cadcraft.unknown"}),
        ] {
            assert_eq!(parse_host_message(&bad), None, "{bad}");
        }
    }

    #[test]
    fn serialises_viewer_messages() {
        assert_eq!(
            ViewerMessage::Ready { version: "0.20.1".into() }.to_json(),
            json!({"type":"cadcraft.ready","version":"0.20.1","capabilities":CAPABILITIES})
        );
        assert_eq!(
            ViewerMessage::Selection { handles: vec!["1B7".into()], bounds: Some([0.0, 0.0, 2.0, 1.0]), from_host: false }.to_json(),
            json!({"type":"cadcraft.selection","handles":["1B7"],"bounds":[0.0,0.0,2.0,1.0],"count":1,"source":"user"})
        );
        assert_eq!(
            ViewerMessage::Selection { handles: vec![], bounds: None, from_host: true }.to_json(),
            json!({"type":"cadcraft.selection","handles":[],"bounds":null,"count":0,"source":"host"})
        );
        assert_eq!(
            ViewerMessage::CommandDone { request_id: "r1".into(), ok: false, handles: vec![], error: Some("nothing selected".into()) }.to_json(),
            json!({"type":"cadcraft.commandDone","requestId":"r1","ok":false,"handles":[],"error":"nothing selected"})
        );
        assert_eq!(
            ViewerMessage::State(ToolState { can_explode: true, ..Default::default() }).to_json(),
            json!({"type":"cadcraft.state","canExplode":true,"canGroup":false,"canUngroup":false,"panels":{"layers":false,"properties":false}})
        );
        assert_eq!(
            ViewerMessage::Opened {
                name: "plan.dxf".into(),
                extents: None,
                units: "mm".into(),
                layers: vec![LayerInfo { name: "0".into(), on: true }],
                saved_view: true
            }
            .to_json(),
            json!({"type":"cadcraft.opened","name":"plan.dxf","extents":null,"units":"mm","layers":[{"name":"0","on":true}],"savedView":true})
        );
    }
}
