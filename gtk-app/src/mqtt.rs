//! Minimal MQTT 3.1.1 QoS 0 publisher for alert actions.

use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub fn publish_alert(config: &Value, body: &str) -> Result<(), String> {
    let url = config
        .get("broker_url")
        .or_else(|| config.get("url"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let (host, port, tls) = parse_broker(url)?;
    let use_tls = tls
        || config
            .get("use_tls")
            .and_then(|x| x.as_bool())
            .unwrap_or(false)
        || port == 8883;
    let topic = config
        .get("topic")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("rclone-manager/alerts");
    let user = config
        .get("username")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    let pass = config
        .get("password")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    publish(&host, port, use_tls, user, pass, topic, body)
}

pub fn parse_broker(url: &str) -> Result<(String, u16, bool), String> {
    let trimmed = url.trim();
    let tls = trimmed.starts_with("mqtts://") || trimmed.starts_with("ssl://");
    let trimmed = trimmed
        .trim_start_matches("mqtts://")
        .trim_start_matches("ssl://")
        .trim_start_matches("mqtt://")
        .trim_start_matches("tcp://");
    if trimmed.is_empty() {
        return Err("broker_url is required".into());
    }
    if let Some((host, port)) = trimmed.rsplit_once(':') {
        if !host.contains('/') {
            let port = port
                .split('/')
                .next()
                .unwrap_or(port)
                .parse::<u16>()
                .unwrap_or(if tls { 8883 } else { 1883 });
            return Ok((host.to_string(), port, tls || port == 8883));
        }
    }
    Ok((
        trimmed.split('/').next().unwrap_or(trimmed).to_string(),
        if tls { 8883 } else { 1883 },
        tls,
    ))
}

pub fn publish(
    host: &str,
    port: u16,
    use_tls: bool,
    user: &str,
    pass: &str,
    topic: &str,
    payload: &str,
) -> Result<(), String> {
    let stream = TcpStream::connect((host, port)).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(8))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(8))).ok();
    if use_tls {
        let connector = native_tls::TlsConnector::new().map_err(|e| e.to_string())?;
        let mut tls = connector.connect(host, stream).map_err(|e| e.to_string())?;
        mqtt_session(&mut tls, user, pass, topic, payload)
    } else {
        let mut plain = stream;
        mqtt_session(&mut plain, user, pass, topic, payload)
    }
}

fn mqtt_session<S: Read + Write>(
    stream: &mut S,
    user: &str,
    pass: &str,
    topic: &str,
    payload: &str,
) -> Result<(), String> {
    stream
        .write_all(&connect_packet(user, pass))
        .map_err(|e| e.to_string())?;
    let mut ack = [0u8; 4];
    stream.read_exact(&mut ack).map_err(|e| e.to_string())?;
    if ack[0] != 0x20 || ack[3] != 0 {
        return Err(format!("mqtt connack rejected: {ack:?}"));
    }
    stream
        .write_all(&publish_packet(topic, payload.as_bytes()))
        .map_err(|e| e.to_string())?;
    stream.write_all(&[0xe0, 0x00]).map_err(|e| e.to_string())?;
    Ok(())
}

fn connect_packet(user: &str, pass: &str) -> Vec<u8> {
    let mut vh = Vec::new();
    encode_string(&mut vh, "MQTT");
    vh.push(4);
    let mut flags = 0x02;
    if !user.is_empty() {
        flags |= 0x80;
    }
    if !pass.is_empty() {
        flags |= 0x40;
    }
    vh.push(flags);
    vh.extend_from_slice(&60u16.to_be_bytes());
    encode_string(&mut vh, "rclone-manager");
    if !user.is_empty() {
        encode_string(&mut vh, user);
    }
    if !pass.is_empty() {
        encode_string(&mut vh, pass);
    }
    packet(0x10, &vh)
}

fn publish_packet(topic: &str, payload: &[u8]) -> Vec<u8> {
    let mut vh = Vec::new();
    encode_string(&mut vh, topic);
    vh.extend_from_slice(payload);
    packet(0x30, &vh)
}

fn encode_string(out: &mut Vec<u8>, value: &str) {
    let bytes = value.as_bytes();
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(bytes);
}

fn packet(header: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![header];
    remaining_length(&mut out, payload.len());
    out.extend_from_slice(payload);
    out
}

fn remaining_length(out: &mut Vec<u8>, mut len: usize) {
    loop {
        let mut byte = (len % 128) as u8;
        len /= 128;
        if len > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if len == 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_broker_urls() {
        assert_eq!(
            parse_broker("mqtt://localhost:1883").unwrap(),
            ("localhost".into(), 1883, false)
        );
        assert_eq!(
            parse_broker("127.0.0.1").unwrap(),
            ("127.0.0.1".into(), 1883, false)
        );
        assert_eq!(
            parse_broker("mqtts://broker.example:8883").unwrap(),
            ("broker.example".into(), 8883, true)
        );
        assert!(parse_broker("").is_err());
    }

    #[test]
    fn remaining_length_encodes() {
        let mut out = Vec::new();
        remaining_length(&mut out, 0);
        assert_eq!(out, vec![0]);
        out.clear();
        remaining_length(&mut out, 127);
        assert_eq!(out, vec![127]);
    }

    #[test]
    fn connect_packet_has_mqtt_header() {
        let pkt = connect_packet("", "");
        assert_eq!(pkt[0], 0x10);
        assert!(pkt.windows(4).any(|w| w == b"MQTT"));
    }
}
