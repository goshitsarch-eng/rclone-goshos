//! Blocking SMTP send for email alert actions.

use base64::Engine;
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
    send_plain(host, port, &user, &pass, &from, &to, &subject, body)
}

#[allow(clippy::too_many_arguments)]
pub fn send_plain(
    host: &str,
    port: u16,
    user: &str,
    pass: &str,
    from: &str,
    to: &str,
    subject: &str,
    body: &str,
) -> Result<(), String> {
    let mut stream = TcpStream::connect((host, port)).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(15))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(15))).ok();
    expect(&mut stream, b"220")?;
    write_line(&mut stream, &format!("EHLO {host}"))?;
    expect(&mut stream, b"250")?;
    if !user.is_empty() {
        write_line(&mut stream, "AUTH LOGIN")?;
        expect(&mut stream, b"334")?;
        write_line(&mut stream, &b64(user))?;
        expect(&mut stream, b"334")?;
        write_line(&mut stream, &b64(pass))?;
        expect(&mut stream, b"235")?;
    }
    let mail_from = if from.is_empty() { user } else { from };
    write_line(&mut stream, &format!("MAIL FROM:<{mail_from}>"))?;
    expect(&mut stream, b"250")?;
    write_line(&mut stream, &format!("RCPT TO:<{to}>"))?;
    expect(&mut stream, b"250")?;
    write_line(&mut stream, "DATA")?;
    expect(&mut stream, b"354")?;
    let message = format!(
        "From: {mail_from}\r\nTo: {to}\r\nSubject: {subject}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}\r\n."
    );
    write_line(&mut stream, &message)?;
    expect(&mut stream, b"250")?;
    write_line(&mut stream, "QUIT")?;
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

fn write_line(stream: &mut TcpStream, line: &str) -> Result<(), String> {
    stream
        .write_all(line.as_bytes())
        .and_then(|_| stream.write_all(b"\r\n"))
        .map_err(|e| e.to_string())
}

fn expect(stream: &mut TcpStream, prefix: &[u8]) -> Result<(), String> {
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
}
