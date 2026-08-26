//! Spawn and supervise `rclone rcd`.

use super::client::RcClient;
use crate::settings::AppSettings;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct RcloneEngine {
    pub client: RcClient,
    pub binary: PathBuf,
    pub port: u16,
    child: Option<Child>,
    pub version: String,
    pub available: bool,
    pub config_path: Option<PathBuf>,
}

impl RcloneEngine {
    pub fn start(settings: &AppSettings) -> Self {
        let binary = resolve_rclone_binary(&settings.core.rclone_binary);
        let port = pick_free_port().unwrap_or(5572);
        let client = RcClient::new("127.0.0.1", port);
        let mut engine = Self {
            client,
            binary: binary.clone(),
            port,
            child: None,
            version: String::new(),
            available: false,
            config_path: None,
        };

        if !binary.exists() {
            log::warn!("rclone binary not found at {}", binary.display());
            return engine;
        }

        let mut cmd = Command::new(&binary);
        for arg in rcd_base_args(port) {
            cmd.arg(arg);
        }
        cmd.stdout(Stdio::null()).stderr(Stdio::null());

        for flag in &settings.core.rclone_additional_flags {
            if is_reserved_flag(flag) {
                continue;
            }
            cmd.arg(flag);
        }
        if let Some(path) =
            crate::repair::config_path_from_flags(&settings.core.rclone_additional_flags)
        {
            cmd.arg(format!("--config={path}"));
            engine.config_path = Some(PathBuf::from(&path));
        }
        for env in &settings.core.rclone_env_vars {
            if let Some((k, v)) = env.split_once('=') {
                cmd.env(k, v);
            }
        }
        let password = crate::keyring::resolve_config_password(&settings.core.config_password);
        crate::security::apply_config_password_env(&mut cmd, &password);
        crate::repair::apply_fusermount_path(&mut cmd);
        let log_path = crate::settings::AppSettings::log_path();
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        cmd.arg(format!("--log-file={}", log_path.display()));
        cmd.arg("--log-level=INFO");

        match cmd.spawn() {
            Ok(child) => {
                engine.child = Some(child);
                let deadline = Instant::now() + Duration::from_secs(8);
                while Instant::now() < deadline {
                    if engine.client.ping() {
                        engine.available = true;
                        engine.version = engine.client.version().unwrap_or_default();
                        if !password.is_empty() {
                            let _ = engine.client.config_unlock(&password);
                        }
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(150));
                }
                if !engine.available {
                    log::error!("rclone rcd started but RC did not become ready");
                }
            }
            Err(err) => log::error!("failed to spawn rclone: {err}"),
        }
        crate::rclone::serve::install_spawn_context(
            crate::rclone::serve::spawn_context_from_settings(
                binary,
                engine.config_path.clone(),
                &settings.core.rclone_additional_flags,
                &settings.core.rclone_env_vars,
                &password,
            ),
        );
        engine
    }

    pub fn restart(&mut self, settings: &AppSettings) {
        self.shutdown();
        *self = Self::start(settings);
    }

    pub fn shutdown(&mut self) {
        crate::rclone::serve::shutdown_legacy(Some(&self.client));
        crate::rclone::serve::clear_spawn_context();
        if self.available {
            let _ = self.client.quit();
        }
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;
        self.available = false;
    }

    pub fn provision_status(&self) -> &'static str {
        if self.available {
            "ready"
        } else if self.binary.exists() {
            "starting"
        } else {
            "missing"
        }
    }
}

impl Drop for RcloneEngine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub fn resolve_rclone_binary(configured: &str) -> PathBuf {
    if !configured.is_empty() {
        return PathBuf::from(configured);
    }
    which::which("rclone").unwrap_or_else(|_| PathBuf::from("rclone"))
}

pub fn rclone_exists(configured: &str) -> bool {
    let path = resolve_rclone_binary(configured);
    path.exists() || which::which("rclone").is_ok()
}

fn pick_free_port() -> Option<u16> {
    TcpListener::bind("127.0.0.1:0")
        .ok()
        .and_then(|listener| listener.local_addr().ok().map(|a| a.port()))
}

/// Base `rclone rcd` arguments. `--rc-serve` enables HTTP Range streaming of
/// remote files (same as the Tauri desktop engine).
pub fn rcd_base_args(port: u16) -> Vec<String> {
    vec![
        "rcd".into(),
        format!("--rc-addr=127.0.0.1:{port}"),
        "--rc-no-auth".into(),
        "--rc-web-gui=false".into(),
        "--rc-serve".into(),
    ]
}

pub fn is_reserved_flag(flag: &str) -> bool {
    const RESERVED: &[&str] = &[
        "rcd",
        "--config",
        "--rc",
        "--rc-serve",
        "--rc-addr",
        "--rc-allow-origin",
        "--log-file",
        "--rc-user",
        "--rc-pass",
        "--rc-no-auth",
        "--rc-template",
        "--log-file-max-size",
        "--log-file-max-backups",
    ];
    RESERVED
        .iter()
        .any(|r| flag == *r || flag.starts_with(&format!("{r}=")))
}

pub fn validate_cron(expression: &str) -> Result<(), String> {
    if expression.trim().is_empty() {
        return Err("empty cron expression".into());
    }
    croner::Cron::new(expression)
        .parse()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

pub fn describe_cron(expression: &str) -> String {
    describe_cron_i18n(expression, &crate::i18n::I18n::default())
}

fn cron_time(hour: &str, min: &str) -> String {
    format!("{hour}:{min:0>2}")
}

fn cron_is_int(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_ascii_digit())
}

fn cron_is_int_list(value: &str) -> bool {
    !value.is_empty() && value.split(',').all(|part| cron_is_int(part.trim()))
}

fn cron_join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [a, b] => format!("{a} and {b}"),
        _ => {
            let last = items[items.len() - 1].clone();
            format!("{} and {last}", items[..items.len() - 1].join(", "))
        }
    }
}

fn cron_range_step(value: &str) -> Option<(&str, &str, &str)> {
    let (range, step) = value.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    if cron_is_int(start) && cron_is_int(end) && cron_is_int(step) {
        Some((start, end, step))
    } else {
        None
    }
}

fn cron_describe_hour_part(part: &str, min: &str) -> String {
    if let Some((start, end)) = part.split_once('-') {
        if cron_is_int(start) && cron_is_int(end) {
            return format!("{}-{}", cron_time(start, min), cron_time(end, min));
        }
    }
    if cron_is_int(part) {
        return cron_time(part, min);
    }
    part.to_string()
}

fn cron_is_weekend(dow: &str) -> bool {
    matches!(dow, "0,6" | "6,0" | "0,6,7" | "6,0,7")
}

fn cron_day_name(dow: &str, i18n: &crate::i18n::I18n) -> String {
    let key = format!("cron.days.{dow}");
    i18n.t_or(
        &key,
        match dow {
            "0" | "7" => "Sunday",
            "1" => "Monday",
            "2" => "Tuesday",
            "3" => "Wednesday",
            "4" => "Thursday",
            "5" => "Friday",
            "6" => "Saturday",
            other => other,
        },
    )
}

fn cron_ordinal(n: &str) -> String {
    match n {
        "1" => "1st".into(),
        "2" => "2nd".into(),
        "3" => "3rd".into(),
        other => format!("{other}th"),
    }
}

fn cron_month_list(mon: &str, i18n: &crate::i18n::I18n) -> String {
    mon.split(',')
        .map(|m| cron_month_name(m.trim(), i18n))
        .collect::<Vec<_>>()
        .join(", ")
}

fn cron_month_name(mon: &str, i18n: &crate::i18n::I18n) -> String {
    let key = format!("cron.months.{mon}");
    i18n.t_or(
        &key,
        match mon {
            "1" => "January",
            "2" => "February",
            "3" => "March",
            "4" => "April",
            "5" => "May",
            "6" => "June",
            "7" => "July",
            "8" => "August",
            "9" => "September",
            "10" => "October",
            "11" => "November",
            "12" => "December",
            other => other,
        },
    )
}

fn cron_day_list(dow: &str, i18n: &crate::i18n::I18n) -> String {
    dow.split(',')
        .map(|d| cron_day_name(d.trim(), i18n))
        .collect::<Vec<_>>()
        .join(", ")
}

fn cron_expand_nickname(expression: &str) -> String {
    match expression.to_ascii_lowercase().as_str() {
        "@yearly" | "@annually" => "0 0 1 1 *".into(),
        "@monthly" => "0 0 1 * *".into(),
        "@weekly" => "0 0 * * 0".into(),
        "@daily" | "@midnight" => "0 0 * * *".into(),
        "@hourly" => "0 * * * *".into(),
        _ => expression.to_string(),
    }
}

fn cron_normalize_dow(dow: &str) -> String {
    let mut out = dow.to_ascii_uppercase();
    for (name, num) in [
        ("SUN", "0"),
        ("MON", "1"),
        ("TUE", "2"),
        ("WED", "3"),
        ("THU", "4"),
        ("FRI", "5"),
        ("SAT", "6"),
    ] {
        out = out.replace(name, num);
    }
    out
}

fn cron_normalize_mon(mon: &str) -> String {
    let mut out = mon.to_ascii_uppercase();
    for (name, num) in [
        ("JANUARY", "1"),
        ("FEBRUARY", "2"),
        ("SEPTEMBER", "9"),
        ("OCTOBER", "10"),
        ("NOVEMBER", "11"),
        ("DECEMBER", "12"),
        ("MARCH", "3"),
        ("APRIL", "4"),
        ("JUNE", "6"),
        ("JULY", "7"),
        ("AUGUST", "8"),
        ("JAN", "1"),
        ("FEB", "2"),
        ("MAR", "3"),
        ("APR", "4"),
        ("MAY", "5"),
        ("JUN", "6"),
        ("JUL", "7"),
        ("AUG", "8"),
        ("SEP", "9"),
        ("OCT", "10"),
        ("NOV", "11"),
        ("DEC", "12"),
    ] {
        out = out.replace(name, num);
    }
    out
}

fn cron_format_named_field(value: &str, name_fn: impl Fn(&str) -> String) -> String {
    value
        .split(',')
        .map(|part| {
            let part = part.trim();
            if let Some((start, end)) = part.split_once('-') {
                if !start.eq_ignore_ascii_case("L") {
                    return format!("{}-{}", name_fn(start.trim()), name_fn(end.trim()));
                }
            }
            name_fn(part)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn describe_cron_time_fragment(min: &str, hour: &str, i18n: &crate::i18n::I18n) -> String {
    if cron_is_int(min) && cron_is_int(hour) {
        let time = cron_time(hour, min);
        return i18n.tf_or("cron.atTime", &format!("At {time}"), &[("time", &time)]);
    }
    if cron_is_int(min) && cron_is_int_list(hour) {
        let times = cron_join_and(
            &hour
                .split(',')
                .map(|h| cron_time(h.trim(), min))
                .collect::<Vec<_>>(),
        );
        return i18n.tf_or("cron.atTimes", &format!("At {times}"), &[("times", &times)]);
    }
    if min.starts_with("*/") && (hour == "*" || hour.is_empty()) {
        let n = min.trim_start_matches("*/");
        return i18n.tf_or(
            "cron.everyMinutes",
            &format!("Every {n} minutes"),
            &[("n", n)],
        );
    }
    i18n.tf_or(
        "cron.atMinuteHour",
        &format!("At minute {min} of hour {hour}"),
        &[("min", min), ("hour", hour)],
    )
}

fn describe_cron_dom_fragment(dom: &str, i18n: &crate::i18n::I18n) -> String {
    if dom.eq_ignore_ascii_case("L") {
        return i18n.t_or("cron.onLastDay", "on the last day of the month");
    }
    if dom.eq_ignore_ascii_case("LW") {
        return i18n.t_or("cron.onLastWeekday", "on the last weekday of the month");
    }
    if let Some(n) = dom.strip_prefix("*/") {
        return i18n.tf_or("cron.onEveryNDays", "every {{n}} days", &[("n", n)]);
    }
    if cron_is_int(dom) {
        return i18n.tf_or("cron.onDay", "on day {{day}}", &[("day", dom)]);
    }
    if dom.len() > 1 && dom.ends_with(['W', 'w']) {
        let day = dom.trim_end_matches(['W', 'w']);
        return i18n.tf_or(
            "cron.onNearestWeekday",
            "on the nearest weekday to day {{day}}",
            &[("day", day)],
        );
    }
    i18n.tf_or("cron.onDays", "on days {{days}}", &[("days", dom)])
}

fn describe_cron_mon_fragment(mon: &str, i18n: &crate::i18n::I18n) -> String {
    let months = cron_format_named_field(mon, |m| cron_month_name(m, i18n));
    i18n.tf_or("cron.inMonths", "in {{months}}", &[("months", &months)])
}

fn describe_cron_dow_fragment(dow: &str, i18n: &crate::i18n::I18n) -> String {
    if dow == "1-5" {
        return i18n.t_or("cron.onWeekdaysWord", "on weekdays");
    }
    if cron_is_weekend(dow) {
        return i18n.t_or("cron.onWeekendsWord", "on weekends");
    }
    if let Some((day, nth)) = dow.split_once('#') {
        if cron_is_int(day) && cron_is_int(nth) {
            let day_name = cron_day_name(day, i18n);
            let nth = cron_ordinal(nth);
            return i18n.tf_or(
                "cron.onNthWeekday",
                "on the {{nth}} {{day}}",
                &[("nth", &nth), ("day", &day_name)],
            );
        }
    }
    if let Some(day) = dow.strip_suffix('L').or_else(|| dow.strip_suffix('l')) {
        if cron_is_int(day) {
            let day_name = cron_day_name(day, i18n);
            return i18n.tf_or(
                "cron.onLastNamedWeekday",
                "on the last {{day}}",
                &[("day", &day_name)],
            );
        }
    }
    let days = cron_format_named_field(dow, |d| cron_day_name(d, i18n));
    i18n.tf_or("cron.onWeekdays", "on {{days}}", &[("days", &days)])
}

fn describe_cron_generic(
    min: &str,
    hour: &str,
    dom: &str,
    mon: &str,
    dow: &str,
    i18n: &crate::i18n::I18n,
) -> String {
    let head = describe_cron_time_fragment(min, hour, i18n);
    let mut tail = Vec::new();
    if dom != "*" {
        tail.push(describe_cron_dom_fragment(dom, i18n));
    }
    if mon != "*" {
        tail.push(describe_cron_mon_fragment(mon, i18n));
    }
    if dow != "*" {
        tail.push(describe_cron_dow_fragment(dow, i18n));
    }
    tail.retain(|part| !part.is_empty());
    if tail.is_empty() {
        return head;
    }
    let tail = tail.join(", ");
    i18n.tf_or(
        "cron.composed",
        &format!("{head}, {tail}"),
        &[("head", &head), ("tail", &tail)],
    )
}

pub fn describe_cron_i18n(expression: &str, i18n: &crate::i18n::I18n) -> String {
    let trimmed = expression.trim();
    if trimmed.eq_ignore_ascii_case("@reboot") {
        return i18n.t_or("cron.atReboot", "At reboot");
    }
    let normalized = cron_expand_nickname(trimmed);
    let parts: Vec<&str> = normalized.split_whitespace().collect();
    let fields = if parts.len() >= 6 {
        let last = parts[parts.len() - 1];
        if last
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == '_' || c == '/')
        {
            &parts[..5]
        } else {
            &parts[parts.len() - 5..]
        }
    } else {
        &parts[..]
    };
    if fields.len() < 5 {
        return expression.to_string();
    }
    let (min, hour, dom, mon, dow_raw) = (fields[0], fields[1], fields[2], fields[3], fields[4]);
    let dow_owned = cron_normalize_dow(dow_raw);
    let dow = dow_owned.as_str();
    let mon_owned = cron_normalize_mon(mon);
    let mon = mon_owned.as_str();
    if min.starts_with("*/")
        && hour.contains('-')
        && cron_is_int(hour.split('-').next().unwrap_or(""))
        && cron_is_int(hour.split('-').nth(1).unwrap_or(""))
        && !hour.contains(',')
        && dom == "*"
        && mon == "*"
        && dow == "*"
    {
        let n = min.trim_start_matches("*/");
        let start = cron_time(hour.split('-').next().unwrap_or(hour), "0");
        let end = cron_time(hour.split('-').nth(1).unwrap_or(hour), "0");
        return if i18n.has("cron.everyMinutesBetween") {
            i18n.tf(
                "cron.everyMinutesBetween",
                &[("n", n), ("start", &start), ("end", &end)],
            )
        } else {
            format!("Every {n} minutes between {start} and {end}")
        };
    }
    if min.starts_with("*/") && hour == "*" && dom == "*" && mon == "*" && dow == "*" {
        let n = min.trim_start_matches("*/");
        return if i18n.has("cron.everyMinutes") {
            i18n.tf("cron.everyMinutes", &[("n", n)])
        } else {
            format!("Every {n} minutes")
        };
    }
    if cron_is_int(min) && hour.starts_with("*/") && dom == "*" && mon == "*" && dow == "*" {
        let n = hour.trim_start_matches("*/");
        if min == "0" {
            return if i18n.has("cron.everyHours") {
                i18n.tf("cron.everyHours", &[("n", n)])
            } else {
                format!("Every {n} hours")
            };
        }
        return if i18n.has("cron.everyHoursAt") {
            i18n.tf("cron.everyHoursAt", &[("n", n), ("min", min)])
        } else {
            format!("Every {n} hours at minute {min}")
        };
    }
    if min == "*" && hour == "*" && dom == "*" && mon == "*" && dow == "*" {
        return i18n.t_or("cron.everyMinute", "Every minute");
    }
    if cron_is_int(min) && hour == "*" && dom == "*" && mon == "*" && dow == "*" {
        return if i18n.has("cron.hourlyAt") {
            i18n.tf("cron.hourlyAt", &[("min", min)])
        } else {
            format!("Hourly at minute {min}")
        };
    }
    if cron_is_int_list(min) && hour == "*" && dom == "*" && mon == "*" && dow == "*" {
        return if i18n.has("cron.hourlyAtMinutes") {
            i18n.tf("cron.hourlyAtMinutes", &[("mins", min)])
        } else {
            format!("Hourly at minutes {min}")
        };
    }
    if cron_is_int(min) && cron_is_int(hour) && dom == "*" && mon == "*" && dow == "*" {
        let time = cron_time(hour, min);
        return if i18n.has("cron.dailyAt") {
            i18n.tf("cron.dailyAt", &[("time", &time)])
        } else {
            format!("Daily at {time}")
        };
    }
    if cron_is_int(min) && cron_is_int_list(hour) && dom == "*" && mon == "*" && dow == "*" {
        let times = cron_join_and(
            &hour
                .split(',')
                .map(|h| cron_time(h.trim(), min))
                .collect::<Vec<_>>(),
        );
        return if i18n.has("cron.atTimes") {
            i18n.tf("cron.atTimes", &[("times", &times)])
        } else {
            format!("At {times}")
        };
    }
    if let Some((start, end, step)) = cron_range_step(hour) {
        if cron_is_int(min) && dom == "*" && mon == "*" && dow == "*" {
            let start_t = cron_time(start, min);
            let end_t = cron_time(end, min);
            return if i18n.has("cron.everyHoursBetween") {
                i18n.tf(
                    "cron.everyHoursBetween",
                    &[("n", step), ("start", &start_t), ("end", &end_t)],
                )
            } else {
                format!("Every {step} hours between {start_t} and {end_t}")
            };
        }
    }
    if min.starts_with("*/")
        && cron_is_int_list(hour)
        && !hour.contains('-')
        && dom == "*"
        && mon == "*"
        && dow == "*"
    {
        let n = min.trim_start_matches("*/");
        let times = cron_join_and(
            &hour
                .split(',')
                .map(|h| cron_time(h.trim(), "0"))
                .collect::<Vec<_>>(),
        );
        return if i18n.has("cron.everyMinutesAt") {
            i18n.tf("cron.everyMinutesAt", &[("n", n), ("times", &times)])
        } else {
            format!("Every {n} minutes at {times}")
        };
    }
    if min.contains(',')
        && cron_is_int_list(min)
        && hour.contains('-')
        && !hour.contains(',')
        && !hour.contains('/')
        && cron_is_int(hour.split('-').next().unwrap_or(""))
        && cron_is_int(hour.split('-').nth(1).unwrap_or(""))
        && dom == "*"
        && mon == "*"
        && dow == "*"
    {
        let start = cron_time(hour.split('-').next().unwrap_or(hour), "0");
        let end = cron_time(hour.split('-').nth(1).unwrap_or(hour), "0");
        return if i18n.has("cron.atMinutesBetween") {
            i18n.tf(
                "cron.atMinutesBetween",
                &[("mins", min), ("start", &start), ("end", &end)],
            )
        } else {
            format!("At minutes {min} between {start} and {end}")
        };
    }
    if cron_is_int(min)
        && hour.contains(',')
        && hour.contains('-')
        && dom == "*"
        && mon == "*"
        && dow == "*"
    {
        let hours = cron_join_and(
            &hour
                .split(',')
                .map(|part| cron_describe_hour_part(part.trim(), min))
                .collect::<Vec<_>>(),
        );
        return if i18n.has("cron.atHours") {
            i18n.tf("cron.atHours", &[("hours", &hours)])
        } else {
            format!("At {hours}")
        };
    }
    if cron_is_int(min)
        && (cron_is_int(hour) || cron_is_int_list(hour))
        && dom == "*"
        && mon == "*"
        && dow == "1-5"
    {
        if cron_is_int(hour) {
            let time = cron_time(hour, min);
            return if i18n.has("cron.weekdaysAt") {
                i18n.tf("cron.weekdaysAt", &[("time", &time)])
            } else {
                format!("Weekdays at {time}")
            };
        }
        let times = cron_join_and(
            &hour
                .split(',')
                .map(|h| cron_time(h.trim(), min))
                .collect::<Vec<_>>(),
        );
        return i18n.tf_or(
            "cron.weekdaysAtTimes",
            &format!("Weekdays at {times}"),
            &[("times", &times)],
        );
    }
    if cron_is_int(min)
        && (cron_is_int(hour) || cron_is_int_list(hour))
        && dom == "*"
        && mon == "*"
        && cron_is_weekend(dow)
    {
        if cron_is_int(hour) {
            let time = cron_time(hour, min);
            return if i18n.has("cron.weekendsAt") {
                i18n.tf("cron.weekendsAt", &[("time", &time)])
            } else {
                format!("Weekends at {time}")
            };
        }
        let times = cron_join_and(
            &hour
                .split(',')
                .map(|h| cron_time(h.trim(), min))
                .collect::<Vec<_>>(),
        );
        return i18n.tf_or(
            "cron.weekendsAtTimes",
            &format!("Weekends at {times}"),
            &[("times", &times)],
        );
    }
    if cron_is_int(min)
        && cron_is_int(hour)
        && dom.eq_ignore_ascii_case("L")
        && mon == "*"
        && dow == "*"
    {
        let time = cron_time(hour, min);
        return if i18n.has("cron.lastDayOfMonthAt") {
            i18n.tf("cron.lastDayOfMonthAt", &[("time", &time)])
        } else {
            format!("Last day of the month at {time}")
        };
    }
    if cron_is_int(min)
        && cron_is_int(hour)
        && dom.to_ascii_uppercase().starts_with("L-")
        && mon == "*"
        && dow == "*"
    {
        let offset = dom.trim_start_matches(['L', 'l', '-']);
        let time = cron_time(hour, min);
        return if i18n.has("cron.lastDayMinusAt") {
            i18n.tf("cron.lastDayMinusAt", &[("n", offset), ("time", &time)])
        } else {
            format!("Last day of the month minus {offset} at {time}")
        };
    }
    if cron_is_int(min)
        && cron_is_int(hour)
        && dom.eq_ignore_ascii_case("LW")
        && mon == "*"
        && dow == "*"
    {
        let time = cron_time(hour, min);
        return if i18n.has("cron.lastWeekdayOfMonthAt") {
            i18n.tf("cron.lastWeekdayOfMonthAt", &[("time", &time)])
        } else {
            format!("Last weekday of the month at {time}")
        };
    }
    if cron_is_int(min) && cron_is_int(hour) && dom == "*" && mon == "*" {
        if let Some((day, nth)) = dow.split_once('#') {
            if cron_is_int(day) && cron_is_int(nth) {
                let time = cron_time(hour, min);
                let day_name = cron_day_name(day, i18n);
                let nth = cron_ordinal(nth);
                return if i18n.has("cron.nthWeekdayAt") {
                    i18n.tf(
                        "cron.nthWeekdayAt",
                        &[("nth", &nth), ("day", &day_name), ("time", &time)],
                    )
                } else {
                    format!("{nth} {day_name} at {time}")
                };
            }
        }
        if let Some(day) = dow.strip_suffix('L').or_else(|| dow.strip_suffix('l')) {
            if cron_is_int(day) {
                let time = cron_time(hour, min);
                let day_name = cron_day_name(day, i18n);
                return if i18n.has("cron.lastWeekdayAt") {
                    i18n.tf("cron.lastWeekdayAt", &[("day", &day_name), ("time", &time)])
                } else {
                    format!("Last {day_name} at {time}")
                };
            }
        }
    }
    if cron_is_int(min)
        && cron_is_int(hour)
        && mon != "*"
        && dow == "*"
        && (cron_is_int(dom) || dom == "*")
        && !cron_is_int(mon)
    {
        let time = cron_time(hour, min);
        let months = cron_month_list(mon, i18n);
        if cron_is_int(dom) {
            return if i18n.has("cron.inMonthsOnDayAt") {
                i18n.tf(
                    "cron.inMonthsOnDayAt",
                    &[("months", &months), ("dom", dom), ("time", &time)],
                )
            } else {
                format!("In {months} on day {dom} at {time}")
            };
        }
        return if i18n.has("cron.inMonthsAt") {
            i18n.tf("cron.inMonthsAt", &[("months", &months), ("time", &time)])
        } else {
            format!("In {months} at {time}")
        };
    }
    if cron_is_int(min) && cron_is_int(hour) && dom == "*" && mon != "*" && dow == "*" {
        let time = cron_time(hour, min);
        let months = cron_month_list(mon, i18n);
        return if i18n.has("cron.inMonthsAt") {
            i18n.tf("cron.inMonthsAt", &[("months", &months), ("time", &time)])
        } else {
            format!("In {months} at {time}")
        };
    }
    if cron_is_int(min) && cron_is_int(hour) && cron_is_int(dom) && cron_is_int(mon) && dow == "*" {
        let time = cron_time(hour, min);
        let month = cron_month_name(mon, i18n);
        return if i18n.has("cron.yearlyOn") {
            i18n.tf(
                "cron.yearlyOn",
                &[("month", &month), ("dom", dom), ("time", &time)],
            )
        } else {
            format!("Yearly on {month} {dom} at {time}")
        };
    }
    if cron_is_int(min)
        && cron_is_int(hour)
        && (dom.contains(',') || (dom.contains('-') && !dom.to_ascii_uppercase().contains('L')))
        && mon == "*"
        && dow == "*"
    {
        let time = cron_time(hour, min);
        return if i18n.has("cron.monthlyDaysAt") {
            i18n.tf("cron.monthlyDaysAt", &[("days", dom), ("time", &time)])
        } else {
            format!("Monthly on days {dom} at {time}")
        };
    }
    if cron_is_int(min)
        && cron_is_int(hour)
        && dom.len() > 1
        && dom.ends_with(['W', 'w'])
        && !dom.eq_ignore_ascii_case("LW")
        && mon == "*"
        && dow == "*"
    {
        let day = dom.trim_end_matches(['W', 'w']);
        let time = cron_time(hour, min);
        return i18n.tf_or(
            "cron.nearestWeekdayAt",
            &format!("Nearest weekday to day {day} at {time}"),
            &[("day", day), ("time", &time)],
        );
    }
    if cron_is_int(min) && cron_is_int(hour) && cron_is_int(dom) && mon == "*" && dow == "*" {
        let time = cron_time(hour, min);
        return if i18n.has("cron.monthlyAt") {
            i18n.tf("cron.monthlyAt", &[("dom", dom), ("time", &time)])
        } else {
            format!("Monthly on day {dom} at {time}")
        };
    }
    if cron_is_int(min)
        && hour.contains('-')
        && cron_is_int(hour.split('-').next().unwrap_or(""))
        && cron_is_int(hour.split('-').nth(1).unwrap_or(""))
        && !hour.contains(',')
        && dom == "*"
        && mon == "*"
        && dow == "*"
    {
        let start = cron_time(hour.split('-').next().unwrap_or(hour), min);
        let end = cron_time(hour.split('-').nth(1).unwrap_or(hour), min);
        return if i18n.has("cron.dailyBetween") {
            i18n.tf("cron.dailyBetween", &[("start", &start), ("end", &end)])
        } else {
            format!("Daily between {start} and {end}")
        };
    }
    if min == "0" && hour == "0" && dom.starts_with("*/") && mon == "*" && dow == "*" {
        let n = dom.trim_start_matches("*/");
        return if i18n.has("cron.everyDays") {
            i18n.tf("cron.everyDays", &[("n", n)])
        } else {
            format!("Every {n} days")
        };
    }
    if cron_is_int(min) && cron_is_int(hour) && dom.starts_with("*/") && mon == "*" && dow == "*" {
        let n = dom.trim_start_matches("*/");
        let time = cron_time(hour, min);
        return i18n.tf_or(
            "cron.everyNDaysAt",
            &format!("Every {n} days at {time}"),
            &[("n", n), ("time", &time)],
        );
    }
    if cron_is_int(min) && cron_is_int(hour) && dom == "*" && mon == "*" && cron_is_int(dow) {
        let time = cron_time(hour, min);
        let day = cron_day_name(dow, i18n);
        if i18n.has("cron.weeklyOnAt") {
            return i18n.tf("cron.weeklyOnAt", &[("day", &day), ("time", &time)]);
        }
        return if i18n.has("cron.weeklyAt") {
            i18n.tf("cron.weeklyAt", &[("dow", dow), ("time", &time)])
        } else {
            format!("Weekly on {day} at {time}")
        };
    }
    if cron_is_int(min) && cron_is_int(hour) && dom == "*" && mon == "*" && dow.contains(',') {
        let time = cron_time(hour, min);
        let days = cron_day_list(dow, i18n);
        if i18n.has("cron.weeklyOnDaysAt") {
            return i18n.tf("cron.weeklyOnDaysAt", &[("days", &days), ("time", &time)]);
        }
        return if i18n.has("cron.weeklyDaysAt") {
            i18n.tf("cron.weeklyDaysAt", &[("days", &days), ("time", &time)])
        } else {
            format!("Weekly on {days} at {time}")
        };
    }
    if cron_is_int(min)
        && cron_is_int(hour)
        && dom == "*"
        && mon == "*"
        && dow.contains('-')
        && !dow.contains('#')
    {
        let time = cron_time(hour, min);
        let days = cron_format_named_field(dow, |d| cron_day_name(d, i18n));
        if i18n.has("cron.weeklyOnDaysAt") {
            return i18n.tf("cron.weeklyOnDaysAt", &[("days", &days), ("time", &time)]);
        }
        return format!("Weekly on {days} at {time}");
    }
    describe_cron_generic(min, hour, dom, mon, dow, i18n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_flags_are_blocked() {
        assert!(is_reserved_flag("--rc-addr"));
        assert!(is_reserved_flag("--rc-addr=127.0.0.1:1"));
        assert!(is_reserved_flag("rcd"));
        assert!(is_reserved_flag("--rc-serve"));
        assert!(!is_reserved_flag("--vfs-cache-mode"));
        assert!(!is_reserved_flag("--transfers"));
    }

    #[test]
    fn rcd_args_enable_serve() {
        let args = rcd_base_args(5572);
        assert_eq!(args[0], "rcd");
        assert!(args.iter().any(|a| a == "--rc-addr=127.0.0.1:5572"));
        assert!(args.iter().any(|a| a == "--rc-no-auth"));
        assert!(args.iter().any(|a| a == "--rc-serve"));
        assert!(args.iter().any(|a| a == "--rc-web-gui=false"));
    }

    #[test]
    fn cron_validation() {
        assert!(validate_cron("*/5 * * * *").is_ok());
        assert!(validate_cron("").is_err());
        assert!(validate_cron("not a cron").is_err());
        assert_eq!(describe_cron("*/5 * * * *"), "Every 5 minutes");
        assert_eq!(describe_cron("0 */2 * * *"), "Every 2 hours");
        assert_eq!(describe_cron("30 8 * * *"), "Daily at 8:30");
        assert_eq!(describe_cron("0 9 * * 1-5"), "Weekdays at 9:00");
        assert_eq!(describe_cron("0 9 * * 0,6"), "Weekends at 9:00");
        assert_eq!(describe_cron("0 0 1 * *"), "Monthly on day 1 at 0:00");
        assert_eq!(describe_cron("30 * * * *"), "Hourly at minute 30");
        assert_eq!(describe_cron("0,30 * * * *"), "Hourly at minutes 0,30");
        assert_eq!(describe_cron("0 9,17 * * *"), "At 9:00 and 17:00");
        assert_eq!(describe_cron("0 9,12,17 * * *"), "At 9:00, 12:00 and 17:00");
        assert_eq!(
            describe_cron("0 9-17/2 * * *"),
            "Every 2 hours between 9:00 and 17:00"
        );
        assert_eq!(
            describe_cron("*/30 9,12 * * *"),
            "Every 30 minutes at 9:00 and 12:00"
        );
        assert_eq!(
            describe_cron("15,45 9-17 * * *"),
            "At minutes 15,45 between 9:00 and 17:00"
        );
        assert_eq!(describe_cron("0 9-17,20 * * *"), "At 9:00-17:00 and 20:00");
        assert_eq!(
            describe_cron("0 9 * * 1,3,5"),
            "Weekly on Monday, Wednesday, Friday at 9:00"
        );
        assert_eq!(describe_cron("0 9 * * 1"), "Weekly on Monday at 9:00");
        assert_eq!(describe_cron("0 9 * * MON"), "Weekly on Monday at 9:00");
        assert_eq!(describe_cron("0 9 * * MON-FRI"), "Weekdays at 9:00");
        assert_eq!(
            describe_cron("0 9-17 * * *"),
            "Daily between 9:00 and 17:00"
        );
        assert_eq!(describe_cron("0 0 */3 * *"), "Every 3 days");
        assert_eq!(
            describe_cron("*/15 9-17 * * *"),
            "Every 15 minutes between 9:00 and 17:00"
        );
        assert_eq!(describe_cron("30 */2 * * *"), "Every 2 hours at minute 30");
        assert_eq!(
            describe_cron("0 9 1,15 * *"),
            "Monthly on days 1,15 at 9:00"
        );
        assert_eq!(describe_cron("0 0 1 1 *"), "Yearly on January 1 at 0:00");
        assert_eq!(describe_cron("@hourly"), "Hourly at minute 0");
        assert_eq!(describe_cron("@daily"), "Daily at 0:00");
        assert_eq!(describe_cron("@weekly"), "Weekly on Sunday at 0:00");
        assert_eq!(describe_cron("@monthly"), "Monthly on day 1 at 0:00");
        assert_eq!(describe_cron("@yearly"), "Yearly on January 1 at 0:00");
        assert_eq!(describe_cron("@reboot"), "At reboot");
        assert_eq!(describe_cron("0 0 9 * * *"), "Daily at 9:00");
        assert_eq!(describe_cron("0 9 * * * UTC"), "Daily at 9:00");
        assert_eq!(describe_cron("0 0 L * *"), "Last day of the month at 0:00");
        assert_eq!(
            describe_cron("0 0 L-1 * *"),
            "Last day of the month minus 1 at 0:00"
        );
        assert_eq!(
            describe_cron("0 9 LW * *"),
            "Last weekday of the month at 9:00"
        );
        assert_eq!(describe_cron("0 9 * * 5L"), "Last Friday at 9:00");
        assert_eq!(describe_cron("0 9 * * 1#2"), "2nd Monday at 9:00");
        assert_eq!(describe_cron("0 9 * * MON#2"), "2nd Monday at 9:00");
        assert_eq!(
            describe_cron("0 0 1 1,6 *"),
            "In January, June on day 1 at 0:00"
        );
        assert_eq!(describe_cron("0 0 * 1,6 *"), "In January, June at 0:00");
        let i18n = crate::i18n::I18n::default();
        assert_eq!(describe_cron_i18n("*/5 * * * *", &i18n), "Every 5 minutes");
        assert_eq!(describe_cron_i18n("* * * * *", &i18n), "Every minute");
        assert_eq!(
            describe_cron_i18n("0 9 * * 1", &i18n),
            "Weekly on Monday at 9:00"
        );
        assert_eq!(
            describe_cron("0 9 * * 1-3"),
            "Weekly on Monday-Wednesday at 9:00"
        );
        assert_eq!(describe_cron("0 9 * MAR *"), "In March at 9:00");
        assert_eq!(
            describe_cron("0 9 1-15 * *"),
            "Monthly on days 1-15 at 9:00"
        );
        assert_eq!(
            describe_cron("0 9,18 * * 1-5"),
            "Weekdays at 9:00 and 18:00"
        );
        assert_eq!(describe_cron("0 9 */2 * *"), "Every 2 days at 9:00");
        assert_eq!(
            describe_cron("0 9 15W * *"),
            "Nearest weekday to day 15 at 9:00"
        );
        assert_eq!(describe_cron("0 9 15 * 1"), "At 9:00, on day 15, on Monday");
        let tr = crate::i18n::I18n::load("tr-TR");
        assert_eq!(describe_cron_i18n("*/5 * * * *", &tr), "Her 5 dakikada");
        let mixed = describe_cron_i18n("0 9 15 * 1", &tr);
        assert_ne!(mixed, "0 9 15 * 1");
        assert!(mixed.contains("Pazartesi") || mixed.contains("15"));
    }

    #[test]
    fn pick_port_is_nonzero() {
        if let Some(port) = pick_free_port() {
            assert!(port > 0);
        }
    }
}
