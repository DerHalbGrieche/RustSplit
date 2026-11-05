# Third-party notices

RustSplit is MIT-licensed. It is an independent rewrite and is not affiliated
with or endorsed by the LiveSplit project.

## LiveSplit

- **Project:** https://github.com/LiveSplit/LiveSplit
- **License:** MIT
- **Copyright:** Copyright (c) 2013 Christopher Serr and Sergey Papushin
- **Material used:**
  - The stock layout data in `crates/rustsplit-ui/assets/default.lsl`,
    which originates from LiveSplit's `DefaultLayout.lsl`.
  - Compatibility behavior and format knowledge derived from LiveSplit's
    C# source. The Rust implementation itself is original code.
  - The title and parts of the architecture are inspired by LiveSplit.

The full MIT permission notice is reproduced in the root `LICENSE` file.

## livesplit-core

- **Project:** https://github.com/LiveSplit/livesplit-core
- **License:** MIT OR Apache-2.0
- **Copyright:** LiveSplit contributors
- **Material used:** Algorithmic reference for timer state handling,
  comparisons and split-file parsing. With the default `auto-splitting`
  feature, livesplit-core's `livesplit-auto-splitting` crate (MIT OR
  Apache-2.0) is linked as the WebAssembly auto splitter runtime, together
  with its dependencies (wasmtime, Apache-2.0 WITH LLVM-exception). The
  `.lss` auto splitter settings format follows livesplit-core's
  `StoredAutoSplitterSettings`.

## DejaVu fonts

- **Project:** https://dejavu-fonts.github.io/
- **License:** Bitstream Vera Fonts license (permissive)
- **Files:** `crates/rustsplit-ui/assets/fonts/DejaVuSans.ttf`,
  `DejaVuSans-Bold.ttf`, `DejaVuSansMono.ttf`, `DejaVuSansMono-Bold.ttf`
- **License text:** `crates/rustsplit-ui/assets/fonts/LICENSE-DejaVu.txt`

## Rust dependencies

RustSplit uses third-party crates declared in `Cargo.toml` and pinned in
`Cargo.lock` (including egui/eframe, image, quick XML helpers, chrono,
global-hotkey, serde and the Rust standard library ecosystem). Their licenses
are available from their respective crates. If you distribute binaries, keep
the relevant license notices for those dependencies.

Most dependencies are MIT and/or Apache-2.0. The exceptions in a default
build are:

- **MPL-2.0:** the Symphonia audio decoders used by rodio for timer sounds
  (`symphonia*`, `sounds` feature) and `option-ext`. MPL-2.0 is file-level
  copyleft: it does not affect RustSplit's own code, but modified Symphonia
  source files must stay under MPL-2.0. Source: https://github.com/pdeljanov/Symphonia
- **CDLA-Permissive-2.0:** `webpki-roots`, the Mozilla root certificates used
  for speedrun.com requests (`speedrun-com` feature).
- **Unicode-3.0:** the ICU4X data crates (`icu_*`) used for URL handling.
- **BSL-1.0:** `clipboard-win` and `error-code` (Windows clipboard support).

## Trademarks

LiveSplit and the LiveSplit logo are marks of their respective owners.
RustSplit does not include the LiveSplit logo and does not claim affiliation.
The bundled default layout is functional compatibility data, not branding.
