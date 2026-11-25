//! The embed transport: `postMessage` with the page that frames CADCraft. Same origin only: a
//! message is taken only from `window.parent` at our own origin, and replies go only there.

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::Closure;

use super::Incoming;
use super::protocol::{ViewerMessage, parse_host_message};

struct Inner {
    parent: web_sys::Window,
    origin: String,
    queue: RefCell<Vec<Incoming>>,
}

#[derive(Clone)]
pub struct Bridge {
    inner: Rc<Inner>,
}

/// The framing page and our shared origin, or `None` when CADCraft is not framed or the parent is
/// on another origin (reading a cross-origin parent's location throws, which is the check).
fn same_origin_parent() -> Option<(web_sys::Window, String)> {
    let window = web_sys::window()?;
    let parent = window.parent().ok().flatten()?;
    if js_sys::Object::is(&parent, &window) {
        return None;
    }
    let origin = window.location().origin().ok()?;
    (parent.location().origin().ok()? == origin).then_some((parent, origin))
}

/// True when a page on our own origin frames CADCraft: the only place embed mode runs.
pub fn framed_by_same_origin() -> bool {
    same_origin_parent().is_some()
}

impl Bridge {
    /// The bridge to the framing page; `None` unless [`framed_by_same_origin`].
    pub fn connect(ctx: &egui::Context) -> Option<Bridge> {
        let window = web_sys::window()?;
        let (parent, origin) = same_origin_parent()?;
        let inner = Rc::new(Inner { parent, origin, queue: RefCell::new(Vec::new()) });
        let listener = {
            let inner = inner.clone();
            let ctx = ctx.clone();
            Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
                if let Some(incoming) = inner.read(&event) {
                    inner.queue.borrow_mut().push(incoming);
                    ctx.request_repaint();
                }
            })
        };
        window.add_event_listener_with_callback("message", listener.as_ref().unchecked_ref()).ok()?;
        // The listener lives as long as the page.
        listener.forget();
        Some(Bridge { inner })
    }

    /// Messages received since the last call, in arrival order.
    pub fn take(&self) -> Vec<Incoming> {
        self.inner.queue.take()
    }

    pub fn post(&self, message: &ViewerMessage) {
        let text = message.to_json().to_string();
        match js_sys::JSON::parse(&text) {
            Ok(value) => {
                if let Err(error) = self.inner.parent.post_message(&value, &self.inner.origin) {
                    log::warn!("embed: could not post to the host page: {error:?}");
                }
            }
            Err(error) => log::warn!("embed: could not build a message for the host page: {error:?}"),
        }
    }
}

impl Inner {
    /// The message an event carries, or `None` when it is not ours to act on: another window or
    /// origin sent it, or it is not a well-formed embed message.
    fn read(&self, event: &web_sys::MessageEvent) -> Option<Incoming> {
        if event.origin() != self.origin {
            return None;
        }
        let source = event.source()?;
        if !js_sys::Object::is(&source, &self.parent) {
            return None;
        }
        let data = event.data();
        if !data.is_object() {
            return None;
        }
        let field = |name: &str| js_sys::Reflect::get(&data, &name.into()).ok();
        let kind = field("type")?.as_string()?;
        if kind == "cadcraft.open" {
            // The drawing can be large: read its name and bytes directly, never through JSON.
            let name = field("name")?.as_string()?;
            let message = parse_host_message(&serde_json::json!({"type": kind, "name": name}))?;
            // A drawing too large to open is refused by its length, before it is copied into the
            // app's memory: a failed allocation aborts the viewer. A missing or unreadable
            // `bytes` still reaches the app, which answers with an error.
            let data = field("bytes");
            let refused = data.as_ref().and_then(byte_length).and_then(|len| cadcraft_ui_egui::CadApp::open_size_error(&name, len));
            let bytes = if refused.is_some() { None } else { data.as_ref().and_then(bytes_of) };
            return Some(Incoming { message, bytes, refused });
        }
        let text = js_sys::JSON::stringify(&data).ok()?.as_string()?;
        let message = parse_host_message(&serde_json::from_str(&text).ok()?)?;
        Some(Incoming { message, bytes: None, refused: None })
    }
}

/// The length of an `ArrayBuffer` or a `Uint8Array`, without copying it.
fn byte_length(value: &wasm_bindgen::JsValue) -> Option<u64> {
    if let Some(array) = value.dyn_ref::<js_sys::Uint8Array>() {
        return Some(u64::from(array.byte_length()));
    }
    value.dyn_ref::<js_sys::ArrayBuffer>().map(|buffer| u64::from(buffer.byte_length()))
}

/// The bytes of an `ArrayBuffer` or a `Uint8Array`.
fn bytes_of(value: &wasm_bindgen::JsValue) -> Option<Vec<u8>> {
    if let Some(array) = value.dyn_ref::<js_sys::Uint8Array>() {
        return Some(array.to_vec());
    }
    value.dyn_ref::<js_sys::ArrayBuffer>().map(|buffer| js_sys::Uint8Array::new(buffer).to_vec())
}
