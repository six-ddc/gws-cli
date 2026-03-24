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

//! Cloud Function proxy authentication for workspace extension credentials.
//!
//! Supports importing OAuth credentials from the `gemini-cli-extensions/workspace`
//! extension, which uses a Cloud Function proxy for token exchange and refresh
//! (no local `client_secret` required).

use serde::{Deserialize, Serialize};

/// Default Cloud Function URL used by the workspace extension.
const DEFAULT_CLOUD_FUNCTION_URL: &str = "https://google-workspace-extension.geminicli.com";

/// Default OAuth client ID used by the workspace extension.
const DEFAULT_CLIENT_ID: &str =
    "338689075775-o75k922vn5fdl18qergr96rp8g63e4d7.apps.googleusercontent.com";

/// OS Keychain service name used by the workspace extension.
const KEYCHAIN_SERVICE: &str = "gemini-cli-workspace-oauth";

/// OS Keychain account name used by the workspace extension.
const KEYCHAIN_ACCOUNT: &str = "main-account";

/// File name for the encrypted cloud token cache.
pub const CLOUD_TOKEN_CACHE_FILE: &str = "cloud_token_cache.json";

/// Credential type identifier used in serialized JSON.
const CREDENTIAL_TYPE: &str = "cloud_function_proxy";

/// Buffer time (in seconds) before token expiry to trigger a refresh.
const EXPIRY_BUFFER_SECS: i64 = 300; // 5 minutes

/// Credential backed by a Cloud Function proxy for token refresh.
///
/// Unlike standard OAuth, this credential does not hold a `client_secret`.
/// Instead, it delegates token refresh to a Cloud Function endpoint that
/// holds the secret server-side.
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

/// Token structure from the workspace extension's keychain entry.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceToken {
    #[allow(dead_code)]
    access_token: Option<String>,
    refresh_token: Option<String>,
    #[allow(dead_code)]
    expires_at: Option<i64>,
    #[allow(dead_code)]
    token_type: Option<String>,
    scope: Option<String>,
}

/// Top-level keychain entry structure from the workspace extension.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceKeychainEntry {
    #[allow(dead_code)]
    server_name: Option<String>,
    token: Option<WorkspaceToken>,
    #[allow(dead_code)]
    updated_at: Option<i64>,
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

/// Import credentials from the workspace extension's OS Keychain.
///
/// Reads the keychain entry at service=`gemini-cli-workspace-oauth`,
/// account=`main-account`, parses the TypeScript-format JSON, and
/// returns a `CloudFunctionCredential`.
pub fn import_from_workspace() -> Result<CloudFunctionCredential, crate::error::GwsError> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).map_err(|e| {
        crate::error::GwsError::Auth(format!(
            "Failed to access OS keychain for workspace extension: {e}"
        ))
    })?;

    let password = entry.get_password().map_err(|e| match e {
        keyring::Error::NoEntry => crate::error::GwsError::Auth(
            "No workspace extension credentials found in OS keychain. \
             Install and authenticate the gemini-cli-extensions/workspace extension first."
                .to_string(),
        ),
        other => crate::error::GwsError::Auth(format!(
            "Failed to read workspace credentials from OS keychain: {other}"
        )),
    })?;

    let keychain_entry: WorkspaceKeychainEntry =
        serde_json::from_str(&password).map_err(|e| {
            crate::error::GwsError::Auth(format!(
                "Failed to parse workspace extension credentials from keychain: {e}"
            ))
        })?;

    let token = keychain_entry.token.ok_or_else(|| {
        crate::error::GwsError::Auth(
            "Workspace extension keychain entry has no token field.".to_string(),
        )
    })?;

    let refresh_token = token.refresh_token.ok_or_else(|| {
        crate::error::GwsError::Auth(
            "Workspace extension token has no refresh_token. Re-authenticate the extension."
                .to_string(),
        )
    })?;

    let scope = token.scope.unwrap_or_default();

    Ok(CloudFunctionCredential {
        client_id: DEFAULT_CLIENT_ID.to_string(),
        refresh_token,
        cloud_function_url: DEFAULT_CLOUD_FUNCTION_URL.to_string(),
        scope,
    })
}

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
    fn test_workspace_keychain_entry_deserialize() {
        let json_str = r#"{
            "serverName": "main-account",
            "token": {
                "accessToken": "ya29.xxx",
                "refreshToken": "1//xxx",
                "expiresAt": 1711234567890,
                "tokenType": "Bearer",
                "scope": "https://www.googleapis.com/auth/documents https://www.googleapis.com/auth/drive"
            },
            "updatedAt": 1711234567890
        }"#;
        let entry: WorkspaceKeychainEntry = serde_json::from_str(json_str).unwrap();
        assert_eq!(entry.server_name.as_deref(), Some("main-account"));
        let token = entry.token.unwrap();
        assert_eq!(token.refresh_token.as_deref(), Some("1//xxx"));
        assert!(token.scope.unwrap().contains("drive"));
    }
}
