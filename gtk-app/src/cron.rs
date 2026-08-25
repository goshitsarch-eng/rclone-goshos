//! Cron preset builder — port of Angular `cron-input.component.ts`.

use chrono::{DateTime, Utc};

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

/// Angular simple-form weekday options plus weekends from the advanced builder.
pub const WEEKDAY_CHOICES: &[(&str, &str, &str)] = &[
    ("1", "wizards.appOperation.days.monday", "Monday"),
    ("2", "wizards.appOperation.days.tuesday", "Tuesday"),
    ("3", "wizards.appOperation.days.wednesday", "Wednesday"),
    ("4", "wizards.appOperation.days.thursday", "Thursday"),
    ("5", "wizards.appOperation.days.friday", "Friday"),
    ("6", "wizards.appOperation.days.saturday", "Saturday"),
    ("0", "wizards.appOperation.days.sunday", "Sunday"),
    (
        "1-5",
        "wizards.appOperation.days.weekdays",
        "Weekdays (Mon-Fri)",
    ),
    (
        "0,6",
        "wizards.appOperation.days.weekends",
        "Weekends (Sat-Sun)",
    ),
];

pub fn weekday_index(dow: &str) -> Option<u32> {
    WEEKDAY_CHOICES
        .iter()
        .position(|(value, _, _)| *value == dow.trim())
        .map(|i| i as u32)
}

pub fn weekday_value(index: u32) -> &'static str {
    WEEKDAY_CHOICES
        .get(index as usize)
        .map(|(value, _, _)| *value)
        .unwrap_or("1")
}

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
        && (is_num(&dow) || dow.contains('-') || dow.contains(','))
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

pub fn user_timezone_label() -> String {
    let now = chrono::Local::now();
    let offset = now.offset().local_minus_utc();
    let sign = if offset >= 0 { '+' } else { '-' };
    let abs = offset.unsigned_abs();
    format!(
        "{} (UTC{sign}{:02}:{:02})",
        timezone_name(),
        abs / 3600,
        (abs % 3600) / 60
    )
}

fn timezone_name() -> String {
    if let Ok(tz) = std::env::var("TZ") {
        let tz = tz.trim();
        if !tz.is_empty() && tz != ":/etc/localtime" {
            return tz.to_string();
        }
    }
    if let Ok(name) = std::fs::read_to_string("/etc/timezone") {
        let name = name.trim();
        if !name.is_empty() {
            return name.to_string();
        }
    }
    chrono::Local::now().format("%Z").to_string()
}

pub fn format_relative_span(i18n: &crate::i18n::I18n, seconds: i64) -> String {
    if seconds < 60 {
        return i18n.t_or(
            "wizards.appOperation.relativeTime.lessThanMinute",
            "in less than a minute",
        );
    }
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3600;
    let minutes = (seconds % 3600) / 60;
    let unit = |one: &str, other: &str, one_fb: &str, other_fb: &str, count: i64| {
        if count == 1 {
            i18n.t_or(one, one_fb)
        } else if i18n.has(other) {
            i18n.tf(other, &[("count", &count.to_string())])
        } else {
            other_fb.replace("{count}", &count.to_string())
        }
    };
    let mut parts = Vec::new();
    if days > 0 {
        parts.push(unit(
            "wizards.appOperation.relativeTime.days.one",
            "wizards.appOperation.relativeTime.days.other",
            "1 day",
            "{count} days",
            days,
        ));
        if hours > 0 {
            parts.push(unit(
                "wizards.appOperation.relativeTime.hours.one",
                "wizards.appOperation.relativeTime.hours.other",
                "1 hour",
                "{count} hours",
                hours,
            ));
        }
    } else if hours > 0 {
        parts.push(unit(
            "wizards.appOperation.relativeTime.hours.one",
            "wizards.appOperation.relativeTime.hours.other",
            "1 hour",
            "{count} hours",
            hours,
        ));
        if minutes > 0 {
            parts.push(unit(
                "wizards.appOperation.relativeTime.minutes.one",
                "wizards.appOperation.relativeTime.minutes.other",
                "1 minute",
                "{count} minutes",
                minutes,
            ));
        }
    } else {
        parts.push(unit(
            "wizards.appOperation.relativeTime.minutes.one",
            "wizards.appOperation.relativeTime.minutes.other",
            "1 minute",
            "{count} minutes",
            minutes,
        ));
    }
    parts.join(", ")
}

pub fn format_next_run(
    i18n: &crate::i18n::I18n,
    next: DateTime<Utc>,
    now: DateTime<Utc>,
) -> String {
    let date = next
        .with_timezone(&chrono::Local)
        .format("%Y-%m-%d %H:%M")
        .to_string();
    let delta = next.signed_duration_since(now).num_seconds();
    if delta < 0 {
        return if i18n.has("wizards.appOperation.relativeTime.inPast") {
            i18n.tf(
                "wizards.appOperation.relativeTime.inPast",
                &[("date", &date)],
            )
        } else {
            format!("in the past ({date})")
        };
    }
    let relative = format_relative_span(i18n, delta);
    let lead = if i18n.has("wizards.appOperation.relativeTime.in") {
        i18n.tf(
            "wizards.appOperation.relativeTime.in",
            &[("time", &relative)],
        )
    } else {
        format!("in {relative}")
    };
    format!("{lead} ({date})")
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

    #[test]
    fn weekday_index_matches_choices() {
        assert_eq!(weekday_index("1"), Some(0));
        assert_eq!(weekday_index("0"), Some(6));
        assert_eq!(weekday_index("1-5"), Some(7));
        assert_eq!(weekday_index("0,6"), Some(8));
        assert_eq!(weekday_index("2-4"), None);
        assert_eq!(weekday_value(7), "1-5");
        assert_eq!(weekday_value(8), "0,6");
        assert_eq!(weekday_value(99), "1");
    }

    #[test]
    fn parse_simple_accepts_weekend_list() {
        let parsed = parse_simple("0 9 * * 0,6").unwrap();
        assert_eq!(parsed.frequency, SimpleFrequency::Weekly);
        assert_eq!(parsed.day_of_week, "0,6");
        assert_eq!(
            build_simple(&SimpleCron {
                frequency: SimpleFrequency::Weekly,
                day_of_week: "0,6".into(),
                ..SimpleCron::default()
            }),
            "0 9 * * 0,6"
        );
    }

    #[test]
    fn format_next_run_past_and_future() {
        let i18n = crate::i18n::I18n::default();
        let now = DateTime::parse_from_rfc3339("2026-08-25T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let past = now - chrono::Duration::hours(2);
        let past_text = format_next_run(&i18n, past, now);
        assert!(past_text.contains("past") || past_text.contains("2026"));
        let future = now + chrono::Duration::minutes(90);
        let future_text = format_next_run(&i18n, future, now);
        assert!(future_text.contains("in"));
        assert!(future_text.contains("1 hour") || future_text.contains("hour"));
        assert!(user_timezone_label().contains("UTC"));
        assert_eq!(
            format_relative_span(&i18n, 30),
            i18n.t_or(
                "wizards.appOperation.relativeTime.lessThanMinute",
                "in less than a minute"
            )
        );
    }
}
