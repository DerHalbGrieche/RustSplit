//! World record data supplied by the application.
//!
//! The UI crate does no networking. The application fetches leaderboard data
//! (for example from speedrun.com) and hands the result to the renderer, which
//! passes it on to the World Record component.

use rustsplit_core::TimeSpan;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WorldRecord {
    /// The record time for the component's timing method.
    pub time: TimeSpan,
    /// One entry per run tied for first place. Co-op runners are joined with
    /// " & ".
    pub runners: Vec<String>,
    /// Whether the leaderboard displays milliseconds.
    pub show_milliseconds: bool,
}

impl WorldRecord {
    /// The runners as shown by LiveSplit, e.g. "A, B & C".
    pub fn runners_text(&self) -> String {
        self.runners.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runners_text_joins_ties() {
        let record = WorldRecord {
            runners: vec!["Alice".to_string(), "Bob & Carol".to_string()],
            ..WorldRecord::default()
        };
        assert_eq!(record.runners_text(), "Alice, Bob & Carol");
    }
}
