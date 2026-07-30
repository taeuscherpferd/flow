use std::collections::BTreeSet;
use std::str::FromStr;

use chrono::{DateTime, Datelike, Duration, Timelike, Utc};
use chrono_tz::Tz;
use thiserror::Error;

const RANGES: [(u32, u32); 5] = [(0, 59), (0, 23), (1, 31), (1, 12), (0, 7)];
const SEARCH_LIMIT_DAYS: i64 = 366 * 8;

#[derive(Clone, Debug, Eq, PartialEq)]
struct ParsedField {
    values: BTreeSet<u32>,
    wildcard: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CronExpression {
    source: String,
    fields: Vec<ParsedField>,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum CronError {
    #[error("Cron expression must contain exactly five fields.")]
    FieldCount,
    #[error("Invalid cron field \"{0}\".")]
    InvalidField(String),
    #[error("Cron value \"{value}\" must be between {minimum} and {maximum}.")]
    ValueOutOfRange {
        value: String,
        minimum: u32,
        maximum: u32,
    },
    #[error("Invalid cron range \"{0}\".")]
    InvalidRange(String),
    #[error("Cron range \"{0}\" is reversed.")]
    ReversedRange(String),
    #[error("Unknown IANA timezone \"{0}\".")]
    UnknownTimezone(String),
    #[error("Cron expression has no occurrence in the next eight years.")]
    NoOccurrence,
}

impl CronExpression {
    /// Parses a normalized five-field cron expression.
    ///
    /// # Errors
    ///
    /// Returns [`CronError`] when the field count, range, value, or step is invalid.
    pub fn parse(source: &str) -> Result<Self, CronError> {
        let parts: Vec<&str> = source.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(CronError::FieldCount);
        }
        let fields = parts
            .iter()
            .enumerate()
            .map(|(index, part)| {
                let (minimum, maximum) = RANGES[index];
                parse_field(part, minimum, maximum, index == 4)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            source: parts.join(" "),
            fields,
        })
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Finds the first matching UTC minute strictly after `after`.
    ///
    /// # Errors
    ///
    /// Returns [`CronError`] for an unknown timezone or when no match is found
    /// within the legacy eight-year search window.
    pub fn next(&self, after: DateTime<Utc>, timezone: &str) -> Result<DateTime<Utc>, CronError> {
        let timezone = parse_timezone(timezone)?;
        let Some(truncated) = after
            .with_second(0)
            .and_then(|value| value.with_nanosecond(0))
        else {
            return Err(CronError::NoOccurrence);
        };
        let Some(mut candidate) = truncated.checked_add_signed(Duration::minutes(1)) else {
            return Err(CronError::NoOccurrence);
        };
        let Some(limit) = candidate.checked_add_signed(Duration::days(SEARCH_LIMIT_DAYS)) else {
            return Err(CronError::NoOccurrence);
        };

        while candidate <= limit {
            let local = candidate.with_timezone(&timezone);
            if self.matches(
                local.minute(),
                local.hour(),
                local.day(),
                local.month(),
                local.weekday().num_days_from_sunday(),
            ) {
                return Ok(candidate);
            }
            let Some(next) = candidate.checked_add_signed(Duration::minutes(1)) else {
                break;
            };
            candidate = next;
        }
        Err(CronError::NoOccurrence)
    }

    fn matches(&self, minute: u32, hour: u32, day: u32, month: u32, weekday: u32) -> bool {
        let [
            minute_field,
            hour_field,
            day_field,
            month_field,
            weekday_field,
        ] = self.fields.as_slice()
        else {
            return false;
        };
        let day_matches = day_field.values.contains(&day);
        let weekday_matches = weekday_field.values.contains(&weekday);
        let calendar_day_matches = if !day_field.wildcard && !weekday_field.wildcard {
            day_matches || weekday_matches
        } else {
            day_matches && weekday_matches
        };
        minute_field.values.contains(&minute)
            && hour_field.values.contains(&hour)
            && month_field.values.contains(&month)
            && calendar_day_matches
    }
}

impl FromStr for CronExpression {
    type Err = CronError;

    fn from_str(source: &str) -> Result<Self, Self::Err> {
        Self::parse(source)
    }
}

/// Validates an IANA timezone identifier.
///
/// # Errors
///
/// Returns [`CronError::UnknownTimezone`] when `timezone` is not recognized.
pub fn validate_timezone(timezone: &str) -> Result<(), CronError> {
    parse_timezone(timezone).map(|_| ())
}

fn parse_timezone(timezone: &str) -> Result<Tz, CronError> {
    timezone
        .parse()
        .map_err(|_| CronError::UnknownTimezone(timezone.to_owned()))
}

fn parse_number(text: &str, minimum: u32, maximum: u32) -> Result<u32, CronError> {
    let parsed = if text.is_empty() {
        0.0
    } else {
        text.parse::<f64>()
            .map_err(|_| CronError::ValueOutOfRange {
                value: text.to_owned(),
                minimum,
                maximum,
            })?
    };
    if !parsed.is_finite()
        || parsed.fract() != 0.0
        || parsed < f64::from(minimum)
        || parsed > f64::from(maximum)
    {
        return Err(CronError::ValueOutOfRange {
            value: text.to_owned(),
            minimum,
            maximum,
        });
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let value = parsed as u32;
    Ok(value)
}

fn parse_field(
    text: &str,
    minimum: u32,
    maximum: u32,
    day_of_week: bool,
) -> Result<ParsedField, CronError> {
    let mut values = BTreeSet::new();
    let wildcard = text == "*" || text.starts_with("*/");
    for segment in text.split(',') {
        let slash_parts: Vec<&str> = segment.split('/').collect();
        if slash_parts.len() > 2 {
            return Err(CronError::InvalidField(text.to_owned()));
        }
        let Some(base) = slash_parts.first().copied() else {
            return Err(CronError::InvalidField(text.to_owned()));
        };
        let step = slash_parts.get(1).map_or(Ok(1), |step_text| {
            parse_number(step_text, 1, maximum - minimum + 1)
        })?;
        let (start, end) = if base == "*" {
            (minimum, maximum)
        } else if base.contains('-') {
            let mut range = base.split('-');
            let Some(start_text) = range.next() else {
                return Err(CronError::InvalidRange(base.to_owned()));
            };
            let Some(end_text) = range.next() else {
                return Err(CronError::InvalidRange(base.to_owned()));
            };
            let start = parse_number(start_text, minimum, maximum)?;
            let end = parse_number(end_text, minimum, maximum)?;
            if start > end {
                return Err(CronError::ReversedRange(base.to_owned()));
            }
            (start, end)
        } else {
            let value = parse_number(base, minimum, maximum)?;
            (value, value)
        };
        for value in (start..=end).step_by(step as usize) {
            values.insert(if day_of_week && value == 7 { 0 } else { value });
        }
    }
    Ok(ParsedField { values, wildcard })
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::{CronExpression, validate_timezone};

    fn instant(value: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
        value.parse::<DateTime<Utc>>()
    }

    // Legacy: CronExpression.test.ts — parses five-field expressions in an IANA timezone.
    #[test]
    fn parses_five_field_cron_and_advances_in_iana_timezone()
    -> Result<(), Box<dyn std::error::Error>> {
        let cron = CronExpression::parse("*/15 9-10 * * 1-5")?;
        let next = cron.next(instant("2026-07-27T14:01:00.000Z")?, "America/Denver")?;

        assert_eq!(next, instant("2026-07-27T15:00:00.000Z")?);
        Ok(())
    }

    // Legacy: CronExpression.test.ts — skips nonexistent daylight-saving local times.
    #[test]
    fn skips_nonexistent_local_times_during_dst_transition()
    -> Result<(), Box<dyn std::error::Error>> {
        let cron = CronExpression::parse("30 2 * * *")?;
        let next = cron.next(instant("2026-03-08T08:00:00.000Z")?, "America/Denver")?;

        assert_eq!(next, instant("2026-03-09T08:30:00.000Z")?);
        Ok(())
    }

    // Legacy: CronExpression.test.ts — retains both repeated daylight-saving local times.
    #[test]
    fn retains_both_repeated_local_times_when_dst_ends() -> Result<(), Box<dyn std::error::Error>> {
        let cron = CronExpression::parse("30 1 * * *")?;
        let first = cron.next(instant("2026-11-01T07:29:00.000Z")?, "America/Denver")?;
        let second = cron.next(first, "America/Denver")?;

        assert_eq!(first, instant("2026-11-01T07:30:00.000Z")?);
        assert_eq!(second, instant("2026-11-01T08:30:00.000Z")?);
        Ok(())
    }

    // Legacy: CronExpression.test.ts — rejects malformed expressions and unknown timezones.
    #[test]
    fn rejects_malformed_cron_and_unknown_timezones() {
        assert!(CronExpression::parse("* * * *").is_err());
        assert!(CronExpression::parse("60 * * * *").is_err());
        assert!(validate_timezone("Mars/Olympus_Mons").is_err());
    }

    #[test]
    fn restricted_day_of_month_and_weekday_use_traditional_or_semantics()
    -> Result<(), Box<dyn std::error::Error>> {
        let cron = CronExpression::parse("0 0 1 * 1")?;
        let next = cron.next(instant("2026-07-01T00:00:00Z")?, "UTC")?;

        assert_eq!(next, instant("2026-07-06T00:00:00Z")?);
        Ok(())
    }
}
