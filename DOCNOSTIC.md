# DocNostic embed mode

This branch (`docnostic`) is upstream CADCraft plus an embed mode for DocNostic: with `?embed=1`
inside a same-origin `<iframe>`, the web app runs as a restricted viewer that the framing page
drives over `postMessage`. Without the flag, or outside a same-origin frame, the build behaves
like upstream.

Base: upstream tag `v0.20.1`. Nothing under `crates/` is changed.

## What the branch changes

| File | What |
|---|---|
| `apps/cadcraft-web/src/embed/protocol.rs` | The messages (pure, tested natively) |
| `apps/cadcraft-web/src/embed/bridge.rs` | `postMessage` transport, same origin only (wasm) |
| `apps/cadcraft-web/src/embed/glue.rs` | What each message does; which changes are reported |
| `apps/cadcraft-web/src/embed/mod.rs` | Module root, `Incoming` |
| `apps/cadcraft-web/src/main.rs` | Embed mode in the shell (`Embed`, `start`) |
| `apps/cadcraft-web/Cargo.toml` | `cadcraft-doc`, `cadcraft-geom` |
| `apps/cadcraft-web/harness/*.html` | Test host pages. Fork only: not shipped, not for upstream (they contain JavaScript) |
| `DOCNOSTIC.md` | This file |

## In embed mode

- The editor's menu bar, toolbar, file tabs, command line, Tool Sets and ViewCube are off. The
  status bar stays. Layers and Properties start closed; the host opens them.
- One drawing at a time: `cadcraft.open` replaces the open drawing.
- Saving and exporting answer "Saving is not available in this viewer"; nothing is written.
  `window.cadcraft` is not installed. Stored preferences and panel layouts are neither read nor
  written.
- Only the host page opens drawings: dropped files and the file picker are off. A drawing over
  the app's size limit is refused by its length, before it is copied.
- A drawing saved with a view opens on that view; otherwise it is fitted. A zoom the host sends
  after `cadcraft.open` wins over that fit.
- Messages are taken only from `window.parent` at our own origin, and replies go only there.
  Malformed or unknown messages are dropped without a reply.
- Not yet: blocking commands the user types or triggers by shortcut (needs a small engine hook),
  and the highlight overlay.

## Messages

All have a `type`. Handles are upper-case hex strings (DXF group 5). Bounds are
`[x0, y0, x1, y1]` in drawing units.

Host page to viewer:

| `type` | Fields | Effect |
|---|---|---|
| `cadcraft.open` | `name`, `bytes` (`ArrayBuffer` or `Uint8Array`) | Open a DXF, replacing the open drawing |
| `cadcraft.theme` | `mode` (`light`, `dark`), `background?`, `selection?` (`#rrggbb`) | Appearance |
| `cadcraft.zoomExtents` | | Fit the drawing |
| `cadcraft.zoomBy` | `factor` (> 0; above 1 zooms in) | Zoom about the view centre |
| `cadcraft.zoomTo` | `bounds` or `handles` | Fit a box or entities. Unknown handles are skipped |
| `cadcraft.select` | `handles` (`[]` clears) | Set the selection. Unknown handles are skipped |
| `cadcraft.command` | `id` (`explode`, `group`, `ungroup`, `pan`, `select`), `requestId` | Run on the selection, or switch the pointer |
| `cadcraft.panel` | `name` (`layers`, `properties`), `open` | Show or hide a palette |

Viewer to host page:

| `type` | Fields | When |
|---|---|---|
| `cadcraft.ready` | `version`, `capabilities` | The viewer is listening |
| `cadcraft.opened` | `name`, `extents`, `units`, `layers[] {name, on}`, `savedView` | A drawing opened. `savedView`: it opened on the view it was saved with (send `cadcraft.zoomExtents` to fit it instead) |
| `cadcraft.error` | `message`, `fatal` | A drawing could not be opened |
| `cadcraft.selection` | `handles`, `bounds`, `count`, `source` (`user`, `host`) | The selection changed |
| `cadcraft.commandDone` | `requestId`, `ok`, `handles`, `error?` | A `cadcraft.command` finished; explode returns the new handles |
| `cadcraft.state` | `canExplode`, `canGroup`, `canUngroup`, `panels {layers, properties}` | Tool state changed |

## Build and try

```sh
cd apps/cadcraft-web
trunk build --release                       # output: ../../dist/web (see Trunk.toml)
mkdir -p <dist>/harness && cp harness/*.html <dist>/harness/
python3 -m http.server 8771 -d <dist>
# http://localhost:8771/harness/harness.html
```

`harness.html` frames the viewer with `?embed=1`, has a button per message and logs everything
it receives. `wrong.html` frames it from another origin (open it on `localhost`; it frames
`127.0.0.1`) and must get the normal app and no messages.

## Checks before a commit

```sh
cargo test -p cadcraft-web
cargo clippy -p cadcraft-web --all-targets -- -D warnings
cargo clippy -p cadcraft-web --no-deps --target wasm32-unknown-unknown -- -D warnings
cargo fmt -p cadcraft-web --check
```

`--no-deps` on the wasm lint: upstream's `cadcraft-ui-egui` does not pass `-D warnings` on wasm
with every toolchain, and this branch does not touch it.

## Moving to a newer upstream release

```sh
git fetch origin --tags
git rebase --onto <new-tag> <old-tag> docnostic
```

Then run the checks, rebuild, and go through the manual checklist below. The branch touches
upstream code only in `apps/cadcraft-web/src/main.rs` and `Cargo.toml`; conflicts can only be there.
After a rebase, re-check the command ids and parameters `glue.rs` uses (`command_for`, `open`,
`close`, `ui.dock`, `pan`): they are the engine's and may change between releases.

## Manual checklist

Run against `harness.html` in Chrome after every rebase. Results of the last run are in the
table; the date and upstream tag are in its heading.

### Last run: 2026-11-20, upstream v0.20.1, Chrome on macOS, `spike/fixtures/plan.dxf`

| # | Check | Result |
|---|---|---|
| 1 | `cadcraft.ready` with version and capabilities; no menu bar, toolbar, file tabs, command line, Start page | Pass |
| 2 | Open a DXF: one `cadcraft.opened` with extents `[0,-2000,40000,27000]`, units `mm`, 10 layers; drawing shown | Pass |
| 3 | Zoom extents, zoom by factor; wheel and drag still work | Pass |
| 4 | Click an entity: `cadcraft.selection` with `source: "user"`. Select by handle: `source: "host"`. An unknown handle beside a known one is skipped | Pass |
| 5 | Explode a selected block reference: `commandDone` `ok: true` with 3 new handles. Group two of them, then ungroup; `cadcraft.state` follows each step. Explode with nothing selected: `ok: false`, "Nothing is selected" | Pass |
| 6 | Layers and Properties open and close; `cadcraft.state.panels` follows | Pass |
| 7 | Two `cadcraft.open` in a row: one `cadcraft.opened` each, the last drawing is shown | Pass |
| 8 | A file that is not a drawing: `cadcraft.error` with `fatal: false`; the next open works | Pass |
| 9 | Framed from another origin: the normal app, no reaction to messages, nothing posted to the parent | Pass |
| 10 | Without embed mode the app is the upstream app (seen in check 9: menus, toolbar, Tool Sets) | Pass |
| 11 | Save shortcut in the viewer: no download, no error | Pass |
| 12 | Typing `LINE` + Enter with the canvas focused starts the LINE command and draws | **Open**: typed commands and shortcuts are not blocked yet. Needs the engine policy hook |

Also seen: a malformed message (`handles: ["zz"]`) and a `cadcraft.open` without bytes are dropped
without a reply. In a sandboxed frame without `allow-same-origin` the app does not start at all
(upstream behaviour: it needs storage).
