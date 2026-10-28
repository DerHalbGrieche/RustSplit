//! LiveSplit compatible time formatting.

use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::time::{TimeSpan, NANOS_PER_DAY, NANOS_PER_HOUR, NANOS_PER_MINUTE, NANOS_PER_SECOND};

pub const DASH: &str = "-";
pub const MINUS: &str = "−";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeAccuracy {
    #[default]
    Seconds,
    Tenths,
    Hundredths,
    Milliseconds,
}

impl TimeAccuracy {
    pub fn decimals(self) -> usize {
        match self {
            Self::Seconds => 0,
            Self::Tenths => 1,
            Self::Hundredths => 2,
            Self::Milliseconds => 3,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Seconds => "Seconds",
            Self::Tenths => "Tenths",
            Self::Hundredths => "Hundredths",
            Self::Milliseconds => "Milliseconds",
        }
    }
}

impl FromStr for TimeAccuracy {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "Seconds" | "seconds" => Ok(Self::Seconds),
            "Tenths" | "tenths" => Ok(Self::Tenths),
            "Hundredths" | "hundredths" => Ok(Self::Hundredths),
            "Milliseconds" | "milliseconds" => Ok(Self::Milliseconds),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DigitsFormat {
    #[default]
    SingleDigitSeconds,
    DoubleDigitSeconds,
    SingleDigitMinutes,
    DoubleDigitMinutes,
    SingleDigitHours,
    DoubleDigitHours,
}

impl DigitsFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::SingleDigitSeconds => "1",
            Self::DoubleDigitSeconds => "01",
            Self::SingleDigitMinutes => "0:01",
            Self::DoubleDigitMinutes => "00:01",
            Self::SingleDigitHours => "0:00:01",
            Self::DoubleDigitHours => "00:00:01",
        }
    }
}

impl FromStr for DigitsFormat {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "SingleDigitSeconds" | "1" => Ok(Self::SingleDigitSeconds),
            "DoubleDigitSeconds" | "01" => Ok(Self::DoubleDigitSeconds),
            "SingleDigitMinutes" | "0:01" => Ok(Self::SingleDigitMinutes),
            "DoubleDigitMinutes" | "00:01" => Ok(Self::DoubleDigitMinutes),
            "SingleDigitHours" | "0:00:01" => Ok(Self::SingleDigitHours),
            "DoubleDigitHours" | "00:00:01" => Ok(Self::DoubleDigitHours),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NullFormat {
    #[default]
    Dash,
    ZeroWithAccuracy,
    ZeroDotZeroZero,
    ZeroValue,
    Dashes,
}

impl FromStr for NullFormat {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "Dash" => Ok(Self::Dash),
            "ZeroWithAccuracy" => Ok(Self::ZeroWithAccuracy),
            "ZeroDotZeroZero" => Ok(Self::ZeroDotZeroZero),
            "ZeroValue" => Ok(Self::ZeroValue),
            "Dashes" => Ok(Self::Dashes),
            _ => Err(()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct GeneralTimeFormatter {
    pub accuracy: TimeAccuracy,
    pub digits_format: DigitsFormat,
    pub null_format: NullFormat,
    pub show_days: bool,
    pub show_plus: bool,
    pub drop_decimals: bool,
    pub automatic_precision: bool,
}

impl Default for GeneralTimeFormatter {
    fn default() -> Self {
        Self {
            accuracy: TimeAccuracy::Seconds,
            digits_format: DigitsFormat::SingleDigitSeconds,
            null_format: NullFormat::Dash,
            show_days: false,
            show_plus: false,
            drop_decimals: false,
            automatic_precision: false,
        }
    }
}

impl GeneralTimeFormatter {
    pub fn format(&self, time: Option<TimeSpan>) -> String {
        let mut is_null = time.is_none();
        let mut time = match (time, self.null_format) {
            (Some(time), _) => time,
            (None, NullFormat::Dash) => return DASH.to_string(),
            (None, NullFormat::ZeroWithAccuracy) => return self.zero_with_accuracy(),
            (None, NullFormat::ZeroDotZeroZero) => return "0.00".to_string(),
            (None, NullFormat::ZeroValue) => TimeSpan::ZERO,
            (None, NullFormat::Dashes) => {
                is_null = true;
                TimeSpan::ZERO
            }
        };

        let mut sign = "";
        if time < TimeSpan::ZERO {
            sign = MINUS;
            time = -time;
        } else if time > TimeSpan::ZERO && self.show_plus {
            sign = "+";
        }

        let decimal_places = self.decimal_places(time);
        let total_seconds = time.as_secs_f64();
        let total_minutes = total_seconds / 60.0;
        let total_hours = total_seconds / 3600.0;
        let total_days = total_seconds / 86400.0;

        let mut formatted = if total_days >= 1.0 && !self.show_days {
            let hours = time.as_nanos() / NANOS_PER_HOUR;
            format!(
                "{hours}:{:02}:{:02}{}",
                (time.as_nanos() / NANOS_PER_MINUTE) % 60,
                (time.as_nanos() / NANOS_PER_SECOND) % 60,
                self.fraction_string(time, decimal_places)
            )
        } else if self.show_days && total_days >= 1.0 {
            let days = time.as_nanos() / NANOS_PER_DAY;
            let hours = (time.as_nanos() / NANOS_PER_HOUR) % 24;
            match self.digits_format {
                DigitsFormat::DoubleDigitHours => format!(
                    "{days}d {hours:02}:{:02}:{:02}{}",
                    (time.as_nanos() / NANOS_PER_MINUTE) % 60,
                    (time.as_nanos() / NANOS_PER_SECOND) % 60,
                    self.fraction_string(time, decimal_places)
                ),
                _ => format!(
                    "{days}d {hours}:{:02}:{:02}{}",
                    (time.as_nanos() / NANOS_PER_MINUTE) % 60,
                    (time.as_nanos() / NANOS_PER_SECOND) % 60,
                    self.fraction_string(time, decimal_places)
                ),
            }
        } else {
            let (show_hours, show_minutes, pad_hours, pad_minutes, pad_seconds) =
                match self.digits_format {
                    DigitsFormat::DoubleDigitHours => {
                        (true, true, true, true, true)
                    }
                    DigitsFormat::SingleDigitHours => (true, true, false, true, true),
                    DigitsFormat::DoubleDigitMinutes => (false, true, false, true, true),
                    DigitsFormat::SingleDigitMinutes => (false, true, false, false, true),
                    DigitsFormat::DoubleDigitSeconds => (false, false, false, false, true),
                    DigitsFormat::SingleDigitSeconds => {
                        // A larger actual magnitude forces a larger unit.
                        if total_hours >= 1.0 {
                            (true, true, false, true, true)
                        } else if total_minutes >= 1.0 {
                            (false, true, false, false, true)
                        } else {
                            (false, false, false, false, false)
                        }
                    }
                };

            let mut parts = Vec::new();
            if show_hours {
                let hours = time.as_nanos() / NANOS_PER_HOUR;
                parts.push(if pad_hours {
                    format!("{hours:02}")
                } else {
                    hours.to_string()
                });
            }
            if show_minutes {
                let minutes = (time.as_nanos() / NANOS_PER_MINUTE) % 60;
                parts.push(if pad_minutes {
                    format!("{minutes:02}")
                } else {
                    minutes.to_string()
                });
            }
            let seconds = (time.as_nanos() / NANOS_PER_SECOND) % 60;
            let mut seconds = if pad_seconds {
                format!("{seconds:02}")
            } else {
                seconds.to_string()
            };
            seconds.push_str(&self.fraction_string(time, decimal_places));
            parts.push(seconds);
            parts.join(":")
        };
        formatted.insert_str(0, sign);

        if is_null && self.null_format == NullFormat::Dashes {
            formatted = formatted.replace('0', "-");
        }
        formatted
    }

    fn decimal_places(&self, time: TimeSpan) -> usize {
        if self.automatic_precision {
            let total_seconds = time.as_secs_f64();
            if self.accuracy == TimeAccuracy::Seconds || total_seconds.fract() == 0.0 {
                0
            } else if self.accuracy == TimeAccuracy::Tenths || (10.0 * total_seconds).fract() == 0.0
            {
                1
            } else if self.accuracy == TimeAccuracy::Hundredths
                || (100.0 * total_seconds).fract() == 0.0
            {
                2
            } else {
                3
            }
        } else if self.drop_decimals && time.as_nanos().abs() / NANOS_PER_MINUTE >= 1 {
            0
        } else {
            self.accuracy.decimals()
        }
    }

    fn fraction_string(&self, time: TimeSpan, decimal_places: usize) -> String {
        if decimal_places == 0 {
            return String::new();
        }
        let nanos = time.as_nanos().abs() % NANOS_PER_SECOND;
        let digits = format!("{nanos:09}");
        format!(".{}", &digits[..decimal_places])
    }

    fn zero_with_accuracy(&self) -> String {
        if self.automatic_precision || self.accuracy == TimeAccuracy::Seconds {
            "0".to_string()
        } else {
            format!("0.{}", "0".repeat(self.accuracy.decimals()))
        }
    }
}

#[derive(Clone, Debug)]
pub struct DeltaTimeFormatter {
    inner: GeneralTimeFormatter,
}

impl Default for DeltaTimeFormatter {
    fn default() -> Self {
        Self {
            inner: GeneralTimeFormatter {
                accuracy: TimeAccuracy::Tenths,
                drop_decimals: true,
                show_plus: true,
                ..Default::default()
            },
        }
    }
}

impl DeltaTimeFormatter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn format(&self, time: Option<TimeSpan>) -> String {
        self.inner.format(time)
    }

    pub fn set_accuracy(&mut self, accuracy: TimeAccuracy) {
        self.inner.accuracy = accuracy;
    }

    pub fn set_drop_decimals(&mut self, drop_decimals: bool) {
        self.inner.drop_decimals = drop_decimals;
    }
}

#[derive(Clone, Debug)]
pub struct DeltaSplitTimeFormatter {
    inner: GeneralTimeFormatter,
}

impl DeltaSplitTimeFormatter {
    pub fn new(accuracy: TimeAccuracy, drop_decimals: bool) -> Self {
        Self {
            inner: GeneralTimeFormatter {
                accuracy,
                drop_decimals,
                show_plus: true,
                ..Default::default()
            },
        }
    }

    pub fn format(&self, time: Option<TimeSpan>) -> String {
        self.inner.format(time)
    }
}

#[derive(Clone, Debug)]
pub struct PossibleTimeSaveFormatter {
    inner: GeneralTimeFormatter,
}

impl Default for PossibleTimeSaveFormatter {
    fn default() -> Self {
        Self {
            inner: GeneralTimeFormatter {
                accuracy: TimeAccuracy::Seconds,
                drop_decimals: false,
                ..Default::default()
            },
        }
    }
}

impl PossibleTimeSaveFormatter {
    pub fn format(&self, time: Option<TimeSpan>) -> String {
        self.inner.format(time)
    }
}

/// Format a duration using the given accuracy and LiveSplit's default digits.
pub fn format_time(time: Option<TimeSpan>, accuracy: TimeAccuracy) -> String {
    GeneralTimeFormatter {
        accuracy,
        ..Default::default()
    }
    .format(time)
}

/// Format a delta (with plus sign) using tenths of a second.
pub fn format_delta(time: Option<TimeSpan>) -> String {
    DeltaTimeFormatter::default().format(time)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(seconds: i64, nanos: i64) -> Option<TimeSpan> {
        Some(TimeSpan::from_nanos(seconds * NANOS_PER_SECOND + nanos))
    }

    #[test]
    fn formats_like_livesplit() {
        let mut f = GeneralTimeFormatter::default();
        assert_eq!(f.format(ts(0, 0)), "0");
        assert_eq!(f.format(ts(1, 500_000_000)), "1");
        f.accuracy = TimeAccuracy::Hundredths;
        assert_eq!(f.format(ts(83, 456_000_000)), "1:23.45");
        f.digits_format = DigitsFormat::DoubleDigitMinutes;
        assert_eq!(f.format(ts(83, 456_000_000)), "01:23.45");
        f.drop_decimals = true;
        assert_eq!(f.format(ts(83, 456_000_000)), "01:23");
        assert_eq!(f.format(None), "-");
    }

    #[test]
    fn formats_days() {
        let mut f = GeneralTimeFormatter {
            accuracy: TimeAccuracy::Seconds,
            digits_format: DigitsFormat::DoubleDigitHours,
            show_days: true,
            ..Default::default()
        };
        assert_eq!(
            f.format(ts(86400 + 3661, 0)),
            "1d 01:01:01"
        );
        f.show_days = false;
        assert_eq!(f.format(ts(86400 + 3661, 0)), "25:01:01");
    }

    #[test]
    fn delta_format() {
        let f = DeltaTimeFormatter::default();
        assert_eq!(f.format(ts(1, 500_000_000)), "+1.5");
        assert_eq!(f.format(Some(TimeSpan::from_secs_f64(-2.25))), "−2.2");
        assert_eq!(f.format(None), "-");
    }

    #[test]
    fn null_formats() {
        let f = GeneralTimeFormatter {
            null_format: NullFormat::Dashes,
            accuracy: TimeAccuracy::Hundredths,
            digits_format: DigitsFormat::DoubleDigitMinutes,
            ..Default::default()
        };
        assert_eq!(f.format(None), "--:--.--");
    }
}
