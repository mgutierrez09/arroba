use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::{connect, Message};

use crate::error::DaemonError;

use super::json_rpc::JsonRpcMessage;
use super::notifications::{parse_notification, rpc_error_message};
use super::socket_io::{read_socket_nonblocking, set_socket_timeouts};
use super::{CodexClient, CodexNotification, CodexSocket};

impl CodexClient {
    pub fn connect_initialized(&self) -> Result<CodexSocket, DaemonError> {
        let (mut socket, _) = connect(self.endpoint.as_str())
            .map_err(|error| self.protocol_error("codex_connect", error.to_string()))?;
        set_socket_timeouts(
            &mut socket,
            Some(Duration::from_secs(10)),
            Some(Duration::from_secs(10)),
        )?;
        self.initialize_socket(&mut socket)?;
        Ok(socket)
    }

    pub fn send_request<T: for<'de> Deserialize<'de>>(
        &self,
        socket: &mut CodexSocket,
        next_request_id: &mut u64,
        method: &'static str,
        params: Value,
    ) -> Result<T, DaemonError> {
        self.send_request_buffering_notifications(
            socket,
            next_request_id,
            method,
            params,
            &mut Vec::new(),
        )
    }

    pub fn send_request_buffering_notifications<T: for<'de> Deserialize<'de>>(
        &self,
        socket: &mut CodexSocket,
        next_request_id: &mut u64,
        method: &'static str,
        params: Value,
        buffered_notifications: &mut Vec<CodexNotification>,
    ) -> Result<T, DaemonError> {
        self.send_request_buffering_notifications_with_timeout(
            socket,
            next_request_id,
            method,
            params,
            buffered_notifications,
            codex_request_timeout(method),
        )
    }

    fn send_request_buffering_notifications_with_timeout<T: for<'de> Deserialize<'de>>(
        &self,
        socket: &mut CodexSocket,
        next_request_id: &mut u64,
        method: &'static str,
        params: Value,
        buffered_notifications: &mut Vec<CodexNotification>,
        timeout: Duration,
    ) -> Result<T, DaemonError> {
        let request_id = *next_request_id;
        *next_request_id += 1;
        let payload = json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        });
        socket
            .send(Message::Text(payload.to_string().into()))
            .map_err(|error| self.protocol_error("codex_write", error.to_string()))?;

        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(self.protocol_error(
                    "codex_read",
                    format!("timed out waiting for Codex app-server after {timeout:?}"),
                ));
            }
            let raw = self.read_next_message(socket, remaining)?;
            let message: JsonRpcMessage = serde_json::from_str(&raw)
                .map_err(|error| self.protocol_error("codex_read_parse", error.to_string()))?;
            if self.respond_to_server_request(socket, &message)? {
                continue;
            }
            if message.id.as_ref() == Some(&json!(request_id)) {
                if let Some(error) = rpc_error_message(&message) {
                    return Err(self.protocol_error(method, error));
                }
                let result = message.result.ok_or_else(|| {
                    self.protocol_error(method, "Codex returned no response payload".to_string())
                })?;
                return serde_json::from_value(result)
                    .map_err(|error| self.protocol_error(method, error.to_string()));
            }
            let parsed_notification = parse_notification(message.clone());
            if parsed_notification.is_none()
                && matches!(
                    message.method.as_deref(),
                    Some("turn/started" | "turn/completed")
                )
            {
                log_unparsed_turn_lifecycle(&self.provider_run_id, &message);
            }
            if let Some(notification) = parsed_notification {
                buffered_notifications.push(notification);
            } else if !is_turn_lifecycle_method(message.method.as_deref()) {
                if let Some(message_method) = message.method.as_deref() {
                    crate::logging::debug_with_fields(
                        "daemon.provider.codex",
                        "ignored codex message while awaiting response",
                        json!({
                            "provider_run_id": self.provider_run_id,
                            "awaiting_method": method,
                            "message_method": message_method,
                            "has_id": message.id.is_some(),
                            "params": message.params,
                            "error": message.error,
                        }),
                    );
                }
            }
        }
    }

    pub fn read_notification(
        &self,
        socket: &mut CodexSocket,
        timeout: Duration,
    ) -> Result<Option<CodexNotification>, DaemonError> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(None);
            }
            match read_socket_nonblocking(socket) {
                Ok(message) => {
                    let raw = match message {
                        Message::Text(text) => text.to_string(),
                        Message::Binary(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                        Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
                        Message::Close(_) => {
                            return Ok(Some(CodexNotification::Error {
                                message: "Codex app-server closed the websocket".to_string(),
                            }));
                        }
                    };
                    let message: JsonRpcMessage = serde_json::from_str(&raw).map_err(|error| {
                        self.protocol_error("codex_notification_parse", error.to_string())
                    })?;
                    if self.respond_to_server_request(socket, &message)? {
                        continue;
                    }
                    let notification = parse_notification(message.clone());
                    if notification.is_none()
                        && matches!(
                            message.method.as_deref(),
                            Some("turn/started" | "turn/completed")
                        )
                    {
                        log_unparsed_turn_lifecycle(&self.provider_run_id, &message);
                    }
                    if let Some(notification) = notification {
                        return Ok(Some(notification));
                    }
                    if !is_turn_lifecycle_method(message.method.as_deref()) {
                        if let Some(method) = message.method.as_deref() {
                            crate::logging::debug_with_fields(
                                "daemon.provider.codex",
                                "ignored codex notification",
                                json!({
                                    "provider_run_id": self.provider_run_id,
                                    "method": method,
                                    "has_id": message.id.is_some(),
                                    "params": message.params,
                                    "error": message.error,
                                }),
                            );
                        }
                    }
                    continue;
                }
                Err(tokio_tungstenite::tungstenite::Error::Io(error))
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None);
                }
                Err(error) => return Err(self.protocol_error("codex_read", error.to_string())),
            }
        }
    }

    fn initialize_socket(&self, socket: &mut CodexSocket) -> Result<(), DaemonError> {
        let initialize = json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "chariox-kernel",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {
                    "experimentalApi": true,
                },
            },
        });
        socket
            .send(Message::Text(initialize.to_string().into()))
            .map_err(|error| self.protocol_error("codex_initialize", error.to_string()))?;
        let response = self.read_next_message(socket, Duration::from_secs(10))?;
        let message: JsonRpcMessage = serde_json::from_str(&response)
            .map_err(|error| self.protocol_error("codex_initialize_parse", error.to_string()))?;
        if message.result.is_none() {
            return Err(self.protocol_error(
                "codex_initialize",
                rpc_error_message(&message)
                    .unwrap_or_else(|| "Codex returned no initialize result".to_string()),
            ));
        }
        let initialized = json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {},
        });
        socket
            .send(Message::Text(initialized.to_string().into()))
            .map_err(|error| self.protocol_error("codex_initialized", error.to_string()))?;
        Ok(())
    }

    fn read_next_message(
        &self,
        socket: &mut CodexSocket,
        timeout: Duration,
    ) -> Result<String, DaemonError> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(self.protocol_error(
                    "codex_read",
                    format!("timed out waiting for Codex app-server after {timeout:?}"),
                ));
            }
            set_socket_timeouts(socket, Some(remaining), Some(Duration::from_secs(5)))?;
            match socket.read() {
                Ok(Message::Text(text)) => return Ok(text.to_string()),
                Ok(Message::Binary(bytes)) => {
                    return Ok(String::from_utf8_lossy(&bytes).into_owned());
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
                Ok(Message::Close(_)) => {
                    return Err(self.protocol_error(
                        "codex_read",
                        "Codex app-server closed the websocket".to_string(),
                    ));
                }
                Ok(Message::Frame(_)) => continue,
                Err(tokio_tungstenite::tungstenite::Error::Io(error))
                    if codex_read_should_retry(&error) =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(error) => return Err(self.protocol_error("codex_read", error.to_string())),
            }
        }
    }
}

fn is_turn_lifecycle_method(method: Option<&str>) -> bool {
    matches!(method, Some("turn/started" | "turn/completed"))
}

fn log_unparsed_turn_lifecycle(provider_run_id: &str, message: &JsonRpcMessage) {
    crate::logging::warn_with_fields(
        "daemon.provider.codex",
        "discarded unrecognized Codex turn lifecycle notification",
        json!({
            "provider_run_id": provider_run_id,
            "method": message.method,
            "has_id": message.id.is_some(),
            "param_keys": message
                .params
                .as_ref()
                .and_then(Value::as_object)
                .map(|params| params.keys().cloned().collect::<Vec<_>>()),
        }),
    );
}

fn codex_read_should_retry(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

fn codex_request_timeout(method: &str) -> Duration {
    match method {
        "thread/start" | "thread/resume" => Duration::from_secs(120),
        _ => Duration::from_secs(30),
    }
}

#[cfg(test)]
mod tests {
    use super::{codex_read_should_retry, codex_request_timeout, CodexClient};
    use crate::provider::CodexNotification;
    use serde_json::{json, Value};
    use std::net::TcpListener;
    use std::thread;
    use std::time::{Duration, Instant};
    use tokio_tungstenite::tungstenite::{accept, connect, Message};

    #[test]
    fn codex_read_retries_transient_empty_socket_errors() {
        assert!(codex_read_should_retry(&std::io::Error::from(
            std::io::ErrorKind::WouldBlock
        )));
        assert!(codex_read_should_retry(&std::io::Error::from(
            std::io::ErrorKind::TimedOut
        )));
        assert!(!codex_read_should_retry(&std::io::Error::from(
            std::io::ErrorKind::ConnectionReset
        )));
    }

    #[test]
    fn codex_thread_lifecycle_requests_get_startup_slack() {
        assert_eq!(
            codex_request_timeout("thread/start"),
            Duration::from_secs(120)
        );
        assert_eq!(
            codex_request_timeout("thread/resume"),
            Duration::from_secs(120)
        );
        assert_eq!(codex_request_timeout("turn/start"), Duration::from_secs(30));
    }

    #[test]
    fn codex_request_deadline_does_not_reset_while_notifications_stream() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind websocket fixture");
        let address = listener.local_addr().expect("resolve websocket fixture");
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept websocket fixture");
            let mut socket = accept(stream).expect("upgrade websocket fixture");
            socket.read().expect("read request payload");
            let deadline = Instant::now() + Duration::from_millis(300);
            while Instant::now() < deadline {
                let notification = json!({
                    "jsonrpc": "2.0",
                    "method": "item/agentMessage/delta",
                    "params": { "itemId": "item-1", "delta": "still working" },
                });
                if socket
                    .send(Message::Text(notification.to_string().into()))
                    .is_err()
                {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
        });
        let endpoint = format!("ws://{address}");
        let (mut socket, _) = connect(&endpoint).expect("connect websocket fixture");
        let client = CodexClient::new("provider-run-test", endpoint).expect("create client");
        let mut next_request_id = 1;
        let mut buffered_notifications = Vec::new();
        let timeout = Duration::from_millis(75);
        let started = Instant::now();

        let error = client
            .send_request_buffering_notifications_with_timeout::<Value>(
                &mut socket,
                &mut next_request_id,
                "turn/start",
                json!({}),
                &mut buffered_notifications,
                timeout,
            )
            .expect_err("continuous notifications must not extend the request deadline");

        assert!(error
            .to_string()
            .contains("timed out waiting for Codex app-server"));
        assert!(started.elapsed() < Duration::from_millis(250));
        assert!(!buffered_notifications.is_empty());
        drop(socket);
        server.join().expect("join websocket fixture");
    }

    #[test]
    fn codex_turn_interrupt_preserves_terminal_notification_before_response() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind websocket fixture");
        let address = listener.local_addr().expect("resolve websocket fixture");
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept websocket fixture");
            let mut socket = accept(stream).expect("upgrade websocket fixture");
            let request = socket.read().expect("read interrupt request");
            let Message::Text(request) = request else {
                panic!("expected text request");
            };
            let request: Value = serde_json::from_str(&request).expect("parse interrupt request");
            socket
                .send(Message::Text(
                    json!({
                        "jsonrpc": "2.0",
                        "method": "turn/completed",
                        "params": {
                            "threadId": "thread-1",
                            "turn": {
                                "id": "turn-1",
                                "status": "interrupted",
                                "items": []
                            }
                        }
                    })
                    .to_string()
                    .into(),
                ))
                .expect("send terminal notification");
            socket
                .send(Message::Text(
                    json!({
                        "jsonrpc": "2.0",
                        "id": request.get("id").cloned().expect("request id"),
                        "result": {}
                    })
                    .to_string()
                    .into(),
                ))
                .expect("send interrupt response");
        });
        let endpoint = format!("ws://{address}");
        let (mut socket, _) = connect(&endpoint).expect("connect websocket fixture");
        let client = CodexClient::new("provider-run-test", endpoint).expect("create client");
        let mut next_request_id = 1;
        let mut buffered_notifications = Vec::new();

        client
            .turn_interrupt(
                &mut socket,
                &mut next_request_id,
                "thread-1",
                "turn-1",
                &mut buffered_notifications,
            )
            .expect("interrupt turn");

        assert_eq!(
            buffered_notifications,
            vec![CodexNotification::TurnCompleted {
                turn_id: "turn-1".to_string(),
                status: "interrupted".to_string(),
                error_message: None,
                items: Vec::new(),
            }]
        );
        drop(socket);
        server.join().expect("join websocket fixture");
    }

    #[test]
    fn malformed_turn_started_is_skipped_without_returning_an_empty_turn_id() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind websocket fixture");
        let address = listener.local_addr().expect("resolve websocket fixture");
        let (valid_frame_ready_tx, valid_frame_ready_rx) = std::sync::mpsc::channel();
        let (release_valid_socket_tx, release_valid_socket_rx) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept websocket fixture");
            let mut malformed_socket = accept(stream).expect("upgrade websocket fixture");
            malformed_socket
                .send(Message::Text(
                    json!({
                        "jsonrpc": "2.0",
                        "method": "turn/started",
                        "params": {"threadId": "thread-1"}
                    })
                    .to_string()
                    .into(),
                ))
                .expect("send malformed turn-start notification");
            // Keep the malformed connection open so the client can deterministically
            // observe that the malformed frame is skipped and no valid frame follows.
            let (stream, _) = listener.accept().expect("accept valid websocket fixture");
            let mut valid_socket = accept(stream).expect("upgrade valid websocket fixture");
            valid_socket
                .send(Message::Text(
                    json!({
                        "jsonrpc": "2.0",
                        "method": "turn/started",
                        "params": {"turnId": "turn-1"}
                    })
                    .to_string()
                    .into(),
                ))
                .expect("send valid turn-start notification");
            valid_frame_ready_tx
                .send(())
                .expect("signal valid frame ready");
            release_valid_socket_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("client should consume valid frame before fixture closes");
        });
        let endpoint = format!("ws://{address}");
        let (mut malformed_socket, _) = connect(&endpoint).expect("connect malformed fixture");
        let client = CodexClient::new("provider-run-test", endpoint).expect("create client");

        assert_eq!(
            client
                .read_notification(&mut malformed_socket, Duration::from_secs(1))
                .expect("read malformed notification"),
            None,
        );

        let (mut valid_socket, _) = connect(&client.endpoint).expect("connect valid fixture");
        valid_frame_ready_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("fixture should send valid frame before notification deadline starts");
        assert_eq!(
            client
                .read_notification(&mut valid_socket, Duration::from_secs(1))
                .expect("read valid notification"),
            Some(CodexNotification::TurnStarted {
                turn_id: "turn-1".to_string(),
            })
        );
        release_valid_socket_tx
            .send(())
            .expect("release valid fixture");
        drop(malformed_socket);
        drop(valid_socket);
        server.join().expect("join websocket fixture");
    }
}
