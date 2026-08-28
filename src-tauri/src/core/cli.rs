//! CLI Arguments for `RClone` Manager
//!
//! This module contains the command-line argument definitions for
//! both Desktop and Headless (web-server) builds.

use clap::{Args, Parser};
use std::path::PathBuf;

/// `RClone` Manager CLI Arguments
#[derive(Parser, Debug, Clone)]
#[command(name = "rclone-manager")]
#[cfg_attr(feature = "web-server", command(about = "RClone Manager - Headless Web UI for Rclone", long_about = None))]
#[cfg_attr(not(feature = "web-server"), command(about = "RClone Manager - Desktop GUI for Rclone", long_about = None))]
pub struct CliArgs {
    #[command(flatten)]
    pub general: GeneralArgs,

    #[cfg(feature = "web-server")]
    #[command(flatten)]
    pub headless: HeadlessArgs,
}

/// General arguments available in all build modes
#[derive(Args, Debug, Clone)]
pub struct GeneralArgs {
    /// Path to data directory (overrides default/env)
    #[arg(long, env = "RCLONE_MANAGER_DATA_DIR")]
    pub data_dir: Option<PathBuf>,

    /// Path to cache directory (overrides default/env)
    #[arg(long, env = "RCLONE_MANAGER_CACHE_DIR")]
    pub cache_dir: Option<PathBuf>,

    /// Path to logs directory (overrides default/env)
    #[arg(long, env = "RCLONE_MANAGER_LOG_DIR")]
    pub logs_dir: Option<PathBuf>,

    /// Start in system tray
    #[cfg(feature = "tray")]
    #[arg(long)]
    pub tray: bool,

    /// Send files/folders to a remote destination (Windows SendTo integration)
    #[arg(long)]
    pub send_to_remote: Option<String>,

    /// Destination path on the remote (Windows SendTo integration)
    #[arg(long)]
    pub send_to_path: Option<String>,

    /// Source paths to send (Windows SendTo integration)
    pub send_to_sources: Vec<PathBuf>,
}

/// Headless web server specific arguments
#[cfg(feature = "web-server")]
#[derive(Args, Debug, Clone)]
pub struct HeadlessArgs {
    /// Host address to bind to
    #[arg(
        short = 'H',
        long,
        env = "RCLONE_MANAGER_HOST",
        default_value = "0.0.0.0"
    )]
    pub host: String,

    /// Port to listen on
    #[arg(short, long, env = "RCLONE_MANAGER_PORT", default_value = "8080")]
    pub port: u16,

    /// Username for Basic Authentication (optional)
    #[arg(short, long, env = "RCLONE_MANAGER_USER")]
    pub user: Option<String>,

    /// Password for Basic Authentication (required if user is set)
    #[arg(long, env = "RCLONE_MANAGER_PASS")]
    pub pass: Option<String>,

    /// Path to TLS certificate file (optional)
    #[arg(long, env = "RCLONE_MANAGER_TLS_CERT")]
    pub tls_cert: Option<PathBuf>,

    /// Path to TLS key file (optional)
    #[arg(long, env = "RCLONE_MANAGER_TLS_KEY")]
    pub tls_key: Option<PathBuf>,

    /// Serve without authentication on a non-loopback address.
    ///
    /// Without credentials the whole API — including the command bridge, file
    /// streaming and uploads — is open to anyone who can reach the port, so
    /// binding a non-loopback address without `--user`/`--pass` is refused
    /// unless this is set explicitly.
    #[arg(long, env = "RCLONE_MANAGER_INSECURE_NO_AUTH", default_value_t = false)]
    pub insecure_no_auth: bool,
}

/// Whether a bind address only accepts connections from this machine.
///
/// `0.0.0.0` / `::` (and any routable address) reach the network; an empty host
/// is treated as routable so an unset value fails closed.
#[cfg(feature = "web-server")]
pub fn host_is_loopback(host: &str) -> bool {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

impl CliArgs {
    /// Validates the CLI arguments for logical consistency.
    pub fn validate(&self) -> Result<(), String> {
        #[cfg(feature = "web-server")]
        {
            // Auth validation: both user and pass must be present or both absent
            match (&self.headless.user, &self.headless.pass) {
                (Some(_), None) => {
                    return Err(
                        "Password is required when username is set (--pass or RCLONE_MANAGER_PASS)"
                            .into(),
                    );
                }
                (None, Some(_)) => {
                    return Err(
                        "Username is required when password is set (--user or RCLONE_MANAGER_USER)"
                            .into(),
                    );
                }
                _ => {}
            }

            // Refuse to expose an unauthenticated API beyond loopback.
            if self.headless.user.is_none()
                && !self.headless.insecure_no_auth
                && !host_is_loopback(&self.headless.host)
            {
                return Err(format!(
                    "Refusing to bind {} without authentication: the API (command bridge, \
                     file streaming, uploads) would be open to anyone who can reach port {}. \
                     Set --user/--pass (RCLONE_MANAGER_USER / RCLONE_MANAGER_PASS), bind \
                     127.0.0.1 instead, or pass --insecure-no-auth to override.",
                    self.headless.host, self.headless.port
                ));
            }

            // TLS validation: both cert and key must be present or both absent
            match (&self.headless.tls_cert, &self.headless.tls_key) {
                (Some(_), None) => {
                    return Err("TLS key is required when certificate is set (--tls-key or RCLONE_MANAGER_TLS_KEY)".into());
                }
                (None, Some(_)) => {
                    return Err("TLS certificate is required when key is set (--tls-cert or RCLONE_MANAGER_TLS_CERT)".into());
                }
                _ => {}
            }
        }

        // SendTo validation
        if self.general.send_to_path.is_some() && self.general.send_to_remote.is_none() {
            return Err("Cannot use --send-to-path without specifying a destination remote via --send-to-remote".into());
        }

        if self.general.send_to_remote.is_some() && self.general.send_to_sources.is_empty() {
            return Err(
                "At least one source file or folder must be provided when using --send-to-remote"
                    .into(),
            );
        }

        Ok(())
    }

    /// Returns auth credentials if both user and pass are set
    #[cfg(feature = "web-server")]
    pub fn auth_credentials(&self) -> Option<(String, String)> {
        match (&self.headless.user, &self.headless.pass) {
            (Some(u), Some(p)) => Some((u.clone(), p.clone())),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "web-server")]
    mod bind_safety {
        use super::super::host_is_loopback;

        #[test]
        fn loopback_hosts_are_recognized() {
            assert!(host_is_loopback("127.0.0.1"));
            assert!(host_is_loopback("127.0.0.53"));
            assert!(host_is_loopback("::1"));
            assert!(host_is_loopback("[::1]"));
            assert!(host_is_loopback("localhost"));
            assert!(host_is_loopback("LocalHost"));
            assert!(host_is_loopback(" 127.0.0.1 "));
        }

        #[test]
        fn routable_and_unparseable_hosts_fail_closed() {
            assert!(!host_is_loopback("0.0.0.0"));
            assert!(!host_is_loopback("::"));
            assert!(!host_is_loopback("192.168.1.10"));
            assert!(!host_is_loopback("example.com"));
            assert!(!host_is_loopback(""));
        }
    }

    use super::*;
    use clap::Parser;

    #[test]
    fn test_general_paths() {
        let args = CliArgs::parse_from([
            "rclone-manager",
            "--data-dir",
            "/data",
            "--cache-dir",
            "/cache",
        ]);
        assert_eq!(args.general.data_dir, Some(PathBuf::from("/data")));
        assert_eq!(args.general.cache_dir, Some(PathBuf::from("/cache")));
        #[cfg(feature = "tray")]
        assert!(!args.general.tray);
    }

    #[cfg(feature = "tray")]
    #[test]
    fn test_tray_flag() {
        let args = CliArgs::parse_from(["rclone-manager", "--tray"]);
        assert!(args.general.tray);
    }

    #[test]
    fn test_send_to_args() {
        let args = CliArgs::parse_from([
            "rclone-manager",
            "--send-to-remote",
            "Dropbox:",
            "--send-to-path",
            "/Photos",
            "C:\\file1.txt",
            "C:\\file2.txt",
        ]);
        assert_eq!(args.general.send_to_remote, Some("Dropbox:".to_string()));
        assert_eq!(args.general.send_to_path, Some("/Photos".to_string()));
        assert_eq!(
            args.general.send_to_sources,
            vec![
                PathBuf::from("C:\\file1.txt"),
                PathBuf::from("C:\\file2.txt")
            ]
        );
    }

    #[cfg(feature = "web-server")]
    #[test]
    fn test_headless_full_args() {
        let args = CliArgs::parse_from([
            "rclone-manager",
            "--host",
            "127.0.0.1",
            "--port",
            "9090",
            "--user",
            "hakan",
            "--pass",
            "secret",
            "--tls-cert",
            "/path/to/cert",
            "--tls-key",
            "/path/to/key",
        ]);
        assert_eq!(args.headless.host, "127.0.0.1");
        assert_eq!(args.headless.port, 9090);
        assert_eq!(args.headless.user, Some("hakan".to_string()));
        assert_eq!(args.headless.pass, Some("secret".to_string()));
        assert_eq!(args.headless.tls_cert, Some(PathBuf::from("/path/to/cert")));
        assert_eq!(args.headless.tls_key, Some(PathBuf::from("/path/to/key")));
        assert!(args.validate().is_ok());
    }

    #[cfg(feature = "web-server")]
    #[test]
    fn test_validate_headless_tls() {
        // Bound to loopback so the non-loopback auth rule is not what decides
        // these cases — this test is about the cert/key pairing.
        let args = CliArgs::parse_from([
            "rclone-manager",
            "--host",
            "127.0.0.1",
            "--tls-cert",
            "/path/to/cert",
        ]);
        assert!(args.validate().is_err());

        let args = CliArgs::parse_from([
            "rclone-manager",
            "--host",
            "127.0.0.1",
            "--tls-key",
            "/path/to/key",
        ]);
        assert!(args.validate().is_err());

        let args = CliArgs::parse_from([
            "rclone-manager",
            "--host",
            "127.0.0.1",
            "--tls-cert",
            "/path/to/cert",
            "--tls-key",
            "/path/to/key",
        ]);
        assert!(args.validate().is_ok());
    }

    #[cfg(feature = "web-server")]
    #[test]
    fn test_validate_refuses_unauthenticated_non_loopback_bind() {
        // The shipped default is 0.0.0.0:8080 with no credentials, which would
        // publish the command bridge, file streaming and uploads to the network.
        let args = CliArgs::parse_from(["rclone-manager"]);
        let err = args.validate().expect_err("default bind must be refused");
        assert!(err.contains("without authentication"), "{err}");

        // Credentials make it fine.
        let args = CliArgs::parse_from(["rclone-manager", "--user", "admin", "--pass", "s3cr3t"]);
        assert!(args.validate().is_ok());

        // So does staying on loopback.
        let args = CliArgs::parse_from(["rclone-manager", "--host", "127.0.0.1"]);
        assert!(args.validate().is_ok());

        // And so does opting in explicitly.
        let args = CliArgs::parse_from(["rclone-manager", "--insecure-no-auth"]);
        assert!(args.validate().is_ok());

        // A username without a password is still rejected first.
        let args = CliArgs::parse_from(["rclone-manager", "--user", "admin"]);
        assert!(args.validate().is_err());
    }

    #[cfg(feature = "web-server")]
    #[test]
    fn insecure_no_auth_can_be_set_from_the_environment() {
        // docker-compose documents RCLONE_MANAGER_INSECURE_NO_AUTH, so the env
        // form has to work, not just the flag.
        // SAFETY: single-threaded test binary section; the var is removed again
        // before returning.
        unsafe { std::env::set_var("RCLONE_MANAGER_INSECURE_NO_AUTH", "true") };
        let args = CliArgs::parse_from(["rclone-manager"]);
        let opted_in = args.headless.insecure_no_auth;
        let validated = args.validate();
        unsafe { std::env::remove_var("RCLONE_MANAGER_INSECURE_NO_AUTH") };

        assert!(opted_in, "env var must set the opt-out");
        assert!(validated.is_ok(), "{validated:?}");
    }

    /// `--host` only exists on the web-server build. Pin it to loopback there so
    /// the non-loopback auth rule does not decide unrelated validation tests.
    fn loopback_args() -> Vec<&'static str> {
        #[cfg(feature = "web-server")]
        {
            vec!["--host", "127.0.0.1"]
        }
        #[cfg(not(feature = "web-server"))]
        {
            Vec::new()
        }
    }

    fn args_from(extra: &[&str]) -> CliArgs {
        let mut argv = vec!["rclone-manager"];
        argv.extend(loopback_args());
        argv.extend_from_slice(extra);
        CliArgs::parse_from(argv)
    }

    #[test]
    fn test_validate_send_to() {
        // Failing: path without remote
        assert!(
            args_from(&["--send-to-path", "/Photos"])
                .validate()
                .is_err()
        );

        // Failing: remote without sources
        assert!(
            args_from(&["--send-to-remote", "Dropbox:"])
                .validate()
                .is_err()
        );

        // Passing: remote with sources
        assert!(
            args_from(&["--send-to-remote", "Dropbox:", "/file.txt"])
                .validate()
                .is_ok()
        );

        // Passing: remote, path, and sources
        assert!(
            args_from(&[
                "--send-to-remote",
                "Dropbox:",
                "--send-to-path",
                "/Photos",
                "/file.txt",
            ])
            .validate()
            .is_ok()
        );
    }
}
