# Changelog

## 1.1.0

### New

- **WebAssembly auto splitters.** RustSplit runs the `.wasm` auto splitters
  made for LiveSplit One and LiveSplit's Auto Splitting Runtime, using
  livesplit-core's runtime. Load one from the context menu under
  **Auto Splitter**; its settings are editable in RustSplit and stored in the
  `.lss` file in livesplit-core's format.
- **Sound effects** for splits, splits ahead/behind, best segments, personal
  bests, undo, skip, reset, pause, resume and start, with a general volume, a
  Test button per sound, and fallbacks between related events.
- **speedrun.com world records** for the World Record component. Opt-in under
  Settings; lookups run in the background and are cached.
- **System file dialogs** through the XDG desktop portal on Linux and the
  native dialogs on Windows and macOS. The built-in browser remains as a
  fallback and can be forced in Settings.
- **LiveSplit background images.** `BackgroundType=Image` layouts now show their
  image instead of the background color, images embedded in LiveSplit layouts
  are decoded, and image opacity and blur match LiveSplit. The layout editor
  can pick the background type and image.
- The Settings window has **OK, Apply and Cancel**.

### Improved

- **Precise input timing.** Global hotkeys, LiveSplit Server commands,
  compositor bindings and auto splitters are timestamped when they arrive, so
  a split keeps its exact time even if the UI handles it later, and they keep
  working while the window is minimized or hidden.
- **Crash-safe saving.** Split files, layouts and settings are written to a
  temporary file, synced and renamed into place, and the previous split file
  and layout are kept as `.bak`. Save errors are reported instead of ignored.
- Global hotkeys wake the app immediately; from an idle timer a start could
  previously be handled up to 250 ms late.
- The LiveSplit Server no longer polls every 33 ms while idle, limits clients
  to 16 and lines to 64 KiB, and no longer autosaves mid-run on every remote
  split.

### Fixed

- A large background image without blur crashed the app at startup.
- Server and control-socket client threads busy-looped on macOS and Windows.
- LiveSplit layouts with an image background opened see-through.
- The layout settings, including embedded images, were cloned every frame.
- Saving through a dangling symlink failed; read-only split files are no longer
  overwritten.

### Changed

- Requires Rust 1.97 or newer.
- The internal crates are renamed from `livesplit-*` to `rustsplit-*`.
- The default `auto-splitting`, `sounds`, `speedrun-com` and `native-dialogs`
  cargo features can be disabled to build a smaller binary. Sound needs the
  ALSA development headers on Linux.
- CI now checks formatting and clippy and runs the tests on Linux, Windows and
  macOS.
