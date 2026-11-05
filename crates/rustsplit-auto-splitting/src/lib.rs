//! WebAssembly auto splitter support for RustSplit.
//!
//! Auto splitters are WebAssembly modules written against the `asr` crate,
//! the format used by LiveSplit One and LiveSplit's Auto Splitting Runtime
//! component. They run on livesplit-core's `livesplit-auto-splitting`
//! runtime (wasmtime).
//!
//! Threading model:
//!
//! - The auto splitter runs on its own thread ([`runner`]) at the tick rate it
//!   requests (120 Hz by default), with a watchdog that interrupts a hung
//!   `update` call after five seconds.
//! - Timer mutations are sent as [`AutoSplitterCommand`]s over a channel,
//!   like the LiveSplit Server does with `ServerRequest`s. The UI thread
//!   drains it in [`AutoSplitterHost::poll`] and applies them with
//!   [`apply_command`], so the [`rustsplit_core::Timer`] is only ever mutated
//!   on the UI thread. A waker callback (e.g. `request_repaint`) gets the UI
//!   to poll promptly.
//! - The auto splitter reads timer state from a [`TimerSnapshot`] that the UI
//!   publishes through an `ArcSwap`, so neither side ever waits on a lock held
//!   by the other. Commands it sent that the UI has not applied yet are
//!   accounted for by predicting their effect.

mod adapter;
mod command;
mod host;
mod runner;
mod snapshot;
mod stored;

pub use adapter::{LogLine, LogSeverity};
pub use command::{apply_command, AutoSplitterCommand, CommandEffect};
pub use host::{AutoSplitterHost, HostEvent, HostStatus, PollOutcome};
pub use livesplit_auto_splitting::{settings, wasi_path};
pub use snapshot::TimerSnapshot;
pub use stored::StoredSettings;

#[cfg(test)]
mod tests;
