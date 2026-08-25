//! Cron preset builder — port of Angular `cron-input.component.ts`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CronPreset {
    pub key: &'static str,
    pub label: &'static str,
    pub cron: &'static str,
}

pub const PRESETS: &[CronPreset] = &[
    CronPreset {
        key: "daily-9am",
        label: "Daily 9:00",
        cron: "0 9 * * *",
    },
    CronPreset {
        key: "daily-6pm",
        label: "Daily 18:00",
        cron: "0 18 * * *",
    },
    CronPreset {
        key: "weekday-9am",
        label: "Weekdays 9:00",
        cron: "0 9 * * 1-5",
    },
    CronPreset {
        key: "weekly-monday",
        label: "Monday 9:00",
        cron: "0 9 * * 1",
    },
    CronPreset {
        key: "every-6-hours",
        label: "Every 6h",
        cron: "0 */6 * * *",
    },
    CronPreset {
        key: "monthly-1st",
        label: "Monthly 1st",
        cron: "0 0 1 * *",
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SimpleFrequency {
    Daily,
    Weekly,
    Monthly,
    Interval,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleCron {
    pub frequency: SimpleFrequency,
    pub minute: u32,
    pub hour: u32,
    pub day_of_week: String,
    pub day_of_month: u32,
    pub interval_hours: u32,
}

impl Default for SimpleCron {
    fn default() -> Self {
        Self {
            frequency: SimpleFrequency::Daily,
            minute: 0,
            hour: 9,
            day_of_week: "1".into(),
            day_of_month: 1,
            interval_hours: 6,
        }
    }
}

pub fn build_simple(simple: &SimpleCron) -> String {
    match simple.frequency {
        SimpleFrequency::Daily => format!("{} {} * * *", simple.minute, simple.hour),
        SimpleFrequency::Weekly => format!(
            "{} {} * * {}",
            simple.minute, simple.hour, simple.day_of_week
        ),
        SimpleFrequency::Monthly => format!(
            "{} {} {} * *",
            simple.minute, simple.hour, simple.day_of_month
        ),
        SimpleFrequency::Interval => format!("0 */{} * * *", simple.interval_hours.max(1)),
    }
}

pub fn build_advanced(
    minute: &str,
    hour: &str,
    day_of_month: &str,
    month: &str,
    day_of_week: &str,
) -> String {
    format!(
        "{} {} {} {} {}",
        nonempty(minute, "0"),
        nonempty(hour, "*"),
        nonempty(day_of_month, "*"),
        nonempty(month, "*"),
        nonempty(day_of_week, "*")
    )
}

fn nonempty<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() {
        fallback
    } else {
        value.trim()
    }
}

pub fn split_cron(cron: &str) -> Option<[String; 5]> {
    let parts: Vec<&str> = cron.split_whitespace().collect();
    if parts.len() != 5 {
        return None;
    }
    Some([
        parts[0].to_string(),
        parts[1].to_string(),
        parts[2].to_string(),
        parts[3].to_string(),
        parts[4].to_string(),
    ])
}

pub fn preset_for(cron: &str) -> Option<&'static CronPreset> {
    PRESETS.iter().find(|preset| preset.cron == cron.trim())
}

pub fn parse_simple(cron: &str) -> Option<SimpleCron> {
    let [min, hour, dom, mon, dow] = split_cron(cron)?;
    let is_num = |s: &str| s.chars().all(|c| c.is_ascii_digit());
    if min == "0" && hour.starts_with("*/") && dom == "*" && mon == "*" && dow == "*" {
        let hours = hour.trim_start_matches("*/").parse().unwrap_or(6);
        return Some(SimpleCron {
            frequency: SimpleFrequency::Interval,
            interval_hours: hours,
            ..SimpleCron::default()
        });
    }
    if is_num(&min) && is_num(&hour) && is_num(&dom) && mon == "*" && dow == "*" {
        return Some(SimpleCron {
            frequency: SimpleFrequency::Monthly,
            minute: min.parse().unwrap_or(0),
            hour: hour.parse().unwrap_or(0),
            day_of_month: dom.parse().unwrap_or(1),
            ..SimpleCron::default()
        });
    }
    if is_num(&min)
        && is_num(&hour)
        && dom == "*"
        && mon == "*"
        && (is_num(&dow) || dow.contains('-'))
    {
        return Some(SimpleCron {
            frequency: SimpleFrequency::Weekly,
            minute: min.parse().unwrap_or(0),
            hour: hour.parse().unwrap_or(0),
            day_of_week: dow,
            ..SimpleCron::default()
        });
    }
    if is_num(&min) && is_num(&hour) && dom == "*" && mon == "*" && dow == "*" {
        return Some(SimpleCron {
            frequency: SimpleFrequency::Daily,
            minute: min.parse().unwrap_or(0),
            hour: hour.parse().unwrap_or(9),
            ..SimpleCron::default()
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_match_angular() {
        assert_eq!(PRESETS.len(), 6);
        assert_eq!(preset_for("0 9 * * *").unwrap().key, "daily-9am");
        assert_eq!(preset_for("0 */6 * * *").unwrap().key, "every-6-hours");
        assert!(preset_for("1 2 3 4 5").is_none());
    }

    #[test]
    fn simple_builder_round_trips() {
        let daily = SimpleCron::default();
        assert_eq!(build_simple(&daily), "0 9 * * *");
        let parsed = parse_simple("30 18 * * *").unwrap();
        assert_eq!(parsed.frequency, SimpleFrequency::Daily);
        assert_eq!(parsed.hour, 18);
        assert_eq!(parsed.minute, 30);
        let weekly = parse_simple("0 9 * * 1-5").unwrap();
        assert_eq!(weekly.frequency, SimpleFrequency::Weekly);
        assert_eq!(weekly.day_of_week, "1-5");
        let monthly = parse_simple("0 0 1 * *").unwrap();
        assert_eq!(monthly.frequency, SimpleFrequency::Monthly);
        let interval = parse_simple("0 */6 * * *").unwrap();
        assert_eq!(interval.frequency, SimpleFrequency::Interval);
        assert_eq!(interval.interval_hours, 6);
        assert_eq!(
            build_simple(&SimpleCron {
                frequency: SimpleFrequency::Interval,
                interval_hours: 4,
                ..SimpleCron::default()
            }),
            "0 */4 * * *"
        );
    }

    #[test]
    fn advanced_joins_five_fields() {
        assert_eq!(build_advanced("15", "10", "*", "*", "1-5"), "15 10 * * 1-5");
        assert_eq!(build_advanced("", "", "", "", ""), "0 * * * *");
        assert_eq!(split_cron("0 9 * * *").unwrap()[1], "9");
        assert!(split_cron("too short").is_none());
    }
}
