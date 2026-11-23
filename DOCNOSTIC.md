# DocNostic embed mode

This branch (`docnostic`) is upstream CADCraft plus an embed mode for DocNostic: with `?embed=1`
inside a same-origin `<iframe>`, the web app runs as a restricted viewer that the framing page
drives over `postMessage`. Without the flag, or outside a same-origin frame, the build behaves
like upstream.

Base: upstream tag `v0.20.1`. The changes under `crates/` are a few lines each, listed below
with the reason they cannot live in the web shell.

## What the branch changes

| File | What |
|---|---|
| `apps/cadcraft-web/src/embed/protocol.rs` | The messages (pure, tested natively) |
| `apps/cadcraft-web/src/embed/bridge.rs` | `postMessage` transport, same origin only (wasm) |
| `apps/cadcraft-web/src/embed/glue.rs` | What each message does; which changes are reported; marks the host's own calls |
| `apps/cadcraft-web/src/embed/policy.rs` | The command allow-list for the user (pure, tested natively) |
| `apps/cadcraft-web/src/embed/overlay.rs` | The highlight overlay: world box to screen, painting, the zoom window |
| `apps/cadcraft-web/src/embed/mod.rs` | Module root, `Incoming` |
| `apps/cadcraft-web/src/main.rs` | Embed mode in the shell (`Embed`, `start`, the overlay after each frame) |
| `apps/cadcraft-web/Cargo.toml` | `cadcraft-doc`, `cadcraft-geom` |
| `apps/cadcraft-web/harness/*.html` | Test host pages. Fork only: not shipped, not for upstream (they contain JavaScript) |
| `.github/workflows/docnostic-web-release.yml` | Fork only, never upstream (upstream CI calls the shared craft-common-actions workflows): the web release, see "Releasing" |
| `DOCNOSTIC.md` | This file |

Under `crates/`. Each is needed because the shell cannot reach the code path, and each could be
offered upstream on its own; the command policy is what storytold/craft-libs#609 asks for (a
host-controlled read-only session; PdfCraft has `Session::set_read_only`).

| File | Lines | What and why |
|---|---|---|
| `crates/engine/src/lib.rs` | +7 | `CommandPolicy` type and the `command_policy` field on `Session` (and its `None` in `Session::empty`). The session is the only object every command path shares |
| `crates/engine/src/session.rs` | +1 | `mod policy;` |
| `crates/engine/src/session/policy.rs` | new | `set_command_policy`, `command_allowed`, `check_command` |
| `crates/engine/src/session/execute.rs` | +1 | Check in `execute_guarded`: programmatic calls, menus, toolbar, palettes (`properties.set`, `layer.set`), shortcuts, `cmd {json}` lines |
| `crates/engine/src/session/interactive.rs` | +2 | Check in `start` right after the name resolves (aliases, `'`, `_`, `.` already resolved), before a running command is cancelled: typed names, Enter repeating the last command, scripts, LISP `(command …)` |
| `crates/engine/src/cmd/griphot.rs` | +1 | Check (`grip`) in `start_grip_edit`: grip edits set the running command directly, past `start` |
| `crates/engine/src/cmd/blocks/dyngrip.rs` | +1 | Same for dynamic block grips (`start_dyn_grip`) |
| `crates/engine/src/lisp.rs` | +2 | Check (`lisp`) in `lisp_start`: `entmake`, `entmod` and `entdel` change the drawing without any command, and `(expr)` typed at the command line, `c:` commands, APPLOAD and startup files all start there |
| `crates/engine/tests/command_policy.rs` | new | Tests of the above |
| `crates/ui-egui/src/menus.rs` | +5 | Check in `run_ui_command`: the UI-only commands (`ui.open`, `ui.sample`, dialogs, toggles) and the engine ids it intercepts (`close`, the file pickers, recent files) never reach the engine |
| `crates/ui-egui/src/lib.rs` | +5 −1 | `UiState::show_toolsets_strip` (default on, as upstream), checked where the strip is drawn: the shell cannot keep it off, `show_toolsets` off is what shows it |
| `crates/ui-egui/tests/command_policy.rs` | new | Tests of the above |

## In embed mode

- The editor's menu bar, toolbar, file tabs, command line, Tool Sets and ViewCube are off. The
  status bar stays. Layers and Properties start closed; the host opens them.
- One drawing at a time: `cadcraft.open` replaces the open drawing.
- The user can only look: view and navigate, select, measure, show and hide layers, undo, and
  explode or group. Every other command is refused however it is started (typed, alias,
  shortcut, context menu, toolbar, grip, palette edit, AutoLISP), with "<COMMAND> is not
  available here" on the (hidden) command line, and the drawing stays as it was. The allow-list
  is `ALLOWED` in `embed/policy.rs`:
  - view: `zoom`, `zoom.all`, `zoom.dynamic`, `zoom.extents`, `zoom.in`, `zoom.object`,
    `zoom.out`, `zoom.previous`, `zoom.realtime`, `zoom.window`, `pan`, `pan.up`, `pan.down`,
    `pan.left`, `pan.right`, `regen`, `regenall`, `redraw`, `view.get`, and the status bar's
    layout tabs `layout.set`, `mspace`, `pspace`
  - selection: `select`, `selectall`, `selectsimilar`, `ai_deselect`
  - inquiry: `list`, `dist`, `id`, `area`, `measuregeom`, `drawing.inspect`, `entities`,
    `properties` (reading; `properties.set` is refused)
  - `u`, `undo`, `redo`, `mredo`; `explode`, `burst`, `group`, `ungroup`
  - layer visibility: `layon`, `layoff`, `layfrz`, `laythw`, `layiso`, `layuniso`, and
    `layer.set` only with `name` and boolean `on` / `frozen` (the Layers palette's bulb and
    snowflake; lock, colour, rename and the rest are refused)
  - the palettes: `ui.dock`, `ui.panel.layers`, `ui.panel.properties`

  Typed names resolve as the engine resolves them (`Z` is `zoom`, `'PAN` and `_PAN` are `pan`,
  `-GROUP` stays `-group` and is refused). The status bar's drafting toggles (grid, snap, ortho,
  osnap…) are refused: they do nothing in a viewer. Context menu entries for refused commands
  still show and do nothing when chosen.
- The host page's own messages are not limited: `glue.rs` runs each in a `HostScope`, which the
  policy lets through and which is reset when it ends, also on an early return.
- Saving and exporting answer "Saving is not available in this viewer"; nothing is written.
  `window.cadcraft` is not installed. Stored preferences and panel layouts are neither read nor
  written.
- Only the host page opens drawings: dropped files and the file picker are off. A drawing over
  the app's size limit is refused by its length, before it is copied.
- A drawing saved with a view opens on that view; otherwise it is fitted. A zoom the host sends
  after `cadcraft.open` wins over that fit.
- Messages are taken only from `window.parent` at our own origin, and replies go only there.
  Malformed or unknown messages are dropped without a reply.
- `cadcraft.highlight` boxes are painted over the drawing by the shell after each frame, clipped
  to the drawing area and below menus and floating windows, on the Model tab only (they are
  model-space coordinates). A box smaller than 12 points (a line, a point) is drawn 12 points
  across. A failed `cadcraft.open` keeps them (the drawing stays); a successful one clears them.
- The Tool Sets strip at the left edge is off (`UiState::show_toolsets_strip`).
- Not yet: hiding the context menu entries the policy refuses (they do nothing), and the
  highlight on layout tabs.

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
| `cadcraft.command` | `id` (`explode`, `group`, `ungroup`, `pan`, `select`), `requestId` | Run on the selection, or switch the pointer: with `pan` the primary button drags the view until `select`. The middle button drags it and the wheel zooms with either pointer |
| `cadcraft.panel` | `name` (`layers`, `properties`), `open` | Show or hide a palette |
| `cadcraft.highlight` | `boxes [{bounds, color?}]` (`#rrggbb`, default `#ff9f1a`; at most 2000, more rejects the message; `[]` clears), `zoom?` (bool) | Paint boxes over the drawing; `zoom: true` also fits the view to them with a margin (a line or a point gets one too) |

Viewer to host page:

| `type` | Fields | When |
|---|---|---|
| `cadcraft.ready` | `version`, `capabilities` (`open`, `zoom`, `select`, `selection`, `command`, `panel`, `theme`, `highlight`) | The viewer is listening |
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
cargo test -p cadcraft-engine --test command_policy
cargo test -p cadcraft-ui-egui --test command_policy
cargo test -p cadcraft-engine --lib
cargo test -p cadcraft-ui-egui --lib
cargo clippy -p cadcraft-web --all-targets -- -D warnings
cargo clippy -p cadcraft-web --no-deps --target wasm32-unknown-unknown -- -D warnings
cargo clippy -p cadcraft-engine -p cadcraft-ui-egui -- -D warnings
cargo clippy -p cadcraft-engine -p cadcraft-ui-egui --test command_policy -- -D warnings
cargo fmt -p cadcraft-web -p cadcraft-engine -p cadcraft-ui-egui --check
```

`--no-deps` on the wasm lint: upstream's `cadcraft-ui-egui` does not pass `-D warnings` on wasm
with every toolchain (dead code in `canvas/rebuild.rs`), and this branch does not change that.
On macOS, `crates/engine/tests/read_file_unicode.rs` fails upstream too (the file system refuses
the non-Unicode file name the test writes).

## Moving to a newer upstream release

```sh
git fetch origin --tags
git rebase --onto <new-tag> <old-tag> docnostic
```

Then run the checks, rebuild, and go through the manual checklist below. The branch touches
upstream code in `apps/cadcraft-web/src/main.rs`, `apps/cadcraft-web/Cargo.toml` and the
`crates/` files in the table above; conflicts can only be there, and each change there is a line
or two at a choke point. After a rebase:

- re-check the command ids and parameters `glue.rs` uses (`command_for`, `open`, `close`,
  `ui.dock`, `pan`): they are the engine's and may change between releases;
- `every_allowed_id_is_a_command` (in `policy.rs`) fails when an allowed id was renamed;
- look for new ways to start a command or change the drawing that bypass `Session::execute`,
  `Session::start`, the grip starts and `lisp_start` (`grep -rn 'running = Some(Running' crates/engine/src`
  and `doc_mut()` outside `cmd/`), and for new engine ids handled in `run_ui_command`;
- compare the trunk version, checksum and action pins in
  `.github/workflows/docnostic-web-release.yml` with upstream's shared release workflow.

## Releasing

Fork only. Pushing a tag `docnostic-v<version>` (for example `docnostic-v0.20.1-1`: the upstream
release and our build number) runs `.github/workflows/docnostic-web-release.yml`: it builds the
web app with `packaging/web/package.sh` (trunk release build, wasm-opt), checks the zip, and
publishes a GitHub release for the tag with

- `cadcraft-web-<tag>.zip`: the site in a top-level folder `cadcraft-web-<tag>/` (`index.html`,
  the hashed `.js` and `_bg.wasm`, `snippets/`, the licence files, `HOSTING.md`, `_headers`,
  `.htaccess`), as upstream's `cadcraft-web-<version>.zip`; no `harness/`;
- `SHA256SUMS.txt`.

```sh
git tag docnostic-v0.20.1-1 && git push <fork-remote> docnostic-v0.20.1-1
```

The app inside still reports its Cargo version (`cadcraft.ready.version`); only the package
carries the tag. Not run yet: the fork's GitHub repository does not exist.

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
| 12 | Typing `LINE` + Enter with the canvas focused starts the LINE command and draws | **Open** at this run; the command policy now refuses it (not yet re-run in the browser) |
| 13 | Typed `LINE`/`L` + Enter, Delete on a selection, Cmd+W, Cmd+O, Cmd+S, a grip drag, `(entmake …)`: nothing changes; `ZOOM`/`Z`, wheel, selection, Layers bulb and snowflake still work; the lock icon does nothing | Not run yet |
| 14 | `cadcraft.highlight` with a box, a line (zero height) and `zoom: true`: orange outlines, the line at least 12 px, the view fitted; `[]` clears; a new `cadcraft.open` clears | Not run yet |
| 15 | No Tool Sets strip at the left edge of the canvas | Not run yet |

Also seen: a malformed message (`handles: ["zz"]`) and a `cadcraft.open` without bytes are dropped
without a reply. In a sandboxed frame without `allow-same-origin` the app does not start at all
(upstream behaviour: it needs storage).
