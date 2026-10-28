//! Core timing, run model, comparison generation and split file support for
//! RustSplit.
//!
//! This crate intentionally has no UI dependencies.  It mirrors the semantics
//! of LiveSplit's C# timer model while using nanosecond precision.

pub mod analysis;
pub mod comparisons;
pub mod format;
pub mod lss;
pub mod model;
pub mod time;
pub mod timer;
pub mod xml;

pub use analysis::*;
pub use format::{
    format_delta, format_time,
    DeltaSplitTimeFormatter, DeltaTimeFormatter, DigitsFormat, GeneralTimeFormatter, NullFormat,
    PossibleTimeSaveFormatter, TimeAccuracy,
};
pub use lss::{load_run, load_run_from_str, save_run, save_run_to_string, LssError};
pub use model::{
    Attempt, ComparisonSettings, IndexedTime, Run, RunMetadata, Segment, TimerPhase, TimingMethod,
    AVERAGE_SEGMENTS, BEST_SEGMENTS, BEST_SPLIT_TIMES, CURRENT_COMPARISON, HCP, LATEST_RUN,
    MEDIAN_SEGMENTS, NONE, PERCENTILE, PERSONAL_BEST, WORST_SEGMENTS,
};
pub use time::{Time, TimeParseError, TimeSpan};
pub use timer::{Clock, SystemClock, TestClock, Timer, TimerEvent};
pub use xml::{XmlElement, XmlError};
