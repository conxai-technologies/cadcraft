//! DocNostic embed mode (`?embed=1`): CADCraft as a restricted viewer inside a same-origin
//! iframe, driven by the host page over `postMessage`. See `DOCNOSTIC.md` at the repo root.

// Until `main.rs` uses the module (the wiring is the last step of the embed work).
#![allow(dead_code)]

pub mod protocol;
