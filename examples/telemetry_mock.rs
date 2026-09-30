//! Minimal War Thunder telemetry mock for manual HUD testing.
//!
//! Run with `cargo run --example telemetry_mock`, then start the HUD normally.

use std::{
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    time::{Duration, Instant},
};

use serde_json::json;

const ADDRESS: &str = "127.0.0.1:8111";
const MAX_REQUEST_BYTES: usize = 8 * 1024;

fn main() -> io::Result<()> {
    let listener = TcpListener::bind(ADDRESS).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("cannot listen on http://{ADDRESS} (is War Thunder or another mock already running?): {error}"),
        )
    })?;
    let started = Instant::now();
    eprintln!("War Thunder telemetry mock listening on http://{ADDRESS}");
    eprintln!("AoA periodically crosses the warning threshold; press Ctrl+C to stop.");

    for connection in listener.incoming() {
        match connection {
            Ok(mut stream) => {
                if let Err(error) = handle_connection(&mut stream, started.elapsed()) {
                    eprintln!("request failed: {error}");
                }
            }
            Err(error) => eprintln!("accept failed: {error}"),
        }
    }
    Ok(())
}

fn handle_connection(stream: &mut TcpStream, elapsed: Duration) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;

    let mut request = [0_u8; MAX_REQUEST_BYTES];
    let length = stream.read(&mut request)?;
    if length == 0 {
        return Ok(());
    }
    let request = String::from_utf8_lossy(&request[..length]);
    let Some(first_line) = request.lines().next() else {
        return write_response(stream, 400, "text/plain; charset=utf-8", "bad request\n");
    };
    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts
        .next()
        .unwrap_or_default()
        .split('?')
        .next()
        .unwrap_or_default();
    if method != "GET" {
        return write_response(
            stream,
            405,
            "text/plain; charset=utf-8",
            "method not allowed\n",
        );
    }

    match response_body(path, elapsed) {
        Some(body) => write_response(stream, 200, "application/json", &body),
        None => write_response(stream, 404, "text/plain; charset=utf-8", "not found\n"),
    }
}

fn response_body(path: &str, elapsed: Duration) -> Option<String> {
    let seconds = elapsed.as_secs_f64();
    let turn = (seconds * 0.8).sin();
    let climb = (seconds * 0.25).cos();
    let ias = 430.0 + 90.0 * (seconds * 0.17).sin();

    match path {
        "/state" => Some(
            json!({
                "valid": true,
                "IAS, km/h": ias,
                "TAS, km/h": ias + 45.0,
                "AoA, deg": 21.0 * turn,
                "Ny": 1.0 + 4.5 * turn.abs(),
                "mock": true
            })
            .to_string(),
        ),
        "/indicators" => Some(
            json!({
                "valid": true,
                "H, m": 1500.0 + 250.0 * (seconds * 0.25).sin(),
                "Vy, m/s": 62.5 * climb,
                "type": "Mock Aircraft",
                "mock": true
            })
            .to_string(),
        ),
        "/map_info.json" => Some(json!({"valid": true, "mock": true}).to_string()),
        "/" => Some(json!({"service": "wt-overlay telemetry mock"}).to_string()),
        _ => None,
    }
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_both_game_endpoints_with_expected_fields() {
        let state: serde_json::Value =
            serde_json::from_str(&response_body("/state", Duration::from_secs(2)).unwrap())
                .unwrap();
        let indicators: serde_json::Value =
            serde_json::from_str(&response_body("/indicators", Duration::from_secs(2)).unwrap())
                .unwrap();
        let map_info: serde_json::Value =
            serde_json::from_str(&response_body("/map_info.json", Duration::ZERO).unwrap())
                .unwrap();

        assert_eq!(state["valid"], true);
        assert!(state["IAS, km/h"].is_number());
        assert!(state["AoA, deg"].is_number());
        assert_eq!(indicators["valid"], true);
        assert!(indicators["H, m"].is_number());
        assert!(indicators["Vy, m/s"].is_number());
        assert_eq!(map_info["valid"], true);
        assert!(response_body("/unknown", Duration::ZERO).is_none());
    }
}
