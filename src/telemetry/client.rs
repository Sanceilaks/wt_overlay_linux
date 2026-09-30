use std::fmt;
use std::time::Duration;

pub trait TelemetryClient: Send + 'static {
    fn get(&self, path: &str) -> Result<String, ClientError>;
}

#[derive(Clone)]
pub struct UreqTelemetryClient {
    base_url: String,
    agent: ureq::Agent,
}

impl UreqTelemetryClient {
    pub fn new(base_url: &str, request_timeout: Duration) -> Result<Self, ClientError> {
        let base_url = base_url.trim_end_matches('/');
        if base_url != "http://127.0.0.1:8111" {
            return Err(ClientError::new(
                "telemetry URL must be exactly http://127.0.0.1:8111",
            ));
        }

        let config = ureq::Agent::config_builder()
            .timeout_global(Some(request_timeout))
            .build();
        Ok(Self {
            base_url: base_url.to_owned(),
            agent: config.into(),
        })
    }
}

impl TelemetryClient for UreqTelemetryClient {
    fn get(&self, path: &str) -> Result<String, ClientError> {
        debug_assert!(path.starts_with('/'));
        let mut response = self
            .agent
            .get(format!("{}{path}", self.base_url))
            .call()
            .map_err(|error| ClientError::new(error.to_string()))?;
        response
            .body_mut()
            .read_to_string()
            .map_err(|error| ClientError::new(error.to_string()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClientError {
    message: String,
}

impl ClientError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ClientError {}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    #[test]
    fn blocking_client_reads_from_a_loopback_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let request_len = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..request_len]);
            assert!(request.starts_with("GET /state HTTP/1.1"));
            let body = r#"{"valid":true}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });

        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(1)))
            .build();
        let client = UreqTelemetryClient {
            base_url: format!("http://{address}"),
            agent: config.into(),
        };
        assert_eq!(client.get("/state").unwrap(), r#"{"valid":true}"#);
        server.join().unwrap();
    }

    #[test]
    fn production_constructor_rejects_non_game_urls() {
        let error = UreqTelemetryClient::new("http://example.com:8111", Duration::from_millis(100))
            .err()
            .unwrap();
        assert!(error.to_string().contains("127.0.0.1"));
    }
}
