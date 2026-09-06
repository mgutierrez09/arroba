//! Cloud API HTTP transport, URL encoding, and error classification.

use crate::error::DaemonError;

const CLOUD_API_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

pub(crate) fn normalize_cloud_api_url(api_url: &str) -> Result<String, DaemonError> {
    let normalized = api_url.trim().trim_end_matches('/').to_string();
    if normalized.is_empty() {
        return Err(DaemonError::LocalTransport {
            operation: "normalize cloud relay api url",
            message: "api_url must not be empty".to_string(),
        });
    }
    Ok(normalized)
}

pub(crate) async fn post_cloud_json<T>(
    api_url: String,
    path: &'static str,
    body: serde_json::Value,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    tokio::task::spawn_blocking(move || post_cloud_json_blocking(api_url, path, body))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "post cloud relay json",
            message: error.to_string(),
        })?
}

pub(crate) async fn post_cloud_json_dynamic<T>(
    api_url: String,
    path: String,
    body: serde_json::Value,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    tokio::task::spawn_blocking(move || post_cloud_json_blocking(api_url, &path, body))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "post cloud relay json",
            message: error.to_string(),
        })?
}

pub(crate) async fn get_cloud_json<T>(api_url: String, path: String) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    tokio::task::spawn_blocking(move || get_cloud_json_blocking(api_url, &path))
        .await
        .map_err(|error| DaemonError::LocalTransport {
            operation: "get cloud relay json",
            message: error.to_string(),
        })?
}

pub(crate) async fn get_cloud_json_authenticated<T>(
    api_url: String,
    path: String,
    bearer_token: String,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        get_cloud_json_authenticated_blocking(api_url, &path, &bearer_token)
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "get authenticated cloud json",
        message: error.to_string(),
    })?
}

pub(crate) async fn post_cloud_json_authenticated<T>(
    api_url: String,
    path: String,
    bearer_token: String,
    body: serde_json::Value,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        post_cloud_json_authenticated_blocking(api_url, &path, &bearer_token, body)
    })
    .await
    .map_err(|error| DaemonError::LocalTransport {
        operation: "post authenticated cloud json",
        message: error.to_string(),
    })?
}

fn post_cloud_json_blocking<T>(
    api_url: String,
    path: &str,
    body: serde_json::Value,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned,
{
    post_cloud_json_blocking_with_timeout(api_url, path, body, CLOUD_API_REQUEST_TIMEOUT)
}

fn post_cloud_json_blocking_with_timeout<T>(
    api_url: String,
    path: &str,
    body: serde_json::Value,
    timeout: std::time::Duration,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned,
{
    let url = format!("{api_url}{path}");
    let agent = ureq::AgentBuilder::new().timeout(timeout).build();
    let response = agent
        .post(&url)
        .set("content-type", "application/json")
        .send_string(&body.to_string())
        .map_err(cloud_transport_error)?;
    let payload = response
        .into_string()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "read cloud relay response",
            message: error.to_string(),
        })?;
    serde_json::from_str::<T>(&payload).map_err(|error| DaemonError::LocalTransport {
        operation: "decode cloud relay response",
        message: error.to_string(),
    })
}

fn get_cloud_json_blocking<T>(api_url: String, path: &str) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned,
{
    let url = format!("{api_url}{path}");
    let agent = ureq::AgentBuilder::new()
        .timeout(CLOUD_API_REQUEST_TIMEOUT)
        .build();
    let response = agent.get(&url).call().map_err(cloud_transport_error)?;
    let payload = response
        .into_string()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "read cloud relay response",
            message: error.to_string(),
        })?;
    serde_json::from_str::<T>(&payload).map_err(|error| DaemonError::LocalTransport {
        operation: "decode cloud relay response",
        message: error.to_string(),
    })
}

fn get_cloud_json_authenticated_blocking<T>(
    api_url: String,
    path: &str,
    bearer_token: &str,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned,
{
    let url = format!("{api_url}{path}");
    let agent = ureq::AgentBuilder::new()
        .timeout(CLOUD_API_REQUEST_TIMEOUT)
        .build();
    let response = agent
        .get(&url)
        .set("authorization", &format!("Bearer {bearer_token}"))
        .call()
        .map_err(cloud_transport_error)?;
    decode_cloud_response(response)
}

fn post_cloud_json_authenticated_blocking<T>(
    api_url: String,
    path: &str,
    bearer_token: &str,
    body: serde_json::Value,
) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned,
{
    let url = format!("{api_url}{path}");
    let agent = ureq::AgentBuilder::new()
        .timeout(CLOUD_API_REQUEST_TIMEOUT)
        .build();
    let response = agent
        .post(&url)
        .set("authorization", &format!("Bearer {bearer_token}"))
        .set("content-type", "application/json")
        .send_string(&body.to_string())
        .map_err(cloud_transport_error)?;
    decode_cloud_response(response)
}

fn decode_cloud_response<T>(response: ureq::Response) -> Result<T, DaemonError>
where
    T: serde::de::DeserializeOwned,
{
    let payload = response
        .into_string()
        .map_err(|error| DaemonError::LocalTransport {
            operation: "read cloud relay response",
            message: error.to_string(),
        })?;
    serde_json::from_str::<T>(&payload).map_err(|error| DaemonError::LocalTransport {
        operation: "decode cloud relay response",
        message: error.to_string(),
    })
}

pub(crate) fn cloud_url_component(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}

fn cloud_transport_error(error: ureq::Error) -> DaemonError {
    let message = match error {
        ureq::Error::Status(status, response) => {
            let body = response.into_string().unwrap_or_default();
            if body.is_empty() {
                format!("cloud relay request failed with {status}")
            } else if let Some(code) = cloud_api_error_code(&body) {
                format!("cloud relay request failed with {status}: cloud_api_code={code}: {body}")
            } else {
                format!("cloud relay request failed with {status}: {body}")
            }
        }
        ureq::Error::Transport(error) => error.to_string(),
    };
    DaemonError::LocalTransport {
        operation: "cloud relay request",
        message,
    }
}

fn cloud_api_error_code(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|payload| {
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(|code| code.as_str())
                .map(str::to_string)
        })
}

pub(crate) fn cloud_error_code(error: &DaemonError) -> Option<&str> {
    let message = match error {
        DaemonError::LocalTransport { message, .. } => message,
        _ => return None,
    };
    let code = message.split_once("cloud_api_code=")?.1;
    Some(code.split(':').next().unwrap_or(code))
}

pub(crate) fn cloud_error_is_retryable(error: &DaemonError) -> bool {
    if let Some(code) = cloud_error_code(error) {
        match code {
            "capacity_exceeded" | "dependency_unavailable" | "internal_error" | "rate_limited" => {
                return true
            }
            "account_deleted"
            | "authorization_denied"
            | "identity_conflict"
            | "identity_revoked"
            | "invalid_request"
            | "not_found"
            | "realm_not_found"
            | "session_invalid"
            | "subscription_required"
            | "user_deleted" => return false,
            _ => {}
        }
    }
    let (operation, message) = match error {
        DaemonError::LocalTransport {
            operation, message, ..
        } => (*operation, message.as_str()),
        _ => return true,
    };
    if operation == "decode cloud relay response" {
        return true;
    }
    ![
        "cloud relay request failed with 400",
        "cloud relay request failed with 401",
        "cloud relay request failed with 402",
        "cloud relay request failed with 403",
        "cloud relay request failed with 404",
        "cloud relay request failed with 409",
        "cloud relay request failed with 410",
        "cloud relay request failed with 422",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

pub(crate) fn is_stale_cloud_link_error(error: &DaemonError) -> bool {
    let message = match error {
        DaemonError::LocalTransport { message, .. } => message.as_str(),
        _ => return false,
    };
    [
        "cloud_api_code=session_invalid",
        "cloud_api_code=realm_not_found",
        "cloud_api_code=account_deleted",
        "cloud_api_code=user_deleted",
        "\"code\":\"session_invalid\"",
        "\"code\":\"realm_not_found\"",
        "\"code\":\"account_deleted\"",
        "\"code\":\"user_deleted\"",
        "invalid_session",
        "cloud relay request failed with 401",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_url_component_percent_encodes_query_values() {
        assert_eq!(
            cloud_url_component("token/a+b?x=1"),
            "token%2Fa%2Bb%3Fx%3D1"
        );
        assert_eq!(cloud_url_component("abc-_.~XYZ"), "abc-_.~XYZ");
    }

    #[test]
    fn cloud_api_error_code_reads_cloud_error_payloads() {
        assert_eq!(
            cloud_api_error_code(r#"{"error":{"code":"session_invalid"}}"#),
            Some("session_invalid".to_string())
        );
        assert_eq!(cloud_api_error_code(r#"{"error":{}}"#), None);
    }

    #[test]
    fn stale_cloud_link_errors_only_include_invalid_cloud_sessions() {
        assert!(!is_stale_cloud_link_error(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "cloud_api_code=identity_revoked".to_string(),
        }));
        assert!(is_stale_cloud_link_error(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "cloud relay request failed with 401".to_string(),
        }));
        assert!(!is_stale_cloud_link_error(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "network timeout".to_string(),
        }));
        assert!(!is_stale_cloud_link_error(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "cloud relay request failed with 403".to_string(),
        }));
    }

    #[test]
    fn cloud_error_code_preserves_structured_api_failures() {
        let error = DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "cloud relay request failed with 409: cloud_api_code=identity_conflict: body"
                .to_string(),
        };
        assert_eq!(cloud_error_code(&error), Some("identity_conflict"));
        assert_eq!(
            cloud_error_code(&DaemonError::LocalTransport {
                operation: "cloud relay request",
                message: "network timeout".to_string(),
            }),
            None
        );
    }

    #[test]
    fn cloud_error_retryability_preserves_terminal_and_transient_classes() {
        assert!(!cloud_error_is_retryable(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "cloud relay request failed with 409: cloud_api_code=identity_conflict: body"
                .to_string(),
        }));
        assert!(cloud_error_is_retryable(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message:
                "cloud relay request failed with 503: cloud_api_code=dependency_unavailable: body"
                    .to_string(),
        }));
        assert!(cloud_error_is_retryable(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message:
                "cloud relay request failed with 503: cloud_api_code=service_unavailable: body"
                    .to_string(),
        }));
        assert!(cloud_error_is_retryable(&DaemonError::LocalTransport {
            operation: "cloud relay request",
            message: "network timeout".to_string(),
        }));
        assert!(cloud_error_is_retryable(&DaemonError::LocalTransport {
            operation: "decode cloud relay response",
            message: "unexpected response shape".to_string(),
        }));
    }

    #[test]
    fn cloud_post_has_a_total_response_deadline() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stalled Cloud fixture");
        let address = listener
            .local_addr()
            .expect("stalled Cloud fixture address");
        let fixture = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().expect("accept stalled Cloud request");
            std::thread::sleep(std::time::Duration::from_millis(250));
        });
        let started = std::time::Instant::now();
        let result = post_cloud_json_blocking_with_timeout::<serde_json::Value>(
            format!("http://{address}"),
            "/stalled",
            serde_json::json!({}),
            std::time::Duration::from_millis(50),
        );
        assert!(result.is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        fixture.join().expect("stalled Cloud fixture");
    }
}
