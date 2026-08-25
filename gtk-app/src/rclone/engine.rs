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
        cmd.arg("rcd")
            .arg(format!("--rc-addr=127.0.0.1:{port}"))
            .arg("--rc-no-auth")
            .arg("--rc-web-gui=false")
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        for flag in &settings.core.rclone_additional_flags {
            if is_reserved_flag(flag) {
                continue;
            }
            cmd.arg(flag);
        }
        for env in &settings.core.rclone_env_vars {
            if let Some((k, v)) = env.split_once('=') {
                cmd.env(k, v);
            }
        }
        let password = crate::keyring::resolve_config_password(&settings.core.config_password);
        crate::security::apply_config_password_env(&mut cmd, &password);
        let log_path = crate::settings::AppSettings::config_dir().join("rclone.log");
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
        engine
    }

    pub fn restart(&mut self, settings: &AppSettings) {
        self.shutdown();
        *self = Self::start(settings);
    }

    pub fn shutdown(&mut self) {
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

pub fn describe_cron_i18n(expression: &str, i18n: &crate::i18n::I18n) -> String {
    let parts: Vec<&str> = expression.split_whitespace().collect();
    if parts.len() < 5 {
        return expression.to_string();
    }
    let (min, hour, dom, mon, dow) = (parts[0], parts[1], parts[2], parts[3], parts[4]);
    if min.starts_with("*/") && hour == "*" && dom == "*" && mon == "*" && dow == "*" {
        let n = min.trim_start_matches("*/");
        return if i18n.has("cron.everyMinutes") {
            i18n.tf("cron.everyMinutes", &[("n", n)])
        } else {
            format!("Every {n} minutes")
        };
    }
    if min == "0" && hour.starts_with("*/") && dom == "*" && mon == "*" && dow == "*" {
        let n = hour.trim_start_matches("*/");
        return if i18n.has("cron.everyHours") {
            i18n.tf("cron.everyHours", &[("n", n)])
        } else {
            format!("Every {n} hours")
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
    if cron_is_int(min) && cron_is_int(hour) && dom == "*" && mon == "*" && dow == "*" {
        let time = cron_time(hour, min);
        return if i18n.has("cron.dailyAt") {
            i18n.tf("cron.dailyAt", &[("time", &time)])
        } else {
            format!("Daily at {time}")
        };
    }
    if cron_is_int(min) && cron_is_int(hour) && dom == "*" && mon == "*" && dow == "1-5" {
        let time = cron_time(hour, min);
        return if i18n.has("cron.weekdaysAt") {
            i18n.tf("cron.weekdaysAt", &[("time", &time)])
        } else {
            format!("Weekdays at {time}")
        };
    }
    if cron_is_int(min) && cron_is_int(hour) && dom == "*" && mon == "*" && cron_is_weekend(dow) {
        let time = cron_time(hour, min);
        return if i18n.has("cron.weekendsAt") {
            i18n.tf("cron.weekendsAt", &[("time", &time)])
        } else {
            format!("Weekends at {time}")
        };
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
        return if i18n.has("cron.weeklyDaysAt") {
            i18n.tf("cron.weeklyDaysAt", &[("days", dow), ("time", &time)])
        } else {
            format!("Weekly on days {dow} at {time}")
        };
    }
    expression.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_flags_are_blocked() {
        assert!(is_reserved_flag("--rc-addr"));
        assert!(is_reserved_flag("--rc-addr=127.0.0.1:1"));
        assert!(is_reserved_flag("rcd"));
        assert!(!is_reserved_flag("--vfs-cache-mode"));
        assert!(!is_reserved_flag("--transfers"));
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
        assert_eq!(
            describe_cron("0 9 * * 1,3,5"),
            "Weekly on days 1,3,5 at 9:00"
        );
        assert_eq!(describe_cron("0 9 * * 1"), "Weekly on Monday at 9:00");
        assert_eq!(
            describe_cron("0 9-17 * * *"),
            "Daily between 9:00 and 17:00"
        );
        assert_eq!(describe_cron("0 0 */3 * *"), "Every 3 days");
        let i18n = crate::i18n::I18n::default();
        assert_eq!(describe_cron_i18n("*/5 * * * *", &i18n), "Every 5 minutes");
        assert_eq!(describe_cron_i18n("* * * * *", &i18n), "Every minute");
        assert_eq!(
            describe_cron_i18n("0 9 * * 1", &i18n),
            "Weekly on Monday at 9:00"
        );
    }

    #[test]
    fn pick_port_is_nonzero() {
        if let Some(port) = pick_free_port() {
            assert!(port > 0);
        }
    }
}
