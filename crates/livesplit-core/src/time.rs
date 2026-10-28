//! Time types with nanosecond precision and LiveSplit compatible parsing and
//! serialization.

use std::fmt;
use std::ops::{Add, AddAssign, Neg, Sub, SubAssign};

use thiserror::Error;

pub const NANOS_PER_MICROSECOND: i64 = 1_000;
pub const NANOS_PER_MILLISECOND: i64 = 1_000_000;
pub const NANOS_PER_SECOND: i64 = 1_000_000_000;
pub const NANOS_PER_MINUTE: i64 = 60 * NANOS_PER_SECOND;
pub const NANOS_PER_HOUR: i64 = 60 * NANOS_PER_MINUTE;
pub const NANOS_PER_DAY: i64 = 24 * NANOS_PER_HOUR;

/// A signed duration stored as nanoseconds.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimeSpan {
    nanos: i64,
}

impl TimeSpan {
    pub const ZERO: TimeSpan = TimeSpan { nanos: 0 };

    pub const fn from_nanos(nanos: i64) -> Self {
        Self { nanos }
    }

    /// Construct from .NET ticks (100 ns).
    pub const fn from_ticks(ticks: i64) -> Self {
        Self {
            nanos: ticks.saturating_mul(100),
        }
    }

    pub const fn as_nanos(self) -> i64 {
        self.nanos
    }

    /// .NET ticks, truncated towards zero.
    pub const fn ticks(self) -> i64 {
        self.nanos / 100
    }

    pub const fn from_millis(millis: i64) -> Self {
        Self {
            nanos: millis.saturating_mul(NANOS_PER_MILLISECOND),
        }
    }

    pub const fn from_seconds(seconds: i64) -> Self {
        Self {
            nanos: seconds.saturating_mul(NANOS_PER_SECOND),
        }
    }

    pub fn from_secs_f64(seconds: f64) -> Self {
        Self {
            nanos: (seconds * NANOS_PER_SECOND as f64).round() as i64,
        }
    }

    pub fn from_secs_f32(seconds: f32) -> Self {
        Self::from_secs_f64(seconds as f64)
    }

    pub fn as_secs_f64(self) -> f64 {
        self.nanos as f64 / NANOS_PER_SECOND as f64
    }

    pub fn total_seconds(self) -> f64 {
        self.as_secs_f64()
    }

    pub fn total_millis(self) -> f64 {
        self.nanos as f64 / NANOS_PER_MILLISECOND as f64
    }

    pub fn is_zero(self) -> bool {
        self.nanos == 0
    }

    pub fn is_negative(self) -> bool {
        self.nanos < 0
    }

    pub const fn abs(self) -> Self {
        Self {
            nanos: self.nanos.saturating_abs(),
        }
    }

    pub fn max(self, other: Self) -> Self {
        if self >= other {
            self
        } else {
            other
        }
    }

    pub fn min(self, other: Self) -> Self {
        if self <= other {
            self
        } else {
            other
        }
    }

    /// Parse a LiveSplit style time.  Accepts `[days.]hours:minutes:seconds`
    /// with an optional fractional part, as well as the shorter `minutes:seconds`
    /// and plain seconds forms used by the timer's text inputs.
    pub fn parse(input: &str) -> Result<Self, TimeParseError> {
        let mut text = input.trim();
        if text.is_empty() {
            return Err(TimeParseError::Empty);
        }

        let mut negative = false;
        if let Some(stripped) = text.strip_prefix('-').or_else(|| text.strip_prefix('−')) {
            negative = true;
            text = stripped.trim_start();
            if text.is_empty() {
                return Err(TimeParseError::Invalid);
            }
        }

        let sections: Vec<&str> = text.split(':').collect();
        if sections.len() > 4 || sections.iter().any(|section| section.trim().is_empty()) {
            return Err(TimeParseError::Invalid);
        }

        let mut days: i128 = 0;
        let mut hours: i128 = 0;
        let mut minutes: i128 = 0;

        let (seconds, fraction_nanos) = match sections.len() {
            1 => split_fraction(sections[0])?,
            2 => {
                minutes = parse_component(sections[0])?;
                split_fraction(sections[1])?
            }
            3 => {
                // Either hh:mm:ss or days.hh:mm:ss.
                if let Some((day_part, hour_part)) = sections[0].split_once('.') {
                    days = parse_component(day_part)?;
                    hours = parse_component(hour_part)?;
                } else {
                    hours = parse_component(sections[0])?;
                }
                minutes = parse_component(sections[1])?;
                split_fraction(sections[2])?
            }
            4 => {
                days = parse_component(sections[0])?;
                hours = parse_component(sections[1])?;
                minutes = parse_component(sections[2])?;
                split_fraction(sections[3])?
            }
            _ => return Err(TimeParseError::Invalid),
        };

        if minutes >= 60 && sections.len() >= 3 {
            // LiveSplit itself is lenient here, so only reject values that
            // clearly are not a time.
            if minutes > 10_000 {
                return Err(TimeParseError::Invalid);
            }
        }

        let total = days * NANOS_PER_DAY as i128
            + hours * NANOS_PER_HOUR as i128
            + minutes * NANOS_PER_MINUTE as i128
            + seconds * NANOS_PER_SECOND as i128
            + fraction_nanos;
        let total = if negative { -total } else { total };
        if total < i64::MIN as i128 || total > i64::MAX as i128 {
            return Err(TimeParseError::OutOfRange);
        }
        Ok(Self {
            nanos: total as i64,
        })
    }

    /// Parse an optional value, mapping the empty string to `None`.
    pub fn parse_optional(input: &str) -> Result<Option<Self>, TimeParseError> {
        if input.trim().is_empty() {
            Ok(None)
        } else {
            Self::parse(input).map(Some)
        }
    }

    /// The precise .NET `TimeSpan.ToString()` representation, e.g.
    /// `00:01:23.4560000`.  This is what LiveSplit writes into split files.
    pub fn to_precise_string(self) -> String {
        let negative = self.nanos < 0;
        let abs = self.nanos.unsigned_abs();
        let nanos_per_second = NANOS_PER_SECOND as u64;
        let days = abs / NANOS_PER_DAY as u64;
        let hours = (abs / NANOS_PER_HOUR as u64) % 24;
        let minutes = (abs / NANOS_PER_MINUTE as u64) % 60;
        let seconds = (abs / nanos_per_second) % 60;
        let fraction = abs % nanos_per_second;

        let mut result = String::new();
        if negative {
            result.push('-');
        }
        if days > 0 {
            result.push_str(&format!("{days}."));
        }
        result.push_str(&format!("{hours:02}:{minutes:02}:{seconds:02}"));
        if fraction != 0 {
            // TimeSpan's "c" format uses seven fractional digits (100 ns).
            let fraction = fraction / 100;
            result.push_str(&format!(".{fraction:07}"));
        }
        result
    }

    /// A compact representation used in labels, e.g. `1:23.4`.
    pub fn to_short_string(self, accuracy: crate::format::TimeAccuracy) -> String {
        let mut formatter = crate::format::GeneralTimeFormatter::default();
        formatter.accuracy = accuracy;
        formatter.drop_decimals = true;
        formatter.format(Some(self))
    }
}

fn split_fraction(section: &str) -> Result<(i128, i128), TimeParseError> {
    let section = section.trim();
    let (whole, fraction) = match section.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (section, None),
    };
    let whole = parse_component(whole)?;
    let fraction_nanos = match fraction {
        Some(fraction) => parse_fraction(fraction)?,
        None => 0,
    };
    Ok((whole, fraction_nanos))
}

fn parse_component(component: &str) -> Result<i128, TimeParseError> {
    let component = component.trim();
    if component.is_empty() || !component.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(TimeParseError::Invalid);
    }
    component
        .parse::<i128>()
        .map_err(|_| TimeParseError::OutOfRange)
}

fn parse_fraction(fraction: &str) -> Result<i128, TimeParseError> {
    if fraction.is_empty() || !fraction.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(TimeParseError::Invalid);
    }
    let mut digits = fraction.to_string();
    if digits.len() > 9 {
        digits.truncate(9);
    }
    while digits.len() < 9 {
        digits.push('0');
    }
    digits
        .parse::<i128>()
        .map_err(|_| TimeParseError::OutOfRange)
}

impl fmt::Debug for TimeSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("TimeSpan")
            .field(&self.to_precise_string())
            .finish()
    }
}

impl fmt::Display for TimeSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_precise_string())
    }
}

impl Add for TimeSpan {
    type Output = TimeSpan;

    fn add(self, rhs: Self) -> Self::Output {
        TimeSpan {
            nanos: self.nanos.saturating_add(rhs.nanos),
        }
    }
}

impl AddAssign for TimeSpan {
    fn add_assign(&mut self, rhs: Self) {
        self.nanos = self.nanos.saturating_add(rhs.nanos);
    }
}

impl Sub for TimeSpan {
    type Output = TimeSpan;

    fn sub(self, rhs: Self) -> Self::Output {
        TimeSpan {
            nanos: self.nanos.saturating_sub(rhs.nanos),
        }
    }
}

impl SubAssign for TimeSpan {
    fn sub_assign(&mut self, rhs: Self) {
        self.nanos = self.nanos.saturating_sub(rhs.nanos);
    }
}

impl Neg for TimeSpan {
    type Output = TimeSpan;

    fn neg(self) -> Self::Output {
        TimeSpan {
            nanos: self.nanos.saturating_neg(),
        }
    }
}

impl From<std::time::Duration> for TimeSpan {
    fn from(duration: std::time::Duration) -> Self {
        let nanos = duration.as_nanos().min(i64::MAX as u128) as i64;
        TimeSpan { nanos }
    }
}

impl TimeSpan {
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.nanos.checked_add(rhs.nanos).map(|nanos| TimeSpan { nanos })
    }

    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.nanos.checked_sub(rhs.nanos).map(|nanos| TimeSpan { nanos })
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TimeParseError {
    #[error("time is empty")]
    Empty,
    #[error("invalid time")]
    Invalid,
    #[error("time is out of range")]
    OutOfRange,
}

/// A time that can carry a real time and/or an in-game time.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Time {
    pub real_time: Option<TimeSpan>,
    pub game_time: Option<TimeSpan>,
}

impl Time {
    pub const ZERO: Time = Time {
        real_time: Some(TimeSpan::ZERO),
        game_time: Some(TimeSpan::ZERO),
    };

    pub const fn new(real_time: Option<TimeSpan>, game_time: Option<TimeSpan>) -> Self {
        Self {
            real_time,
            game_time,
        }
    }

    pub const fn from_method(method: crate::model::TimingMethod, time: Option<TimeSpan>) -> Self {
        match method {
            crate::model::TimingMethod::RealTime => Self::new(time, None),
            crate::model::TimingMethod::GameTime => Self::new(None, time),
        }
    }

    pub fn is_empty(self) -> bool {
        self.real_time.is_none() && self.game_time.is_none()
    }

    pub fn get(self, method: crate::model::TimingMethod) -> Option<TimeSpan> {
        match method {
            crate::model::TimingMethod::RealTime => self.real_time,
            crate::model::TimingMethod::GameTime => self.game_time,
        }
    }

    pub fn set(&mut self, method: crate::model::TimingMethod, value: Option<TimeSpan>) {
        match method {
            crate::model::TimingMethod::RealTime => self.real_time = value,
            crate::model::TimingMethod::GameTime => self.game_time = value,
        }
    }

    pub fn only(method: crate::model::TimingMethod, value: Option<TimeSpan>) -> Self {
        Self::from_method(method, value)
    }

    pub fn add(self, other: Self) -> Self {
        Self {
            real_time: match (self.real_time, other.real_time) {
                (Some(a), Some(b)) => a.checked_add(b),
                _ => None,
            },
            game_time: match (self.game_time, other.game_time) {
                (Some(a), Some(b)) => a.checked_add(b),
                _ => None,
            },
        }
    }

    pub fn sub(self, other: Self) -> Self {
        Self {
            real_time: match (self.real_time, other.real_time) {
                (Some(a), Some(b)) => a.checked_sub(b),
                _ => None,
            },
            game_time: match (self.game_time, other.game_time) {
                (Some(a), Some(b)) => a.checked_sub(b),
                _ => None,
            },
        }
    }

    /// Merge only the fields that are set in `other`.
    pub fn overlay(self, other: Self) -> Self {
        Self {
            real_time: other.real_time.or(self.real_time),
            game_time: other.game_time.or(self.game_time),
        }
    }
}

impl Add for Time {
    type Output = Time;

    fn add(self, rhs: Self) -> Self::Output {
        self.add(rhs)
    }
}

impl Sub for Time {
    type Output = Time;

    fn sub(self, rhs: Self) -> Self::Output {
        self.sub(rhs)
    }
}

impl fmt::Debug for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Time")
            .field("real_time", &self.real_time)
            .field("game_time", &self.game_time)
            .finish()
    }
}

impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let real = self
            .real_time
            .map(|time| time.to_precise_string())
            .unwrap_or_default();
        let game = self
            .game_time
            .map(|time| time.to_precise_string())
            .unwrap_or_default();
        write!(f, "{real} | {game}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_livesplit_times() {
        assert_eq!(
            TimeSpan::parse("00:01:23.4560000").unwrap(),
            TimeSpan::from_nanos(83_456_000_000)
        );
        assert_eq!(
            TimeSpan::parse("1:23.45").unwrap(),
            TimeSpan::from_nanos(83_450_000_000)
        );
        assert_eq!(
            TimeSpan::parse("-0:00:01.5").unwrap(),
            TimeSpan::from_nanos(-1_500_000_000)
        );
        assert_eq!(
            TimeSpan::parse("1.02:03:04.5").unwrap(),
            TimeSpan::from_nanos(NANOS_PER_DAY + 2 * NANOS_PER_HOUR + 3 * NANOS_PER_MINUTE + 4_500_000_000)
        );
        assert_eq!(TimeSpan::parse("12.5").unwrap(), TimeSpan::from_nanos(12_500_000_000));
    }

    #[test]
    fn precise_roundtrip() {
        for text in ["00:00:00", "01:02:03.4500000", "-00:00:01"] {
            let parsed = TimeSpan::parse(text).unwrap();
            assert_eq!(parsed.to_precise_string(), text);
        }
    }

    #[test]
    fn parses_empty_optional() {
        assert_eq!(TimeSpan::parse_optional("  ").unwrap(), None);
        assert_eq!(
            TimeSpan::parse_optional("0:01.5").unwrap(),
            Some(TimeSpan::from_nanos(1_500_000_000))
        );
    }
}
