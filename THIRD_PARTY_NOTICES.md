# Third-party notices

RustSplit is MIT-licensed. It is an independent rewrite and is not affiliated
with or endorsed by the LiveSplit project.

## LiveSplit

- **Project:** https://github.com/LiveSplit/LiveSplit
- **License:** MIT
- **Copyright:** Copyright (c) 2013 Christopher Serr and Sergey Papushin
- **Material used:**
  - The stock layout data in `crates/livesplit-ui/assets/default.lsl`,
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
  comparisons and split-file parsing. No livesplit-core code is linked into
  this project.

## DejaVu fonts

- **Project:** https://dejavu-fonts.github.io/
- **License:** Bitstream Vera Fonts license (permissive)
- **Files:** `crates/livesplit-ui/assets/fonts/DejaVuSans.ttf`,
  `DejaVuSans-Bold.ttf`, `DejaVuSansMono.ttf`, `DejaVuSansMono-Bold.ttf`
- **License text:** `crates/livesplit-ui/assets/fonts/LICENSE-DejaVu.txt`

## Rust dependencies

RustSplit uses third-party crates declared in `Cargo.toml` and pinned in
`Cargo.lock` (including egui/eframe, image, quick XML helpers, chrono,
global-hotkey, serde and the Rust standard library ecosystem). Their licenses
are available from their respective crates. If you distribute binaries, keep
the relevant license notices for those dependencies.

## Trademarks

LiveSplit and the LiveSplit logo are marks of their respective owners.
RustSplit does not include the LiveSplit logo and does not claim affiliation.
The bundled default layout is functional compatibility data, not branding.
