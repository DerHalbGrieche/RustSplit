# Architecture

RustSplit is split into five crates with a one-way dependency graph:

```
rustsplit (binary)
 ├── rustsplit-ui ─────────────┐
 ├── rustsplit-server ─────────┤
 ├── rustsplit-auto-splitting ─┤   (optional, `auto-splitting` feature)
 └── rustsplit-core ◄──────────┘
```

## rustsplit-core

The timer and data model. It has no UI dependencies and is deterministic when
given a custom `Clock`.

- `time.rs` — `TimeSpan` (i64 nanoseconds), dual real/game-time `Time`,
  LiveSplit-compatible parsing and precise `.NET` serialization.
- `model.rs` — `Run`, `Segment`, `Attempt`, metadata, comparison names and
  repair helpers (`fix_splits`, attempt history import).
- `timer.rs` — `Timer` state machine. Mirrors LiveSplit's `TimerModel`:
  start, split, skip, undo, reset, pause, game time, loading times, offsets,
  attempts and comparison switching. `Clock`/`TestClock` make it testable.
- `analysis.rs` — derived values used by components: live and previous-segment
  deltas, best segment checks, split colors, possible time save and predicted
  time.
- `comparisons.rs` — sum-of-best/sum-of-worst branch prediction and generated
  comparisons (best segments, best split times, latest run, weighted average,
  median, worst, balanced PB).
- `format.rs` — LiveSplit's general/delta time formatters (accuracy, digits,
  drop decimals, automatic precision, days, null formats).
- `lss.rs` — LiveSplit `.lss` XML reader/writer.
- `fs.rs` — crash-safe writes (temp file, fsync, rename) with an optional
  `.bak` of the previous version. Used for `.lss`, `.lsl` and `settings.toml`.
- `xml.rs` — small XML tree helpers shared by `.lss` and `.lsl`.

The timer stores a `Run`. The UI never mutates timer internals directly; it
calls state-machine methods and reads `current_time`, the run's segments and
the analysis helpers.

## rustsplit-ui

Layout and rendering. The crate is independent of the application and could be
reused by another frontend.

- `layout.rs` — `Layout`, `LayoutSettings`, `LayoutComponent`, fonts, colors,
  gradients and the layout background type.
- `layout_xml.rs` — `.lsl` parser/writer, including legacy .NET font blobs.
- `components/` — typed settings for every built-in component and their egui
  renderers/settings UIs:
  - `basic.rs`: Title, Text, Sum of Best, Blank Space, Total Playtime,
    Counter, Hotkey Indicator, Comparison Time, World Record, separators.
  - `splits.rs`: Splits table, Graph, Subsplits.
  - `timing.rs`: Timer, Detailed Timer, Previous Segment, Delta, Possible Time
    Save, Current Comparison, Current Pace, Run Prediction, Sum of Segments,
    Manual Game Time.
- `render.rs` — computes the layout scale, draws backgrounds and stacks
  components vertically or horizontally. Layouts are scaled from their stored
  logical size, giving LiveSplit-style dynamic resizing.
- `canvas.rs` — gradients, shadows, outlines, text fitting and separators.
- `icon.rs` — lazy icon decoding and texture caching.

## rustsplit-server

A `std::net` TCP server. Each client thread parses newline-delimited commands
and sends `ServerRequest`s over a channel. An optional notifier wakes the app
when a request is queued; the app drains the channel, mutates the timer on the
UI thread, and sends the optional response back. This keeps all timer mutation
on one thread. Concurrent clients and line lengths are capped.

## rustsplit-auto-splitting

Runs WebAssembly auto splitters (the LiveSplit One / `asr` format) against
the timer. The runtime itself is livesplit-core's `livesplit-auto-splitting`
crate (wasmtime), pinned to an upstream git revision because the crates.io
release predates the current host API.

```
auto splitter thread                       UI thread
────────────────────                       ─────────
update() at the tick rate ──commands──►    AutoSplitterHost::poll
  ChannelTimer (asr Timer trait)  mpsc      apply_command(&mut Timer)
      ▲                                     │
      └──── TimerSnapshot ◄── ArcSwap ◄─────┘ AutoSplitterHost::publish
```

- `runner.rs` — the auto splitter thread: reads and compiles the module (one
  wasmtime engine per load), then calls `update` at the tick rate the auto
  splitter requests (120 Hz by default). A watchdog thread interrupts an
  `update` call that runs longer than five seconds. A trap unloads the auto
  splitter and reports the error.
- `adapter.rs` — `ChannelTimer`, the runtime's `Timer` trait implementation.
  Mutations become `AutoSplitterCommand`s on a channel, like the server's
  `ServerRequest`s. Commands that change the run's state call a waker
  (`egui::Context::request_repaint`); game time and variables, which are
  usually set every tick, wait for the next regular repaint. Queries read a
  `TimerSnapshot` published through `ArcSwap`, so the thread never waits on
  the UI. Until the UI has applied its commands (tracked by sequence number),
  the adapter answers from a prediction of their effect, so an auto splitter
  does not start or split twice while the UI catches up. Redundant commands
  are not sent at all.
- `command.rs` — `apply_command`, run on the UI thread. Game time commands
  are ignored without an attempt in progress, like livesplit-core does.
- `snapshot.rs` — `TimerSnapshot` and the prediction, which mirrors
  `Timer`'s state machine (a test checks both agree).
- `host.rs` — `AutoSplitterHost`, owned by the UI: load/unload, status, the
  settings map and widgets, the log, and writing the settings into the run.
  Commands from a previous auto splitter are dropped by generation number.
- `stored.rs` — `<AutoSplitterSettings>` in livesplit-core's format
  (`Version`, `ScriptPath`, `CustomSettings` with typed `Setting`s).

## rustsplit (binary)

The desktop application and integration layer:

- `main.rs` — CLI, startup, window options.
- `app.rs` — eframe app, context menu, shortcuts, global hotkeys, server
  polling, Niri geometry tracking, dialogs and persistence.
- `auto_splitter.rs` — auto splitter menu, settings window and per-run
  selection (`settings.toml` `[auto_splitter]`, then the `.lss` `ScriptPath`).
  It notices when another split file is opened and loads or unloads the
  matching auto splitter. Without the feature it is a stub with a disabled
  menu entry.
- `config.rs` — `settings.toml` model and recent-file handling.
- `control.rs` — local Unix control socket used by compositor key bindings
  (`rustsplit --command ...`).
- `file_dialog.rs` — dependency-free file browser.
- `file_dialogs.rs` — `FileDialogs` front used by the app. With the default
  `native-dialogs` feature it runs an `rfd` system dialog (XDG portal on
  Linux) on a background thread and polls the result over a channel, falling
  back to the built-in browser when system dialogs are off, unavailable or
  fail to open.
- `niri.rs` — Niri IPC client that floats the overlay and restores its
  floating position and size.
- `run_editor.rs` — splits/segment/comparison/attempt editor.
- `layout_editor.rs` — component list, component settings and global theme.
- `hotkeys.rs` — global hotkey registration and action mapping.
- `speedrun_com.rs` — opt-in speedrun.com client (`speedrun-com` feature) for
  the World Record component. A background thread resolves the game and
  category and fetches the leaderboard's top run; the app hands the cached
  `WorldRecord` to the renderer, so `rustsplit-ui` stays free of networking.
- `sounds.rs` — timer sound effects: detects events by comparing timer
  snapshots before and after each action and plays them on an audio worker
  thread (rodio, behind the default `sounds` feature).

## Data flow

1. Timer state changes (keys, server, auto splitter, menu, editor) go through
   `Timer`, always on the UI thread.
2. The renderer reads a snapshot: current phase/time, run segments, current
   comparison and timing method.
3. Components compute display values with `rustsplit_core::analysis` and the
   time formatters, then paint with egui.
4. When a run is reset, `Timer::reset` updates attempt history, best segments,
   PB comparisons and segment history before the UI saves.
5. `.lss` writes only stored comparisons; generated comparisons live in
   memory and are refreshed on change.

## Testing strategy

- Core unit tests cover time parsing/formatting, timer transitions, reset
  semantics, comparison generation and analysis helpers.
- Integration tests cover `.lss` roundtrips, `.lsl` parsing and the server
  protocol.
- `rustsplit-ui/tests/render.rs` renders whole layouts headlessly with
  `egui_kittest` (no GPU): the default layout with fresh, running and finished
  runs, transparent mode, image backgrounds, empty runs, tiny and huge
  windows, horizontal layouts, and every component type with missing data. The tests assert on
  egui's shape output (rendered text, background rects, component rects, no
  NaN geometry or out-of-window clipping) instead of pixel snapshots, so they
  need no wgpu adapter and are stable across font rasterizers. The harness
  also panics if any text needs a glyph the bundled fonts lack. There are no
  snapshot images to update; when a change alters rendered text on purpose,
  adjust the expected strings in the test.
- Renderer unit tests cover layout scaling, clipping, the background image
  blur and texture cache, and extracting LiveSplit's embedded .NET bitmaps.
- Auto splitting: unit tests drive the timer adapter, command application
  and settings storage without WebAssembly. `tests/wasm.rs` assembles tiny
  auto splitters from WAT and runs them on the real runtime thread (start,
  split, settings, variables, traps, the watchdog).
