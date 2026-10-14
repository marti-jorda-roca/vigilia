//! Safety rules, evaluated only when a power or thermal event arrives — never on a timer.
//!
//! Rules are a list of `(condition → action)` entries. Step 6 adds more entries (power saver
//! below 40%, power saver at serious heat) without changing the evaluation path.

use vigilia_protocol::{PowerSnapshot, ThermalLevel};

use crate::config::RulesConfig;

/// What a fired rule asks the service to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleAction {
    /// End the session, restoring normal sleep behaviour.
    EndSession { reason: String },
}

/// Evaluates every rule against one power snapshot.
pub fn evaluate(power: &PowerSnapshot, config: &RulesConfig) -> Vec<RuleAction> {
    let mut actions = Vec::new();
    if let Some(percent) = power.battery_percent
        && percent < config.battery_floor_percent
    {
        actions.push(RuleAction::EndSession {
            reason: format!(
                "battery at {percent}% is below the {}% floor",
                config.battery_floor_percent
            ),
        });
    }
    if config.end_at_critical_heat && power.thermal == ThermalLevel::Critical {
        actions.push(RuleAction::EndSession {
            reason: "thermal level is critical".into(),
        });
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn power(battery_percent: Option<u8>, thermal: ThermalLevel) -> PowerSnapshot {
        PowerSnapshot {
            battery_percent,
            on_ac: false,
            thermal,
        }
    }

    #[test]
    fn battery_below_the_floor_ends_the_session() {
        let config = RulesConfig::default();
        assert_eq!(config.battery_floor_percent, 10);
        assert_eq!(
            evaluate(&power(Some(9), ThermalLevel::Nominal), &config).len(),
            1
        );
        assert!(evaluate(&power(Some(10), ThermalLevel::Nominal), &config).is_empty());
        assert!(evaluate(&power(Some(80), ThermalLevel::Nominal), &config).is_empty());
        // A desktop without a battery never fires the battery rule.
        assert!(evaluate(&power(None, ThermalLevel::Nominal), &config).is_empty());
    }

    #[test]
    fn a_raised_floor_ends_a_session_above_the_default() {
        let config = RulesConfig {
            battery_floor_percent: 50,
            ..RulesConfig::default()
        };
        assert_eq!(
            evaluate(&power(Some(42), ThermalLevel::Nominal), &config).len(),
            1
        );
    }

    #[test]
    fn critical_heat_ends_the_session_but_serious_does_not() {
        let config = RulesConfig::default();
        assert_eq!(
            evaluate(&power(Some(80), ThermalLevel::Critical), &config).len(),
            1
        );
        assert!(evaluate(&power(Some(80), ThermalLevel::Serious), &config).is_empty());
    }

    #[test]
    fn critical_heat_is_ignored_when_the_flag_is_off() {
        let config = RulesConfig {
            end_at_critical_heat: false,
            ..RulesConfig::default()
        };
        assert!(evaluate(&power(Some(80), ThermalLevel::Critical), &config).is_empty());
        // The battery rule still fires.
        assert_eq!(
            evaluate(&power(Some(5), ThermalLevel::Critical), &config).len(),
            1
        );
    }
}
