//! OAuth 2.1 authorization for Streamable HTTP MCP servers (spec 2025-11-25).

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, WWW_AUTHENTICATE};
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{info, warn};

use crate::config::McpOAuthConfig;
use crate::mcp::Error;

const PROTOCOL_RESOURCE_META_WELL_KNOWN: &str = "/.well-known/oauth-protected-resource";

/// Parsed `WWW-Authenticate: Bearer ...` challenge.
#[derive(Debug, Clone, Default)]
pub struct BearerChallenge {
    pub resource_metadata: Option<String>,
    pub scope: Option<String>,
    pub error: Option<String>,
}

/// In-memory + on-disk OAuth token set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub token_type: String,
    pub scope: Option<String>,
    /// Absolute unix expiry seconds; `None` means unknown (treat as short-lived).
    pub expires_at: Option<u64>,
}

impl TokenSet {
    /// Returns whether the access token should be treated as unusable.
    ///
    /// Missing `expires_at` is treated as expired so callers refresh or re-auth instead of
    /// assuming an immortal token (`api-parse-dont-validate`).
    #[must_use]
    pub fn is_expired(&self, skew_secs: u64) -> bool {
        match self.expires_at {
            Some(at) => now_unix().saturating_add(skew_secs) >= at,
            None => true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProtectedResourceMetadata {
    pub resource: Option<String>,
    pub authorization_servers: Vec<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthorizationServerMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
    #[serde(default)]
    pub client_id_metadata_document_supported: bool,
}

#[derive(Debug, Clone)]
struct ResolvedClient {
    client_id: String,
    client_secret: Option<String>,
}

/// OAuth helper bound to one MCP HTTP endpoint.
pub struct McpOAuth {
    server_name: String,
    mcp_url: Url,
    cfg: McpOAuthConfig,
    http: Client,
    tokens: Option<TokenSet>,
}

impl McpOAuth {
    /// Builds an OAuth helper and loads any cached tokens from `token_store`.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when the HTTP client cannot be built or the token store is corrupt.
    pub fn new(server_name: &str, mcp_url: Url, cfg: McpOAuthConfig) -> Result<Self, Error> {
        let http = Client::builder()
            .timeout(Duration::from_secs(60))
            .no_proxy()
            .build()
            .map_err(|source| Error::Http {
                server: server_name.to_string(),
                source,
            })?;
        let mut this = Self {
            server_name: server_name.to_string(),
            mcp_url,
            cfg,
            http,
            tokens: None,
        };
        if let Some(path) = this.token_store_path() {
            if path.exists() {
                match load_token_store(&path) {
                    Ok(tokens) => this.tokens = Some(tokens),
                    Err(err) => warn!(
                        server = %this.server_name,
                        path = %path.display(),
                        error = %err,
                        "ignoring corrupt MCP token store"
                    ),
                }
            }
        }
        Ok(this)
    }

    #[must_use]
    pub fn access_token(&self) -> Option<&str> {
        self.tokens.as_ref().map(|t| t.access_token.as_str())
    }

    /// Ensures a usable access token, refreshing or running the interactive code flow.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when discovery, refresh, or interactive login fails.
    pub async fn ensure_access_token(
        &mut self,
        challenge: Option<&BearerChallenge>,
    ) -> Result<String, Error> {
        if let Some(tokens) = &self.tokens {
            if !tokens.is_expired(30) {
                return Ok(tokens.access_token.clone());
            }
        }
        self.refresh_or_authorize(challenge).await
    }

    /// Recovers after an HTTP `401` / `403 insufficient_scope` challenge.
    ///
    /// Unlike [`Self::ensure_access_token`], this never reuses a cached access token that the
    /// server has already rejected. Scope step-up always runs interactive authorization with the
    /// challenged scopes; other challenges try refresh first, then interactive login.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] when refresh or interactive login fails.
    pub async fn recover_access_token(
        &mut self,
        challenge: Option<&BearerChallenge>,
    ) -> Result<String, Error> {
        let step_up = challenge
            .and_then(|c| c.error.as_deref())
            .is_some_and(|e| e == "insufficient_scope");
        if step_up {
            // Drop the under-scoped access token so we cannot accidentally reuse it.
            if let Some(tokens) = &mut self.tokens {
                tokens.access_token.clear();
                tokens.expires_at = Some(0);
            }
            return self.authorize_interactive(challenge).await;
        }
        self.refresh_or_authorize(challenge).await
    }

    async fn refresh_or_authorize(
        &mut self,
        challenge: Option<&BearerChallenge>,
    ) -> Result<String, Error> {
        if self
            .tokens
            .as_ref()
            .and_then(|t| t.refresh_token.as_ref())
            .is_some()
        {
            match self.refresh_tokens(challenge).await {
                Ok(token) => return Ok(token),
                Err(err) => {
                    warn!(
                        server = %self.server_name,
                        error = %err,
                        "MCP token refresh failed; falling back to interactive login"
                    );
                }
            }
        }
        self.authorize_interactive(challenge).await
    }

    async fn refresh_tokens(
        &mut self,
        challenge: Option<&BearerChallenge>,
    ) -> Result<String, Error> {
        let refresh = self
            .tokens
            .as_ref()
            .and_then(|t| t.refresh_token.clone())
            .ok_or_else(|| Error::OAuth {
                server: self.server_name.clone(),
                error: "no refresh_token available".to_string(),
            })?;
        let (prm, as_meta) = self.discover(challenge).await?;
        let client = self.resolve_client(&as_meta).await?;
        let resource = canonical_resource_uri(&self.mcp_url, prm.resource.as_deref());
        let mut form = vec![
            ("grant_type".to_string(), "refresh_token".to_string()),
            ("refresh_token".to_string(), refresh),
            ("resource".to_string(), resource),
            ("client_id".to_string(), client.client_id.clone()),
        ];
        if let Some(secret) = &client.client_secret {
            form.push(("client_secret".to_string(), secret.clone()));
        }
        let tokens = self.exchange_token(&as_meta.token_endpoint, &form).await?;
        self.persist_tokens(tokens)?;
        self.tokens
            .as_ref()
            .map(|t| t.access_token.clone())
            .ok_or_else(|| Error::OAuth {
                server: self.server_name.clone(),
                error: "token store empty after refresh".to_string(),
            })
    }

    async fn authorize_interactive(
        &mut self,
        challenge: Option<&BearerChallenge>,
    ) -> Result<String, Error> {
        let (prm, as_meta) = self.discover(challenge).await?;
        if as_meta.code_challenge_methods_supported.is_empty()
            || !as_meta
                .code_challenge_methods_supported
                .iter()
                .any(|m| m.eq_ignore_ascii_case("S256"))
        {
            return Err(Error::OAuth {
                server: self.server_name.clone(),
                error: "authorization server does not advertise PKCE S256 support".to_string(),
            });
        }
        let client = self.resolve_client(&as_meta).await?;
        let scopes = challenged_scopes(challenge, &self.cfg, &prm);
        let resource = canonical_resource_uri(&self.mcp_url, prm.resource.as_deref());

        let listener =
            TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], self.cfg.redirect_port))).await?;
        let local = listener.local_addr()?;
        let redirect_uri = format!("http://127.0.0.1:{}/callback", local.port());

        let (verifier, challenge_s256) = pkce_pair();
        let state = random_urlsafe(16);

        let mut auth_url =
            Url::parse(&as_meta.authorization_endpoint).map_err(|err| Error::OAuth {
                server: self.server_name.clone(),
                error: format!("invalid authorization_endpoint: {err}"),
            })?;
        {
            let mut q = auth_url.query_pairs_mut();
            q.append_pair("response_type", "code");
            q.append_pair("client_id", &client.client_id);
            q.append_pair("redirect_uri", &redirect_uri);
            q.append_pair("code_challenge", &challenge_s256);
            q.append_pair("code_challenge_method", "S256");
            q.append_pair("state", &state);
            q.append_pair("resource", &resource);
            if !scopes.is_empty() {
                q.append_pair("scope", &scopes.join(" "));
            }
        }

        info!(
            server = %self.server_name,
            url = %auth_url,
            "opening browser for MCP OAuth login"
        );
        if let Err(err) = open_browser(auth_url.as_str()) {
            return Err(Error::OAuth {
                server: self.server_name.clone(),
                error: format!(
                    "failed to open browser for OAuth ({err}); open {auth_url} manually or set a Bearer token in mcp headers / token_store"
                ),
            });
        }

        let code = wait_for_auth_code(listener, &state)
            .await
            .map_err(|error| Error::OAuth {
                server: self.server_name.clone(),
                error,
            })?;

        let mut form = vec![
            ("grant_type".to_string(), "authorization_code".to_string()),
            ("code".to_string(), code),
            ("redirect_uri".to_string(), redirect_uri),
            ("client_id".to_string(), client.client_id.clone()),
            ("code_verifier".to_string(), verifier),
            ("resource".to_string(), resource),
        ];
        if let Some(secret) = &client.client_secret {
            form.push(("client_secret".to_string(), secret.clone()));
        }
        let tokens = self.exchange_token(&as_meta.token_endpoint, &form).await?;
        self.persist_tokens(tokens)?;
        self.tokens
            .as_ref()
            .map(|t| t.access_token.clone())
            .ok_or_else(|| Error::OAuth {
                server: self.server_name.clone(),
                error: "token store empty after authorization".to_string(),
            })
    }

    /// Runs discovery and interactive/refresh auth when a 401 challenge is received.
    ///
    /// Used by tests to inject a pre-set authorization code path via token exchange only.
    #[cfg(test)]
    pub async fn exchange_authorization_code_for_test(
        &mut self,
        token_endpoint: &str,
        client_id: &str,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
        resource: &str,
    ) -> Result<String, Error> {
        let form = vec![
            ("grant_type".to_string(), "authorization_code".to_string()),
            ("code".to_string(), code.to_string()),
            ("redirect_uri".to_string(), redirect_uri.to_string()),
            ("client_id".to_string(), client_id.to_string()),
            ("code_verifier".to_string(), code_verifier.to_string()),
            ("resource".to_string(), resource.to_string()),
        ];
        let tokens = self.exchange_token(token_endpoint, &form).await?;
        self.persist_tokens(tokens)?;
        Ok(self.tokens.as_ref().unwrap().access_token.clone())
    }

    /// Injects a cached token set for unit tests (e.g. revoked-but-unexpired access tokens).
    #[cfg(test)]
    pub fn inject_tokens_for_test(&mut self, tokens: TokenSet) {
        self.tokens = Some(tokens);
    }

    async fn discover(
        &self,
        challenge: Option<&BearerChallenge>,
    ) -> Result<(ProtectedResourceMetadata, AuthorizationServerMetadata), Error> {
        let prm = self.fetch_protected_resource_metadata(challenge).await?;
        let as_url = prm
            .authorization_servers
            .first()
            .ok_or_else(|| Error::OAuth {
                server: self.server_name.clone(),
                error: "protected resource metadata has no authorization_servers".to_string(),
            })?;
        let as_meta = self.fetch_authorization_server_metadata(as_url).await?;
        Ok((prm, as_meta))
    }

    async fn fetch_protected_resource_metadata(
        &self,
        challenge: Option<&BearerChallenge>,
    ) -> Result<ProtectedResourceMetadata, Error> {
        let mut candidates = Vec::new();
        if let Some(url) = challenge.and_then(|c| c.resource_metadata.clone()) {
            match validate_metadata_fetch_url(&url, Some(&self.mcp_url)) {
                Ok(()) => candidates.push(url),
                Err(reason) => {
                    warn!(
                        server = %self.server_name,
                        url = %url,
                        reason = %reason,
                        "ignoring unsafe resource_metadata URL from WWW-Authenticate"
                    );
                }
            }
        }
        candidates.extend(well_known_resource_metadata_urls(&self.mcp_url));

        let mut last_err = None;
        for url in candidates {
            match self.http.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    return resp.json().await.map_err(|source| Error::Http {
                        server: self.server_name.clone(),
                        source,
                    });
                }
                Ok(resp) => {
                    last_err = Some(format!("{url} -> HTTP {}", resp.status()));
                }
                Err(err) => last_err = Some(format!("{url} -> {err}")),
            }
        }
        Err(Error::OAuth {
            server: self.server_name.clone(),
            error: format!(
                "failed to fetch protected resource metadata: {}",
                last_err.unwrap_or_else(|| "no candidates".to_string())
            ),
        })
    }

    async fn fetch_authorization_server_metadata(
        &self,
        issuer: &str,
    ) -> Result<AuthorizationServerMetadata, Error> {
        let issuer_url = Url::parse(issuer).map_err(|err| Error::OAuth {
            server: self.server_name.clone(),
            error: format!("invalid authorization server issuer `{issuer}`: {err}"),
        })?;
        validate_metadata_fetch_url(issuer, None).map_err(|reason| Error::OAuth {
            server: self.server_name.clone(),
            error: format!("rejected authorization server issuer `{issuer}`: {reason}"),
        })?;
        let mut last_err = None;
        for url in authorization_server_metadata_urls(&issuer_url) {
            match self.http.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    let meta: AuthorizationServerMetadata =
                        resp.json().await.map_err(|source| Error::Http {
                            server: self.server_name.clone(),
                            source,
                        })?;
                    validate_authorization_server_metadata(&issuer_url, &meta).map_err(
                        |reason| Error::OAuth {
                            server: self.server_name.clone(),
                            error: reason,
                        },
                    )?;
                    return Ok(meta);
                }
                Ok(resp) => last_err = Some(format!("{url} -> HTTP {}", resp.status())),
                Err(err) => last_err = Some(format!("{url} -> {err}")),
            }
        }
        Err(Error::OAuth {
            server: self.server_name.clone(),
            error: format!(
                "failed to fetch authorization server metadata: {}",
                last_err.unwrap_or_else(|| "no candidates".to_string())
            ),
        })
    }

    async fn resolve_client(
        &self,
        as_meta: &AuthorizationServerMetadata,
    ) -> Result<ResolvedClient, Error> {
        if let Some(meta_url) = self
            .cfg
            .client_id_metadata_url
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            if as_meta.client_id_metadata_document_supported {
                return Ok(ResolvedClient {
                    client_id: meta_url.to_string(),
                    client_secret: self.resolve_client_secret()?,
                });
            }
            return Err(Error::OAuth {
                server: self.server_name.clone(),
                error: "client_id_metadata_url set but AS does not support CIMD".to_string(),
            });
        }

        if let Some(id) = self
            .cfg
            .client_id
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            return Ok(ResolvedClient {
                client_id: id.to_string(),
                client_secret: self.resolve_client_secret()?,
            });
        }

        if let Some(reg) = &as_meta.registration_endpoint {
            return self.dynamic_register(reg).await;
        }

        Err(Error::OAuth {
            server: self.server_name.clone(),
            error: "mcp auth requires `client_id`, `client_id_metadata_url`, or AS dynamic registration"
                .to_string(),
        })
    }

    fn resolve_client_secret(&self) -> Result<Option<String>, Error> {
        let Some(env_name) = self
            .cfg
            .client_secret_env
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        else {
            return Ok(None);
        };
        match std::env::var(env_name) {
            Ok(v) if !v.trim().is_empty() => Ok(Some(v)),
            Ok(_) | Err(std::env::VarError::NotPresent) => Err(Error::OAuth {
                server: self.server_name.clone(),
                error: format!("client secret env `{env_name}` is missing or empty"),
            }),
            Err(err) => Err(Error::OAuth {
                server: self.server_name.clone(),
                error: format!("failed to read client secret env `{env_name}`: {err}"),
            }),
        }
    }

    async fn dynamic_register(&self, registration_endpoint: &str) -> Result<ResolvedClient, Error> {
        let body = serde_json::json!({
            "client_name": "agent_Kuibyshev",
            "redirect_uris": ["http://127.0.0.1/callback"],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        });
        let resp = self
            .http
            .post(registration_endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|source| Error::Http {
                server: self.server_name.clone(),
                source,
            })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::OAuth {
                server: self.server_name.clone(),
                error: format!("dynamic client registration failed: HTTP {status}: {text}"),
            });
        }
        #[derive(Deserialize)]
        struct Reg {
            client_id: String,
            client_secret: Option<String>,
        }
        let reg: Reg = resp.json().await.map_err(|source| Error::Http {
            server: self.server_name.clone(),
            source,
        })?;
        Ok(ResolvedClient {
            client_id: reg.client_id,
            client_secret: reg.client_secret,
        })
    }

    async fn exchange_token(
        &self,
        token_endpoint: &str,
        form: &[(String, String)],
    ) -> Result<TokenSet, Error> {
        let resp = self
            .http
            .post(token_endpoint)
            .form(form)
            .send()
            .await
            .map_err(|source| Error::Http {
                server: self.server_name.clone(),
                source,
            })?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::OAuth {
                server: self.server_name.clone(),
                error: format!("token endpoint returned HTTP {status}: {text}"),
            });
        }
        #[derive(Deserialize)]
        struct TokenResponse {
            access_token: String,
            #[serde(default)]
            refresh_token: Option<String>,
            #[serde(default = "default_bearer_token_type")]
            token_type: String,
            #[serde(default)]
            expires_in: Option<u64>,
            #[serde(default)]
            scope: Option<String>,
        }
        let raw: TokenResponse = resp.json().await.map_err(|source| Error::Http {
            server: self.server_name.clone(),
            source,
        })?;
        if !raw.token_type.eq_ignore_ascii_case("bearer") {
            return Err(Error::OAuth {
                server: self.server_name.clone(),
                error: format!(
                    "unsupported token_type `{}` (expected Bearer)",
                    raw.token_type
                ),
            });
        }
        let expires_at = raw.expires_in.map(|secs| now_unix().saturating_add(secs));
        Ok(TokenSet {
            access_token: raw.access_token,
            refresh_token: raw
                .refresh_token
                .or_else(|| self.tokens.as_ref().and_then(|t| t.refresh_token.clone())),
            token_type: raw.token_type,
            scope: raw.scope,
            expires_at,
        })
    }

    fn persist_tokens(&mut self, tokens: TokenSet) -> Result<(), Error> {
        if let Some(path) = self.token_store_path() {
            save_token_store(&path, &tokens).map_err(|source| Error::OAuth {
                server: self.server_name.clone(),
                error: format!("failed to write token store `{}`: {source}", path.display()),
            })?;
        }
        self.tokens = Some(tokens);
        Ok(())
    }

    fn token_store_path(&self) -> Option<PathBuf> {
        self.cfg.token_store.as_ref().map(|p| expand_tilde(p))
    }
}

fn default_bearer_token_type() -> String {
    "Bearer".to_string()
}

/// Rejects metadata fetch URLs that are not HTTPS (except loopback HTTP for local development).
///
/// When `same_origin_as` is set, HTTP(S) URLs must share that origin (scheme+host+port).
fn validate_metadata_fetch_url(raw: &str, same_origin_as: Option<&Url>) -> Result<(), String> {
    let url = Url::parse(raw).map_err(|err| format!("invalid URL: {err}"))?;
    if !is_allowed_oauth_endpoint(&url) {
        return Err(format!(
            "URL must use https (or http only for loopback); got `{}`",
            url.scheme()
        ));
    }
    if let Some(origin) = same_origin_as {
        if !urls_same_origin(&url, origin) && !is_loopback_url(&url) {
            return Err(
                "resource_metadata URL must share the MCP server origin or be loopback".to_string(),
            );
        }
    }
    Ok(())
}

fn validate_authorization_server_metadata(
    expected_issuer: &Url,
    meta: &AuthorizationServerMetadata,
) -> Result<(), String> {
    let reported = Url::parse(&meta.issuer).map_err(|err| {
        format!(
            "authorization server metadata has invalid issuer `{}`: {err}",
            meta.issuer
        )
    })?;
    if !issuers_match(expected_issuer, &reported) {
        return Err(format!(
            "authorization server issuer mismatch: expected `{}`, got `{}`",
            expected_issuer, meta.issuer
        ));
    }
    for (label, endpoint) in [
        (
            "authorization_endpoint",
            meta.authorization_endpoint.as_str(),
        ),
        ("token_endpoint", meta.token_endpoint.as_str()),
    ] {
        let url =
            Url::parse(endpoint).map_err(|err| format!("invalid {label} `{endpoint}`: {err}"))?;
        if !is_allowed_oauth_endpoint(&url) {
            return Err(format!(
                "{label} must use https (or http only for loopback); got `{endpoint}`"
            ));
        }
    }
    if let Some(reg) = &meta.registration_endpoint {
        let url = Url::parse(reg)
            .map_err(|err| format!("invalid registration_endpoint `{reg}`: {err}"))?;
        if !is_allowed_oauth_endpoint(&url) {
            return Err(format!(
                "registration_endpoint must use https (or http only for loopback); got `{reg}`"
            ));
        }
    }
    Ok(())
}

fn is_allowed_oauth_endpoint(url: &Url) -> bool {
    match url.scheme() {
        "https" => true,
        "http" => is_loopback_url(url),
        _ => false,
    }
}

fn is_loopback_url(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(addr)) => addr.is_loopback(),
        Some(url::Host::Ipv6(addr)) => addr.is_loopback(),
        Some(url::Host::Domain(host)) => {
            host.eq_ignore_ascii_case("localhost") || host.eq_ignore_ascii_case("localhost.")
        }
        None => false,
    }
}

fn urls_same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

/// RFC 8414 issuer comparison: identical after stripping a single trailing slash.
fn issuers_match(expected: &Url, reported: &Url) -> bool {
    fn normalize(url: &Url) -> String {
        let mut s = url.as_str().trim_end_matches('/').to_string();
        if s.is_empty() {
            s = url.as_str().to_string();
        }
        s
    }
    normalize(expected) == normalize(reported)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn expand_tilde(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            return PathBuf::from(home).join(rest);
        }
    }
    if raw == "~" {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            return PathBuf::from(home);
        }
    }
    path.to_path_buf()
}

fn challenged_scopes(
    challenge: Option<&BearerChallenge>,
    cfg: &McpOAuthConfig,
    prm: &ProtectedResourceMetadata,
) -> Vec<String> {
    if let Some(scope) = challenge.and_then(|c| c.scope.as_ref()) {
        return scope.split_whitespace().map(str::to_string).collect();
    }
    if !cfg.scopes.is_empty() {
        return cfg.scopes.clone();
    }
    prm.scopes_supported.clone()
}

/// Canonical resource URI for RFC 8707 (no fragment; trailing slash normalized away for root).
#[must_use]
pub fn canonical_resource_uri(mcp_url: &Url, prm_resource: Option<&str>) -> String {
    if let Some(r) = prm_resource.map(str::trim).filter(|s| !s.is_empty()) {
        return r.to_string();
    }
    let mut url = mcp_url.clone();
    url.set_fragment(None);
    url.set_query(None);
    let mut s = url.to_string();
    if s.ends_with('/') && url.path() != "/" {
        s.pop();
    }
    s
}

/// Builds PRM well-known URLs for an MCP endpoint (path-aware then root).
#[must_use]
pub fn well_known_resource_metadata_urls(mcp_url: &Url) -> Vec<String> {
    let mut out = Vec::new();
    let path = mcp_url.path().trim_end_matches('/');
    if !path.is_empty() && path != "/" {
        let mut u = mcp_url.clone();
        u.set_path(&format!("{PROTOCOL_RESOURCE_META_WELL_KNOWN}{path}"));
        u.set_query(None);
        u.set_fragment(None);
        out.push(u.to_string());
    }
    let mut root = mcp_url.clone();
    root.set_path(PROTOCOL_RESOURCE_META_WELL_KNOWN);
    root.set_query(None);
    root.set_fragment(None);
    out.push(root.to_string());
    out
}

/// AS metadata + OIDC discovery URLs (path-aware then root).
#[must_use]
pub fn authorization_server_metadata_urls(issuer: &Url) -> Vec<String> {
    let mut out = Vec::new();
    let path = issuer.path().trim_end_matches('/');
    if !path.is_empty() && path != "/" {
        let mut oauth = issuer.clone();
        oauth.set_path(&format!("/.well-known/oauth-authorization-server{path}"));
        oauth.set_query(None);
        oauth.set_fragment(None);
        out.push(oauth.to_string());

        let mut oidc = issuer.clone();
        oidc.set_path(&format!("{path}/.well-known/openid-configuration"));
        oidc.set_query(None);
        oidc.set_fragment(None);
        out.push(oidc.to_string());
    }
    let mut oauth_root = issuer.clone();
    oauth_root.set_path("/.well-known/oauth-authorization-server");
    oauth_root.set_query(None);
    oauth_root.set_fragment(None);
    out.push(oauth_root.to_string());

    let mut oidc_root = issuer.clone();
    oidc_root.set_path("/.well-known/openid-configuration");
    oidc_root.set_query(None);
    oidc_root.set_fragment(None);
    out.push(oidc_root.to_string());
    out
}

/// Parse a Bearer WWW-Authenticate header value.
#[must_use]
pub fn parse_www_authenticate(headers: &HeaderMap) -> Option<BearerChallenge> {
    let raw = headers.get(WWW_AUTHENTICATE)?.to_str().ok()?;
    parse_www_authenticate_value(raw)
}

/// Parse one `WWW-Authenticate` header string.
#[must_use]
pub fn parse_www_authenticate_value(raw: &str) -> Option<BearerChallenge> {
    let trimmed = raw.trim();
    let rest = trimmed
        .strip_prefix("Bearer")
        .or_else(|| trimmed.strip_prefix("bearer"))?
        .trim_start();
    let mut challenge = BearerChallenge::default();
    for part in split_auth_params(rest) {
        let (k, v) = match part.split_once('=') {
            Some(pair) => pair,
            None => continue,
        };
        let key = k.trim();
        let value = unquote(v.trim());
        match key {
            "resource_metadata" => challenge.resource_metadata = Some(value),
            "scope" => challenge.scope = Some(value),
            "error" => challenge.error = Some(value),
            _ => {}
        }
    }
    Some(challenge)
}

fn split_auth_params(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for ch in input.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            ',' if !in_quotes => {
                if !current.trim().is_empty() {
                    parts.push(current.trim().to_string());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts
}

fn unquote(value: &str) -> String {
    value
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(value)
        .to_string()
}

fn pkce_pair() -> (String, String) {
    let verifier = random_urlsafe(32);
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());
    (verifier, challenge)
}

fn random_urlsafe(nbytes: usize) -> String {
    let mut buf = vec![0u8; nbytes];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

fn load_token_store(path: &Path) -> Result<TokenSet, String> {
    let data = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&data).map_err(|e| e.to_string())
}

fn save_token_store(path: &Path, tokens: &TokenSet) -> Result<(), std::io::Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let data = serde_json::to_vec_pretty(tokens)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let mut file = fs::File::create(path)?;
    file.write_all(&data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn open_browser(url: &str) -> Result<(), String> {
    let result = {
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("cmd")
                .args(["/C", "start", "", url])
                .spawn()
        }
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open").arg(url).spawn()
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            std::process::Command::new("xdg-open").arg(url).spawn()
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
        {
            Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "no browser opener for this platform",
            ))
        }
    };
    result.map(|_| ()).map_err(|e| e.to_string())
}

async fn wait_for_auth_code(listener: TcpListener, expected_state: &str) -> Result<String, String> {
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(300), listener.accept())
        .await
        .map_err(|_| "timed out waiting for OAuth redirect".to_string())?
        .map_err(|e| e.to_string())?;

    let mut buf = vec![0u8; 8192];
    let n = socket.read(&mut buf).await.map_err(|e| e.to_string())?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let first_line = req.lines().next().unwrap_or("");
    let path = first_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| format!("malformed OAuth callback request: {first_line}"))?;
    let url = Url::parse(&format!("http://127.0.0.1{path}"))
        .map_err(|e| format!("invalid callback path: {e}"))?;
    let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
    if let Some(err) = params.get("error") {
        let desc = params
            .get("error_description")
            .map(String::as_str)
            .unwrap_or("");
        let body = format!("OAuth error: {err} {desc}");
        write_http_response(&mut socket, StatusCode::BAD_REQUEST, &body).await?;
        return Err(body);
    }
    let state = params
        .get("state")
        .ok_or_else(|| "OAuth callback missing state".to_string())?;
    if state != expected_state {
        write_http_response(&mut socket, StatusCode::BAD_REQUEST, "state mismatch").await?;
        return Err("OAuth state mismatch".to_string());
    }
    let code = params
        .get("code")
        .ok_or_else(|| "OAuth callback missing code".to_string())?
        .clone();
    write_http_response(
        &mut socket,
        StatusCode::OK,
        "Authorization complete. You can close this tab and return to the agent.",
    )
    .await?;
    Ok(code)
}

async fn write_http_response(
    socket: &mut tokio::net::TcpStream,
    status: StatusCode,
    body: &str,
) -> Result<(), String> {
    let resp = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status.as_u16(),
        status.canonical_reason().unwrap_or("OK"),
        body.len(),
        body
    );
    socket
        .write_all(resp.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let _ = socket.shutdown().await;
    Ok(())
}

/// Apply Bearer token to a header map (replacing any existing Authorization).
pub fn apply_bearer(headers: &mut HeaderMap, token: &str) -> Result<(), Error> {
    let value = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|err| Error::OAuth {
        server: "headers".to_string(),
        error: format!("invalid access token for Authorization header: {err}"),
    })?;
    headers.insert(AUTHORIZATION, value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bearer_challenge() {
        let raw = r#"Bearer error="insufficient_scope", scope="a b", resource_metadata="https://example.com/.well-known/oauth-protected-resource""#;
        let c = parse_www_authenticate_value(raw).unwrap();
        assert_eq!(c.error.as_deref(), Some("insufficient_scope"));
        assert_eq!(c.scope.as_deref(), Some("a b"));
        assert_eq!(
            c.resource_metadata.as_deref(),
            Some("https://example.com/.well-known/oauth-protected-resource")
        );
    }

    #[test]
    fn well_known_urls_are_path_aware() {
        let url = Url::parse("https://mcp.example.com/public/mcp").unwrap();
        let urls = well_known_resource_metadata_urls(&url);
        assert!(urls[0].ends_with("/.well-known/oauth-protected-resource/public/mcp"));
        assert!(urls[1].ends_with("/.well-known/oauth-protected-resource"));
    }

    #[test]
    fn pkce_challenge_is_s256() {
        let (verifier, challenge) = pkce_pair();
        assert!(!verifier.is_empty());
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        assert_eq!(URL_SAFE_NO_PAD.encode(hasher.finalize()), challenge);
    }

    #[test]
    fn canonical_resource_strips_query() {
        let url = Url::parse("https://mcp.example.com/mcp?x=1#frag").unwrap();
        assert_eq!(
            canonical_resource_uri(&url, None),
            "https://mcp.example.com/mcp"
        );
    }

    #[test]
    fn missing_expires_at_is_treated_as_expired() {
        let tokens = TokenSet {
            access_token: "a".into(),
            refresh_token: None,
            token_type: "Bearer".into(),
            scope: None,
            expires_at: None,
        };
        assert!(tokens.is_expired(0));
    }

    #[test]
    fn rejects_issuer_mismatch() {
        let expected = Url::parse("https://auth.example.com").unwrap();
        let meta = AuthorizationServerMetadata {
            issuer: "https://evil.example.com".into(),
            authorization_endpoint: "https://evil.example.com/authorize".into(),
            token_endpoint: "https://evil.example.com/token".into(),
            registration_endpoint: None,
            code_challenge_methods_supported: vec!["S256".into()],
            client_id_metadata_document_supported: false,
        };
        let err = validate_authorization_server_metadata(&expected, &meta).unwrap_err();
        assert!(err.contains("issuer mismatch"));
    }

    #[test]
    fn rejects_non_loopback_http_token_endpoint() {
        let expected = Url::parse("https://auth.example.com").unwrap();
        let meta = AuthorizationServerMetadata {
            issuer: "https://auth.example.com".into(),
            authorization_endpoint: "https://auth.example.com/authorize".into(),
            token_endpoint: "http://auth.example.com/token".into(),
            registration_endpoint: None,
            code_challenge_methods_supported: vec!["S256".into()],
            client_id_metadata_document_supported: false,
        };
        let err = validate_authorization_server_metadata(&expected, &meta).unwrap_err();
        assert!(err.contains("token_endpoint"));
    }

    #[test]
    fn allows_loopback_http_endpoints() {
        let expected = Url::parse("http://127.0.0.1:8080").unwrap();
        let meta = AuthorizationServerMetadata {
            issuer: "http://127.0.0.1:8080".into(),
            authorization_endpoint: "http://127.0.0.1:8080/authorize".into(),
            token_endpoint: "http://127.0.0.1:8080/token".into(),
            registration_endpoint: None,
            code_challenge_methods_supported: vec!["S256".into()],
            client_id_metadata_document_supported: false,
        };
        validate_authorization_server_metadata(&expected, &meta).expect("loopback http ok");
    }

    #[test]
    fn rejects_cross_origin_resource_metadata() {
        let mcp = Url::parse("https://mcp.example.com/mcp").unwrap();
        let err =
            validate_metadata_fetch_url("https://169.254.169.254/latest/meta-data", Some(&mcp))
                .unwrap_err();
        assert!(err.contains("origin") || err.contains("https"));
    }

    #[test]
    fn accepts_same_origin_resource_metadata() {
        let mcp = Url::parse("https://mcp.example.com/mcp").unwrap();
        validate_metadata_fetch_url(
            "https://mcp.example.com/.well-known/oauth-protected-resource",
            Some(&mcp),
        )
        .expect("same origin");
    }
}
