// Copyright 2026 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Cloud Function proxy authentication for Google Workspace.
//!
//! Implements the same OAuth flow as the `gemini-cli-extensions/workspace`
//! extension: the Cloud Function holds the `client_secret` server-side, so
//! the CLI only needs the public `client_id`.
//!
//! ## Login flow
//!
//! 1. Start a local HTTP server on a random port.
//! 2. Build a Google OAuth URL with `redirect_uri` pointing at the Cloud
//!    Function (which holds the client secret).
//! 3. The `state` parameter carries a base64-encoded JSON payload containing
//!    a CSRF token and the local callback URI.
//! 4. The user authorises in the browser → Google redirects to the Cloud
//!    Function with the authorisation code.
//! 5. The Cloud Function exchanges the code for tokens and redirects back
//!    to the local server with the tokens as query parameters.
//! 6. The local server receives the tokens and the login completes.
//!
//! ## Token refresh
//!
//! POST to `{cloud_function_url}/refreshToken` with the refresh token.
//! The Cloud Function refreshes the access token server-side.

use serde::{Deserialize, Serialize};

/// Default Cloud Function URL used by the workspace extension.
const DEFAULT_CLOUD_FUNCTION_URL: &str = "https://google-workspace-extension.geminicli.com";

/// Default OAuth client ID used by the workspace extension.
const DEFAULT_CLIENT_ID: &str =
    "338689075775-o75k922vn5fdl18qergr96rp8g63e4d7.apps.googleusercontent.com";

/// Google OAuth authorization endpoint.
const GOOGLE_AUTH_URI: &str = "https://accounts.google.com/o/oauth2/auth";

/// File name for the encrypted cloud token cache.
pub const CLOUD_TOKEN_CACHE_FILE: &str = "cloud_token_cache.json";

/// Credential type identifier used in serialized JSON.
const CREDENTIAL_TYPE: &str = "cloud_function_proxy";

/// Buffer time (in seconds) before token expiry to trigger a refresh.
const EXPIRY_BUFFER_SECS: i64 = 300; // 5 minutes

/// Timeout for the browser-based login flow.
const LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// Credential backed by a Cloud Function proxy for token refresh.
///
/// Unlike standard OAuth, this credential does not hold a `client_secret`.
/// Instead, it delegates token exchange and refresh to a Cloud Function
/// endpoint that holds the secret server-side.
#[derive(Debug, Clone)]
pub struct CloudFunctionCredential {
    pub client_id: String,
    pub refresh_token: String,
    pub cloud_function_url: String,
    pub scope: String,
}

/// Cached access token stored on disk (encrypted).
#[derive(Debug, Serialize, Deserialize)]
struct CachedToken {
    access_token: String,
    /// Unix timestamp in seconds when the token expires.
    expires_at: i64,
}

/// Tokens received from the Cloud Function callback after initial login.
struct CallbackTokens {
    access_token: String,
    refresh_token: String,
    scope: String,
    expiry_date: i64,
}

/// Response from the Cloud Function `/refreshToken` endpoint.
#[derive(Debug, Deserialize)]
struct RefreshResponse {
    access_token: String,
    /// Expiry as a Unix timestamp in milliseconds.
    expiry_date: i64,
    #[allow(dead_code)]
    scope: Option<String>,
    #[allow(dead_code)]
    token_type: Option<String>,
}

impl CloudFunctionCredential {
    /// Obtain a valid access token, refreshing via the Cloud Function if needed.
    ///
    /// 1. Try to load a cached token from the encrypted cache file.
    /// 2. If the cached token is still valid (with 5-minute buffer), return it.
    /// 3. Otherwise, POST to `{cloud_function_url}/refreshToken` to get a new one.
    /// 4. Cache the new token encrypted on disk.
    pub async fn get_token(&self) -> anyhow::Result<String> {
        let cache_path = crate::auth_commands::config_dir().join(CLOUD_TOKEN_CACHE_FILE);

        // Try cached token
        if let Some(cached) = self.load_cached_token(&cache_path) {
            let now = chrono::Utc::now().timestamp();
            if cached.expires_at - now > EXPIRY_BUFFER_SECS {
                return Ok(cached.access_token);
            }
        }

        // Refresh via Cloud Function
        let new_token = self.refresh_token_via_cloud_function().await?;

        // Cache the new token (best-effort)
        if let Err(e) = self.save_cached_token(&cache_path, &new_token) {
            eprintln!("Warning: failed to cache cloud token: {e}");
        }

        Ok(new_token.access_token)
    }

    /// Load and decrypt the cached token from disk.
    fn load_cached_token(&self, path: &std::path::Path) -> Option<CachedToken> {
        let data = std::fs::read(path).ok()?;
        let decrypted = crate::credential_store::decrypt(&data).ok()?;
        let json_str = String::from_utf8(decrypted).ok()?;
        serde_json::from_str(&json_str).ok()
    }

    /// Encrypt and save the token cache to disk.
    fn save_cached_token(
        &self,
        path: &std::path::Path,
        token: &CachedToken,
    ) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json_str = serde_json::to_string(token)?;
        let encrypted = crate::credential_store::encrypt(json_str.as_bytes())?;
        crate::fs_util::atomic_write(path, &encrypted)?;
        Ok(())
    }

    /// Refresh the access token by calling the Cloud Function proxy.
    async fn refresh_token_via_cloud_function(&self) -> anyhow::Result<CachedToken> {
        let client = crate::client::build_client()?;
        let url = format!("{}/refreshToken", self.cloud_function_url);

        let body = serde_json::json!({
            "refresh_token": self.refresh_token,
        });

        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Cloud Function refresh request failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!(
                "Cloud Function returned HTTP {status} during token refresh: {text}"
            );
        }

        let refresh_resp: RefreshResponse = resp
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse Cloud Function response: {e}"))?;

        Ok(CachedToken {
            access_token: refresh_resp.access_token,
            // expiry_date is in milliseconds, convert to seconds
            expires_at: refresh_resp.expiry_date / 1000,
        })
    }
}

// ── OAuth login flow ────────────────────────────────────────────────

/// Run the full Cloud Function proxy OAuth login flow.
///
/// 1. Start a local callback server on a random port.
/// 2. Open the Google OAuth consent page in the user's browser (the
///    `redirect_uri` points to the Cloud Function, NOT localhost).
/// 3. After user consent, Google redirects to the Cloud Function with
///    the auth code.  The Cloud Function exchanges it for tokens and
///    redirects back to the local server with the tokens as query params.
/// 4. Return a [`CloudFunctionCredential`] and the initial access token.
pub async fn login(
    scopes: &[&str],
) -> Result<(CloudFunctionCredential, String), crate::error::GwsError> {
    let cloud_function_url = DEFAULT_CLOUD_FUNCTION_URL.to_string();
    let client_id = DEFAULT_CLIENT_ID.to_string();

    // 1. Bind a local TCP listener on a random port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| {
            crate::error::GwsError::Auth(format!("Failed to bind local callback server: {e}"))
        })?;
    let local_addr = listener.local_addr().map_err(|e| {
        crate::error::GwsError::Auth(format!("Failed to get local address: {e}"))
    })?;
    let local_redirect_uri = format!("http://127.0.0.1:{}/oauth2callback", local_addr.port());

    // 2. Generate CSRF token and build state payload.
    let csrf_token = generate_csrf_token();
    let state_payload = serde_json::json!({
        "uri": local_redirect_uri,
        "manual": false,
        "csrf": csrf_token,
    });
    let state = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        serde_json::to_string(&state_payload).unwrap_or_default(),
    );

    // 3. Construct Google OAuth authorization URL.
    let scope_str = scopes.join(" ");
    let auth_url = format!(
        "{GOOGLE_AUTH_URI}?client_id={client_id}\
         &redirect_uri={redirect_uri}\
         &response_type=code\
         &access_type=offline\
         &prompt=consent\
         &scope={scope}\
         &state={state}",
        client_id = percent_encoding::percent_encode(
            client_id.as_bytes(),
            percent_encoding::NON_ALPHANUMERIC,
        ),
        redirect_uri = percent_encoding::percent_encode(
            cloud_function_url.as_bytes(),
            percent_encoding::NON_ALPHANUMERIC,
        ),
        scope = percent_encoding::percent_encode(
            scope_str.as_bytes(),
            percent_encoding::NON_ALPHANUMERIC,
        ),
        state = percent_encoding::percent_encode(
            state.as_bytes(),
            percent_encoding::NON_ALPHANUMERIC,
        ),
    );

    // 4. Open browser automatically; fall back to printing the URL.
    if let Err(e) = open::that(&auth_url) {
        eprintln!("Failed to open browser automatically: {e}");
        eprintln!("Open this URL in your browser to authenticate:\n");
        eprintln!("  {auth_url}\n");
    } else {
        eprintln!("Opened browser for authentication. Waiting for callback...");
    }

    // 5. Wait for the Cloud Function to redirect back with tokens.
    let tokens = tokio::time::timeout(LOGIN_TIMEOUT, wait_for_callback(listener, &csrf_token))
        .await
        .map_err(|_| {
            crate::error::GwsError::Auth(
                "Authentication timed out after 5 minutes. \
                 Please try again and complete the login in your browser."
                    .to_string(),
            )
        })?
        .map_err(|e| crate::error::GwsError::Auth(format!("OAuth callback failed: {e}")))?;

    let cred = CloudFunctionCredential {
        client_id,
        refresh_token: tokens.refresh_token,
        cloud_function_url,
        scope: tokens.scope,
    };

    // Cache the access token for later use.
    let cache_path = crate::auth_commands::config_dir().join(CLOUD_TOKEN_CACHE_FILE);
    let cached = CachedToken {
        access_token: tokens.access_token.clone(),
        expires_at: tokens.expiry_date / 1000,
    };
    if let Err(e) = cred.save_cached_token(&cache_path, &cached) {
        eprintln!("Warning: failed to cache cloud token: {e}");
    }

    Ok((cred, tokens.access_token))
}

/// Generate a cryptographically random CSRF token (64 hex chars).
fn generate_csrf_token() -> String {
    use rand::Rng;
    use std::fmt::Write;
    let bytes: [u8; 32] = rand::thread_rng().gen();
    let mut s = String::with_capacity(64);
    for b in &bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Wait for the Cloud Function to redirect back to our local server with tokens.
///
/// The Cloud Function sends the tokens as query parameters:
/// `?access_token=...&refresh_token=...&scope=...&token_type=...&expiry_date=...&state=...`
async fn wait_for_callback(
    listener: tokio::net::TcpListener,
    expected_csrf: &str,
) -> anyhow::Result<CallbackTokens> {
    use tokio::io::AsyncReadExt;

    let (mut stream, _addr) = listener.accept().await?;

    // Read the HTTP request (we only need the first line with the path + query).
    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).await?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // Parse the request line: "GET /oauth2callback?... HTTP/1.1"
    let request_line = request.lines().next().unwrap_or_default();
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default();

    let url = url::Url::parse(&format!("http://localhost{path}"))
        .map_err(|e| anyhow::anyhow!("Failed to parse callback URL: {e}"))?;

    // Check for errors from the Cloud Function.
    if let Some(error) = url.query_pairs().find(|(k, _)| k == "error").map(|(_, v)| v) {
        let desc = url
            .query_pairs()
            .find(|(k, _)| k == "error_description")
            .map(|(_, v)| v.to_string())
            .unwrap_or_default();
        send_response(&mut stream, 400, "Authentication failed. You may close this tab.").await;
        anyhow::bail!("OAuth error: {error}. {desc}");
    }

    // Validate CSRF token.
    let returned_state = url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .map(|(_, v)| v.to_string())
        .unwrap_or_default();
    if returned_state != expected_csrf {
        send_response(&mut stream, 403, "State mismatch. Possible CSRF attack.").await;
        anyhow::bail!("OAuth state mismatch — possible CSRF attack");
    }

    // Extract tokens from query parameters.
    let get_param = |name: &str| -> Option<String> {
        url.query_pairs()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.to_string())
    };

    let access_token = get_param("access_token")
        .ok_or_else(|| anyhow::anyhow!("No access_token in callback"))?;
    let refresh_token = get_param("refresh_token")
        .ok_or_else(|| anyhow::anyhow!("No refresh_token in callback"))?;
    let scope = get_param("scope").unwrap_or_default();
    let expiry_date_str = get_param("expiry_date")
        .ok_or_else(|| anyhow::anyhow!("No expiry_date in callback"))?;
    let expiry_date: i64 = expiry_date_str
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid expiry_date: {e}"))?;

    send_response(&mut stream, 200, "Authentication successful! You may close this tab.").await;

    Ok(CallbackTokens {
        access_token,
        refresh_token,
        scope,
        expiry_date,
    })
}

/// Send a minimal HTTP response and close the connection.
async fn send_response(stream: &mut tokio::net::TcpStream, status: u16, body: &str) {
    use tokio::io::AsyncWriteExt;
    let status_text = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {status_text}\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        len = body.len(),
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
}

// ── Serialization helpers ───────────────────────────────────────────

/// Try to parse a `CloudFunctionCredential` from a JSON value.
///
/// Returns `Some` if the JSON has `"type": "cloud_function_proxy"` and
/// all required fields; `None` otherwise.
pub fn try_parse(json: &serde_json::Value) -> Option<CloudFunctionCredential> {
    if json.get("type")?.as_str()? != CREDENTIAL_TYPE {
        return None;
    }
    let client_id = json.get("client_id")?.as_str()?.to_string();
    let refresh_token = json.get("refresh_token")?.as_str()?.to_string();
    let cloud_function_url = json.get("cloud_function_url")?.as_str()?.to_string();
    let scope = json
        .get("scope")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    Some(CloudFunctionCredential {
        client_id,
        refresh_token,
        cloud_function_url,
        scope,
    })
}

/// Serialize a `CloudFunctionCredential` to a JSON value for storage.
pub fn to_json(cred: &CloudFunctionCredential) -> serde_json::Value {
    serde_json::json!({
        "type": CREDENTIAL_TYPE,
        "client_id": cred.client_id,
        "refresh_token": cred.refresh_token,
        "cloud_function_url": cred.cloud_function_url,
        "scope": cred.scope,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_parse_valid() {
        let json = serde_json::json!({
            "type": "cloud_function_proxy",
            "client_id": "test-client-id",
            "refresh_token": "1//test-refresh",
            "cloud_function_url": "https://example.com",
            "scope": "https://www.googleapis.com/auth/drive",
        });
        let cred = try_parse(&json).expect("should parse");
        assert_eq!(cred.client_id, "test-client-id");
        assert_eq!(cred.refresh_token, "1//test-refresh");
        assert_eq!(cred.cloud_function_url, "https://example.com");
        assert_eq!(cred.scope, "https://www.googleapis.com/auth/drive");
    }

    #[test]
    fn test_try_parse_wrong_type() {
        let json = serde_json::json!({
            "type": "authorized_user",
            "client_id": "id",
            "refresh_token": "rt",
        });
        assert!(try_parse(&json).is_none());
    }

    #[test]
    fn test_try_parse_missing_fields() {
        let json = serde_json::json!({
            "type": "cloud_function_proxy",
            "client_id": "id",
            // missing refresh_token and cloud_function_url
        });
        assert!(try_parse(&json).is_none());
    }

    #[test]
    fn test_to_json_roundtrip() {
        let cred = CloudFunctionCredential {
            client_id: "cid".to_string(),
            refresh_token: "rt".to_string(),
            cloud_function_url: "https://fn.example.com".to_string(),
            scope: "scope1 scope2".to_string(),
        };
        let json = to_json(&cred);
        assert_eq!(json["type"], "cloud_function_proxy");
        assert_eq!(json["client_id"], "cid");
        assert_eq!(json["refresh_token"], "rt");
        assert_eq!(json["cloud_function_url"], "https://fn.example.com");
        assert_eq!(json["scope"], "scope1 scope2");

        // Roundtrip through try_parse
        let parsed = try_parse(&json).expect("roundtrip should work");
        assert_eq!(parsed.client_id, cred.client_id);
        assert_eq!(parsed.refresh_token, cred.refresh_token);
        assert_eq!(parsed.cloud_function_url, cred.cloud_function_url);
        assert_eq!(parsed.scope, cred.scope);
    }

    #[test]
    fn test_try_parse_missing_scope_defaults_to_empty() {
        let json = serde_json::json!({
            "type": "cloud_function_proxy",
            "client_id": "id",
            "refresh_token": "rt",
            "cloud_function_url": "https://fn.example.com",
        });
        let cred = try_parse(&json).expect("should parse without scope");
        assert_eq!(cred.scope, "");
    }

    #[test]
    fn test_generate_csrf_token() {
        let token = generate_csrf_token();
        assert_eq!(token.len(), 64); // 32 bytes = 64 hex chars
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
