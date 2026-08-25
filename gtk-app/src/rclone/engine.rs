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

        match cmd.spawn() {
            Ok(child) => {
                engine.child = Some(child);
                let deadline = Instant::now() + Duration::from_secs(8);
                while Instant::now() < deadline {
                    if engine.client.ping() {
                        engine.available = true;
                        engine.version = engine.client.version().unwrap_or_default();
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
    }

    #[test]
    fn pick_port_is_nonzero() {
        if let Some(port) = pick_free_port() {
            assert!(port > 0);
        }
    }
}
