//! Blocking SMTP send for email alert actions (AUTH LOGIN, STARTTLS, implicit TLS).

use base64::Engine;
use native_tls::TlsConnector;
use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub fn send_alert_email(config: &Value, title: &str, body: &str) -> Result<(), String> {
    let host = config
        .get("smtp_server")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim();
    if host.is_empty() {
        return Err("smtp_server is required".into());
    }
    let port = config
        .get("smtp_port")
        .and_then(|x| x.as_u64())
        .unwrap_or(25) as u16;
    let from = first_str(config, &["from", "username"]).unwrap_or_default();
    let to = first_str(config, &["to"]).unwrap_or_default();
    if to.is_empty() {
        return Err("to address is required".into());
    }
    let user = first_str(config, &["username"]).unwrap_or_default();
    let pass = first_str(config, &["password"]).unwrap_or_default();
    let subject = config
        .get("subject_template")
        .and_then(|x| x.as_str())
        .unwrap_or("{{title}}")
        .replace("{{title}}", title);
    let mode = tls_mode(config, port);
    send_plain(host, port, mode, &user, &pass, &from, &to, &subject, body)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpTls {
    None,
    StartTls,
    Implicit,
}

pub fn tls_mode(config: &Value, port: u16) -> SmtpTls {
    if config
        .get("use_tls")
        .and_then(|x| x.as_bool())
        .unwrap_or(false)
        || port == 465
    {
        return SmtpTls::Implicit;
    }
    if config
        .get("starttls")
        .and_then(|x| x.as_bool())
        .unwrap_or(false)
        || port == 587
    {
        return SmtpTls::StartTls;
    }
    SmtpTls::None
}

#[allow(clippy::too_many_arguments)]
pub fn send_plain(
    host: &str,
    port: u16,
    mode: SmtpTls,
    user: &str,
    pass: &str,
    from: &str,
    to: &str,
    subject: &str,
    body: &str,
) -> Result<(), String> {
    let stream = connect(host, port)?;
    match mode {
        SmtpTls::Implicit => {
            let mut tls = wrap_tls(stream, host)?;
            smtp_session(&mut tls, host, user, pass, from, to, subject, body)
        }
        SmtpTls::StartTls => {
            let mut plain = stream;
            expect(&mut plain, b"220")?;
            write_line(&mut plain, &format!("EHLO {host}"))?;
            expect(&mut plain, b"250")?;
            write_line(&mut plain, "STARTTLS")?;
            expect(&mut plain, b"220")?;
            let mut tls = wrap_tls(plain, host)?;
            smtp_after_hello(&mut tls, host, user, pass, from, to, subject, body)
        }
        SmtpTls::None => {
            let mut plain = stream;
            smtp_session(&mut plain, host, user, pass, from, to, subject, body)
        }
    }
}

fn connect(host: &str, port: u16) -> Result<TcpStream, String> {
    let stream = TcpStream::connect((host, port)).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(15))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(15))).ok();
    Ok(stream)
}

fn wrap_tls(stream: TcpStream, host: &str) -> Result<native_tls::TlsStream<TcpStream>, String> {
    let connector = TlsConnector::builder().build().map_err(|e| e.to_string())?;
    connector.connect(host, stream).map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)]
fn smtp_session<S: Read + Write>(
    stream: &mut S,
    host: &str,
    user: &str,
    pass: &str,
    from: &str,
    to: &str,
    subject: &str,
    body: &str,
) -> Result<(), String> {
    expect(stream, b"220")?;
    smtp_after_hello(stream, host, user, pass, from, to, subject, body)
}

#[allow(clippy::too_many_arguments)]
fn smtp_after_hello<S: Read + Write>(
    stream: &mut S,
    host: &str,
    user: &str,
    pass: &str,
    from: &str,
    to: &str,
    subject: &str,
    body: &str,
) -> Result<(), String> {
    write_line(stream, &format!("EHLO {host}"))?;
    expect(stream, b"250")?;
    if !user.is_empty() {
        write_line(stream, "AUTH LOGIN")?;
        expect(stream, b"334")?;
        write_line(stream, &b64(user))?;
        expect(stream, b"334")?;
        write_line(stream, &b64(pass))?;
        expect(stream, b"235")?;
    }
    let mail_from = if from.is_empty() { user } else { from };
    write_line(stream, &format!("MAIL FROM:<{mail_from}>"))?;
    expect(stream, b"250")?;
    write_line(stream, &format!("RCPT TO:<{to}>"))?;
    expect(stream, b"250")?;
    write_line(stream, "DATA")?;
    expect(stream, b"354")?;
    let message = format!(
        "From: {mail_from}\r\nTo: {to}\r\nSubject: {subject}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}\r\n."
    );
    write_line(stream, &message)?;
    expect(stream, b"250")?;
    write_line(stream, "QUIT")?;
    Ok(())
}

fn first_str<'a>(config: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| config.get(*k).and_then(|x| x.as_str()))
        .filter(|s| !s.is_empty())
}

fn b64(value: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(value.as_bytes())
}

fn write_line<S: Write>(stream: &mut S, line: &str) -> Result<(), String> {
    stream
        .write_all(line.as_bytes())
        .and_then(|_| stream.write_all(b"\r\n"))
        .map_err(|e| e.to_string())
}

fn expect<S: Read>(stream: &mut S, prefix: &[u8]) -> Result<(), String> {
    let mut buf = [0u8; 1024];
    let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
    if n == 0 {
        return Err("smtp closed the connection".into());
    }
    if !buf[..n].starts_with(prefix) {
        return Err(format!(
            "smtp error: {}",
            String::from_utf8_lossy(&buf[..n]).trim()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_host_and_to() {
        assert!(send_alert_email(&serde_json::json!({}), "t", "b").is_err());
        assert!(send_alert_email(&serde_json::json!({ "smtp_server": "h" }), "t", "b").is_err());
    }

    #[test]
    fn encodes_auth() {
        assert_eq!(b64("user"), "dXNlcg==");
    }

    #[test]
    fn selects_tls_from_port_and_flags() {
        assert_eq!(tls_mode(&serde_json::json!({}), 25), SmtpTls::None);
        assert_eq!(tls_mode(&serde_json::json!({}), 465), SmtpTls::Implicit);
        assert_eq!(tls_mode(&serde_json::json!({}), 587), SmtpTls::StartTls);
        assert_eq!(
            tls_mode(&serde_json::json!({ "use_tls": true }), 25),
            SmtpTls::Implicit
        );
        assert_eq!(
            tls_mode(&serde_json::json!({ "starttls": true }), 25),
            SmtpTls::StartTls
        );
    }
}
