# Architecture

RustSplit is split into four crates with a one-way dependency graph:

```
rustsplit (binary)
 ├── livesplit-ui ──┐
 ├── livesplit-server ─┤
 └── livesplit-core ◄─┘
```

## livesplit-core

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
- `xml.rs` — small XML tree helpers shared by `.lss` and `.lsl`.

The timer stores a `Run`. The UI never mutates timer internals directly; it
calls state-machine methods and reads `current_time`, the run's segments and
the analysis helpers.

## livesplit-ui

Layout and rendering. The crate is independent of the application and could be
reused by another frontend.

- `layout.rs` — `Layout`, `LayoutSettings`, `LayoutComponent`, fonts, colors
  and gradients.
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

## livesplit-server

A `std::net` TCP server. Each client thread parses newline-delimited commands
and sends `ServerRequest`s over a channel. The app polls the channel, mutates
the timer on the UI thread, and sends the optional response back. This keeps
all timer mutation on one thread.

## rustsplit (binary)

The desktop application and integration layer:

- `main.rs` — CLI, startup, window options.
- `app.rs` — eframe app, context menu, shortcuts, global hotkeys, server
  polling, dialogs and persistence.
- `config.rs` — `settings.toml` model and recent-file handling.
- `file_dialog.rs` — dependency-free file browser.
- `run_editor.rs` — splits/segment/comparison/attempt editor.
- `layout_editor.rs` — component list, component settings and global theme.
- `hotkeys.rs` — global hotkey registration and action mapping.

## Data flow

1. Timer state changes (keys, server, menu, editor) go through `Timer`.
2. The renderer reads a snapshot: current phase/time, run segments, current
   comparison and timing method.
3. Components compute display values with `livesplit_core::analysis` and the
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
- UI components are render-tested via `cargo check`; the renderer is designed
  to be panic-free with missing times, empty runs and out-of-range indices.
