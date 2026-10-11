//! Human-readable durations such as `2h`, `30m` or `1d2h30m`.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

const UNITS: [(char, u64); 4] = [('d', 86_400), ('h', 3_600), ('m', 60), ('s', 1)];

/// A duration written as one or more `<number><unit>` groups, units `d`, `h`, `m`, `s`.
///
/// On the wire it is always a string (`"2h"`), so clients don't need to agree on a unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HumanDuration(Duration);

impl HumanDuration {
    pub const fn from_secs(secs: u64) -> Self {
        Self(Duration::from_secs(secs))
    }

    pub const fn as_duration(self) -> Duration {
        self.0
    }

    pub const fn as_secs(self) -> u64 {
        self.0.as_secs()
    }
}

impl From<HumanDuration> for Duration {
    fn from(d: HumanDuration) -> Self {
        d.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DurationParseError {
    #[error("duration is empty")]
    Empty,
    #[error("expected a number before `{0}`")]
    MissingNumber(char),
    #[error("`{0}` is not a unit; use d, h, m or s")]
    BadUnit(char),
    #[error("`{0}` has no unit; write e.g. `{0}m`")]
    MissingUnit(String),
    #[error("duration is too large")]
    Overflow,
}

impl FromStr for HumanDuration {
    type Err = DurationParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err(DurationParseError::Empty);
        }
        let mut total: u64 = 0;
        let mut number = String::new();
        for c in s.chars() {
            if c.is_ascii_digit() {
                number.push(c);
                continue;
            }
            let unit = c.to_ascii_lowercase();
            let Some(&(_, secs)) = UNITS.iter().find(|(u, _)| *u == unit) else {
                return Err(DurationParseError::BadUnit(c));
            };
            if number.is_empty() {
                return Err(DurationParseError::MissingNumber(c));
            }
            let n: u64 = number.parse().map_err(|_| DurationParseError::Overflow)?;
            total = n
                .checked_mul(secs)
                .and_then(|v| total.checked_add(v))
                .ok_or(DurationParseError::Overflow)?;
            number.clear();
        }
        if !number.is_empty() {
            return Err(DurationParseError::MissingUnit(number));
        }
        Ok(Self::from_secs(total))
    }
}

impl fmt::Display for HumanDuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut rest = self.0.as_secs();
        if rest == 0 {
            return f.write_str("0s");
        }
        for (unit, secs) in UNITS {
            if rest >= secs {
                write!(f, "{}{unit}", rest / secs)?;
                rest %= secs;
            }
        }
        Ok(())
    }
}

impl Serialize for HumanDuration {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for HumanDuration {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = Cow::<str>::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for HumanDuration {
    fn schema_name() -> Cow<'static, str> {
        "HumanDuration".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^([0-9]+[dhmsDHMS])+$",
            "description": "One or more <number><unit> groups, units d, h, m, s. Example: \"1d2h30m\"."
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<u64, DurationParseError> {
        s.parse::<HumanDuration>().map(HumanDuration::as_secs)
    }

    #[test]
    fn parses_single_and_combined_units() {
        assert_eq!(parse("2h"), Ok(7_200));
        assert_eq!(parse("30m"), Ok(1_800));
        assert_eq!(parse("90s"), Ok(90));
        assert_eq!(parse("1d2h30m"), Ok(86_400 + 7_200 + 1_800));
        assert_eq!(parse(" 2H "), Ok(7_200));
        assert_eq!(parse("1h1h"), Ok(7_200));
    }

    #[test]
    fn rejects_malformed_input() {
        assert_eq!(parse(""), Err(DurationParseError::Empty));
        assert_eq!(parse("h"), Err(DurationParseError::MissingNumber('h')));
        assert_eq!(
            parse("10"),
            Err(DurationParseError::MissingUnit("10".into()))
        );
        assert_eq!(parse("2x"), Err(DurationParseError::BadUnit('x')));
        assert_eq!(parse("2h 30m"), Err(DurationParseError::BadUnit(' ')));
        assert_eq!(
            parse("99999999999999999999d"),
            Err(DurationParseError::Overflow)
        );
    }

    #[test]
    fn displays_canonical_form() {
        assert_eq!(HumanDuration::from_secs(0).to_string(), "0s");
        assert_eq!(HumanDuration::from_secs(5_400).to_string(), "1h30m");
        assert_eq!(HumanDuration::from_secs(90_061).to_string(), "1d1h1m1s");
    }

    #[test]
    fn serializes_as_string() {
        let d: HumanDuration = serde_json::from_str("\"2h\"").unwrap();
        assert_eq!(serde_json::to_string(&d).unwrap(), "\"2h\"");
        assert!(serde_json::from_str::<HumanDuration>("7200").is_err());
    }
}
