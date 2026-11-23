//! DocNostic embed mode (`?embed=1`): CADCraft as a restricted viewer inside a same-origin
//! iframe, driven by the host page over `postMessage`. See `DOCNOSTIC.md` at the repo root.

#[cfg(target_arch = "wasm32")]
pub mod bridge;
pub mod glue;
pub mod overlay;
pub mod policy;
pub mod protocol;

/// One message from the host page; `bytes` is the drawing of a `cadcraft.open`. `refused` says
/// why the drawing was not taken in (too large to copy): the open then fails with that reason.
pub struct Incoming {
    pub message: protocol::HostMessage,
    pub bytes: Option<Vec<u8>>,
    pub refused: Option<String>,
}
