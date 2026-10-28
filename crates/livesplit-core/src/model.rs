//! Run and segment model.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::time::{Time, TimeSpan};

pub const PERSONAL_BEST: &str = "Personal Best";
pub const BEST_SEGMENTS: &str = "Best Segments";
pub const BEST_SPLIT_TIMES: &str = "Best Split Times";
pub const LATEST_RUN: &str = "Latest Run";
pub const AVERAGE_SEGMENTS: &str = "Average Segments";
pub const MEDIAN_SEGMENTS: &str = "Median Segments";
pub const WORST_SEGMENTS: &str = "Worst Segments";
pub const PERCENTILE: &str = "Balanced PB";
pub const HCP: &str = "Golf HCP";
pub const NONE: &str = "None";
pub const CURRENT_COMPARISON: &str = "Current Comparison";

/// Comparisons that are computed from attempt history instead of being
/// stored in the split file.
pub const GENERATED_COMPARISONS: &[&str] = &[
    BEST_SEGMENTS,
    BEST_SPLIT_TIMES,
    LATEST_RUN,
    AVERAGE_SEGMENTS,
    MEDIAN_SEGMENTS,
    WORST_SEGMENTS,
    PERCENTILE,
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TimingMethod {
    #[default]
    RealTime,
    GameTime,
}

impl TimingMethod {
    pub fn other(self) -> Self {
        match self {
            Self::RealTime => Self::GameTime,
            Self::GameTime => Self::RealTime,
        }
    }

    pub fn short_name(self) -> &'static str {
        match self {
            Self::RealTime => "RTA",
            Self::GameTime => "IGT",
        }
    }
}

impl std::fmt::Display for TimingMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::RealTime => "Real Time",
            Self::GameTime => "Game Time",
        })
    }
}

impl std::str::FromStr for TimingMethod {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "real time" | "realtime" | "rta" | "real" => Ok(Self::RealTime),
            "game time" | "gametime" | "igt" | "game" => Ok(Self::GameTime),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimerPhase {
    #[default]
    NotRunning,
    Running,
    Paused,
    Ended,
}

impl TimerPhase {
    pub fn is_running(self) -> bool {
        matches!(self, Self::Running | Self::Paused)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Segment {
    pub name: String,
    /// Raw image bytes.  The UI decodes these lazily.
    pub icon: Option<Vec<u8>>,
    /// Custom comparison times keyed by comparison name, including the
    /// personal best.
    pub comparisons: HashMap<String, Time>,
    pub best_segment_time: Time,
    pub split_time: Time,
    pub segment_history: BTreeMap<i32, Time>,
    pub custom_variable_values: HashMap<String, String>,
}

impl Segment {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    pub fn comparison(&self, comparison: &str) -> Time {
        self.comparisons.get(comparison).copied().unwrap_or_default()
    }

    pub fn set_comparison(&mut self, comparison: impl Into<String>, time: Time) {
        self.comparisons.insert(comparison.into(), time);
    }

    pub fn personal_best(&self) -> Time {
        self.comparison(PERSONAL_BEST)
    }

    pub fn set_personal_best(&mut self, time: Time) {
        self.set_comparison(PERSONAL_BEST, time);
    }

    pub fn best_segment(&self, method: TimingMethod) -> Option<TimeSpan> {
        self.best_segment_time.get(method)
    }

    pub fn split(&self, method: TimingMethod) -> Option<TimeSpan> {
        self.split_time.get(method)
    }

    pub fn history_time(&self, attempt: i32) -> Option<Time> {
        self.segment_history.get(&attempt).copied()
    }

    pub fn min_history_index(&self) -> i32 {
        self.segment_history
            .keys()
            .next()
            .copied()
            .map(|index| index.min(1))
            .unwrap_or(1)
    }
}

#[derive(Clone, Debug, Default)]
pub struct RunMetadata {
    pub run_id: Option<String>,
    pub platform_name: Option<String>,
    pub uses_emulator: bool,
    pub region_name: Option<String>,
    /// Speedrun.com variables (name -> value name).
    pub variables: BTreeMap<String, String>,
    /// Permanent custom variables (name -> value).
    pub custom_variables: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default)]
pub struct Attempt {
    pub index: i32,
    pub time: Time,
    pub started: Option<DateTime<Utc>>,
    pub ended: Option<DateTime<Utc>>,
    pub pause_time: Option<TimeSpan>,
}

impl Attempt {
    pub fn duration(&self) -> Option<TimeSpan> {
        match (self.started, self.ended) {
            (Some(started), Some(ended)) => {
                let duration = ended.signed_duration_since(started);
                Some(TimeSpan::from_nanos(duration.num_nanoseconds()?))
            }
            _ => self.time.real_time,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexedTime {
    pub time: Time,
    pub index: i32,
}

#[derive(Clone, Debug)]
pub struct ComparisonSettings {
    pub simple_sum_of_best: bool,
    pub use_current_run: bool,
}

impl Default for ComparisonSettings {
    fn default() -> Self {
        Self {
            simple_sum_of_best: false,
            use_current_run: true,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Run {
    pub game_name: String,
    pub category_name: String,
    pub game_icon: Option<Vec<u8>>,
    pub layout_path: Option<String>,
    pub offset: TimeSpan,
    pub attempt_count: i32,
    pub attempts: Vec<Attempt>,
    pub segments: Vec<Segment>,
    /// Stored comparisons, with "Personal Best" always first.
    pub custom_comparisons: Vec<String>,
    pub metadata: RunMetadata,
    /// Raw inner XML of `<AutoSplitterSettings>`.
    pub auto_splitter_settings: Option<String>,
    pub file_path: Option<PathBuf>,
    pub has_changed: bool,
}

impl Run {
    pub fn new(game_name: impl Into<String>, category_name: impl Into<String>) -> Self {
        Self {
            game_name: game_name.into(),
            category_name: category_name.into(),
            custom_comparisons: vec![PERSONAL_BEST.to_string()],
            segments: vec![Segment::new("")],
            ..Self::default()
        }
    }

    pub fn with_segments(
        game_name: impl Into<String>,
        category_name: impl Into<String>,
        segment_names: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        let segments = segment_names
            .into_iter()
            .map(|name| Segment::new(name))
            .collect::<Vec<_>>();
        Self {
            game_name: game_name.into(),
            category_name: category_name.into(),
            custom_comparisons: vec![PERSONAL_BEST.to_string()],
            segments,
            ..Self::default()
        }
    }

    pub fn ensure_personal_best(&mut self) {
        if !self
            .custom_comparisons
            .iter()
            .any(|name| name == PERSONAL_BEST)
        {
            self.custom_comparisons.insert(0, PERSONAL_BEST.to_string());
        }
        for segment in &mut self.segments {
            segment
                .comparisons
                .entry(PERSONAL_BEST.to_string())
                .or_default();
        }
    }

    /// All selectable comparisons: stored ones first, then generated ones.
    pub fn comparisons(&self) -> Vec<String> {
        let mut result = self.custom_comparisons.clone();
        for name in GENERATED_COMPARISONS {
            if !result.iter().any(|existing| existing == name) {
                result.push((*name).to_string());
            }
        }
        if !result.iter().any(|name| name == NONE) {
            result.push(NONE.to_string());
        }
        result
    }

    pub fn contains_comparison(&self, name: &str) -> bool {
        self.comparisons().iter().any(|comparison| comparison == name)
    }

    /// Recompute generated comparisons.
    pub fn refresh_comparisons(&mut self, settings: &ComparisonSettings) {
        self.ensure_personal_best();
        for comparison in GENERATED_COMPARISONS {
            for segment in &mut self.segments {
                segment.comparisons.remove(*comparison);
            }
        }
        crate::comparisons::generate_all(self, settings);
        self.ensure_personal_best();
    }

    pub fn sum_of_best(&self, method: TimingMethod, settings: &ComparisonSettings) -> Option<TimeSpan> {
        crate::comparisons::calculate_sum_of_best(
            self,
            settings.simple_sum_of_best,
            settings.use_current_run,
            method,
        )
    }

    pub fn sum_of_worst(&self, method: TimingMethod) -> Option<TimeSpan> {
        crate::comparisons::calculate_sum_of_worst(self, true, method)
    }

    pub fn min_segment_history_index(&self) -> i32 {
        self.segments
            .iter()
            .map(Segment::min_history_index)
            .min()
            .unwrap_or(1)
    }

    pub fn max_attempt_index(&self) -> i32 {
        self.attempts
            .iter()
            .map(|attempt| attempt.index)
            .max()
            .unwrap_or(0)
    }

    /// Import the current personal best into the attempt history.  This is
    /// used when a run is reset and the new personal best should be remembered.
    pub fn import_segment_history(&mut self) {
        let min_index = self.min_segment_history_index();
        self.import_segment_history_for_method(TimingMethod::RealTime, min_index - 1);
        self.import_segment_history_for_method(TimingMethod::GameTime, min_index - 2);
    }

    fn import_segment_history_for_method(&mut self, method: TimingMethod, index: i32) {
        let mut previous = TimeSpan::ZERO;
        for segment in &mut self.segments {
            let pb = segment.personal_best().get(method);
            let segment_time = pb.map(|time| time - previous);
            segment
                .segment_history
                .insert(index, Time::from_method(method, segment_time));
            if let Some(time) = pb {
                previous = time;
            }
        }
    }

    /// Insert the best segment time of a segment into the history.
    pub fn import_best_segment(&mut self, segment_index: usize) {
        let Some(segment) = self.segments.get(segment_index) else {
            return;
        };
        if segment.best_segment_time.is_empty() {
            return;
        }
        let index = self.min_segment_history_index() - 1;
        let best = segment.best_segment_time;
        self.segments[segment_index]
            .segment_history
            .insert(index, best);
    }

    pub fn clear_history(&mut self) {
        self.attempts.clear();
        for segment in &mut self.segments {
            segment.segment_history.clear();
        }
    }

    pub fn clear_times(&mut self) {
        self.clear_history();
        self.custom_comparisons.clear();
        self.custom_comparisons.push(PERSONAL_BEST.to_string());
        for segment in &mut self.segments {
            segment.comparisons.clear();
            segment.best_segment_time = Time::default();
        }
        self.attempt_count = 0;
        self.metadata.run_id = None;
    }

    /// Repair common inconsistencies in a run.  This is a pragmatic port of
    /// LiveSplit's `RunExtensions.FixSplits`.
    pub fn fix_splits(&mut self) {
        for method in [TimingMethod::RealTime, TimingMethod::GameTime] {
            // Best segment times cannot be negative.
            for segment in &mut self.segments {
                if segment
                    .best_segment_time
                    .get(method)
                    .is_some_and(|time| time < TimeSpan::ZERO)
                {
                    segment.best_segment_time.set(method, None);
                }
            }

            // If the best segment time is gone, the segment history for that
            // timing method is not valid anymore.  LiveSplit clears the
            // affected history entries entirely.
            for segment in &mut self.segments {
                if segment.best_segment_time.get(method).is_none() {
                    segment
                        .segment_history
                        .retain(|_, time| time.get(method).is_none());
                }
            }

            // Comparisons must be non-decreasing and the best segment can not
            // be slower than a PB segment.
            for comparison in self.custom_comparisons.clone() {
                let mut previous = TimeSpan::ZERO;
                for index in 0..self.segments.len() {
                    let Some(time) = self.segments[index]
                        .comparisons
                        .get(&comparison)
                        .and_then(|time| time.get(method))
                    else {
                        continue;
                    };
                    let mut time = time;
                    if time < previous {
                        time = previous;
                        self.segments[index]
                            .comparisons
                            .entry(comparison.clone())
                            .and_modify(|entry| entry.set(method, Some(time)))
                            .or_insert_with(|| Time::from_method(method, Some(time)));
                    }
                    if comparison == PERSONAL_BEST {
                        let segment_time = time - previous;
                        let best = self.segments[index].best_segment_time.get(method);
                        if best.is_none() || best.is_some_and(|best| best > segment_time) {
                            self.segments[index]
                                .best_segment_time
                                .set(method, Some(segment_time));
                        }
                    }
                    previous = time;
                }
            }

            // History entries can not beat the best segment and can not be
            // negative.
            for segment in &mut self.segments {
                for time in segment.segment_history.values_mut() {
                    if let Some(value) = time.get(method) {
                        if value < TimeSpan::ZERO {
                            time.set(method, None);
                        } else if let Some(best) = segment.best_segment_time.get(method) {
                            if value < best {
                                time.set(method, Some(best));
                            }
                        }
                    }
                }
            }
        }

        self.remove_duplicate_history_times();
        self.remove_none_history_values();
    }

    /// Remove duplicate segment history times from route changes.  Only
    /// non-positive history indices are considered, as positive indices are
    /// actual attempts made by the runner.
    fn remove_duplicate_history_times(&mut self) {
        let mut rta_set = std::collections::HashSet::new();
        let mut igt_set = std::collections::HashSet::new();
        let attempt_indices: Vec<i32> = self.attempts.iter().map(|attempt| attempt.index).collect();

        for segment in &mut self.segments {
            rta_set.clear();
            igt_set.clear();

            for index in &attempt_indices {
                if let Some(time) = segment.segment_history.get(index) {
                    if let Some(time) = time.real_time {
                        rta_set.insert(time);
                    }
                    if let Some(time) = time.game_time {
                        igt_set.insert(time);
                    }
                }
            }

            segment.segment_history.retain(|&index, time| {
                if index >= 1 {
                    return true;
                }
                let mut is_none = true;
                let mut is_unique = false;
                if let Some(time) = time.real_time {
                    is_unique |= rta_set.insert(time);
                    is_none = false;
                }
                if let Some(time) = time.game_time {
                    is_unique |= igt_set.insert(time);
                    is_none = false;
                }
                is_none || is_unique
            });
        }
    }

    /// Remove empty history entries that are not followed by a non-empty time.
    ///
    /// Empty entries in the middle of an attempt are meaningful: they mark a
    /// skipped segment.  Only trailing empty entries are removed.
    fn remove_none_history_values(&mut self) {
        if self.segments.is_empty() {
            return;
        }
        let mut cache: Vec<i32> = Vec::new();
        let min_index = self
            .segments
            .iter()
            .map(Segment::min_history_index)
            .min()
            .unwrap_or(1);
        let max_index = self.max_attempt_index().saturating_add(1);

        for run_index in min_index..max_index {
            for index in 0..self.segments.len() {
                match self.segments[index].segment_history.get(&run_index) {
                    Some(element) if element.is_empty() => cache.push(run_index),
                    Some(_) => cache.clear(),
                    None => remove_history_cache(&mut self.segments, index, &mut cache),
                }
            }
            let segment_count = self.segments.len();
            remove_history_cache(&mut self.segments, segment_count, &mut cache);
        }
    }

    pub fn extended_name(&self) -> String {
        match (self.game_name.trim(), self.category_name.trim()) {
            ("", "") => String::new(),
            ("", category) => category.to_string(),
            (game, "") => game.to_string(),
            (game, category) => format!("{game} - {category}"),
        }
    }

    pub fn extended_file_name(&self) -> String {
        self.extended_name()
            .chars()
            .filter(|ch| !matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
            .collect()
    }

    pub fn segment_index_by_name(&self, name: &str) -> Option<usize> {
        self.segments.iter().position(|segment| segment.name == name)
    }
}

/// Remove the cached empty history indices from the segments preceding the
/// position where the history sequence ended.  See `remove_none_history_values`.
fn remove_history_cache(segments: &mut [Segment], index: usize, cache: &mut Vec<i32>) {
    let start = index.saturating_sub(cache.len());
    for (offset, item) in cache.drain(..).enumerate() {
        if let Some(segment) = segments.get_mut(start + offset) {
            segment.segment_history.remove(&item);
        }
    }
}
