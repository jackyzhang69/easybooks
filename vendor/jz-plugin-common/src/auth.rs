//! Durable token exchange and in-memory product JWT cache.

use crate::home;
use crate::http::{self, HttpError};
use crate::identity::{AuthMode, PluginIdentity};
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine as _;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use thiserror::Error;

const MAX_EXCHANGED_TOKEN_TTL: f64 = 300.0;
// The issuer's clock can be slightly ahead of the host. This tolerance applies
// only to the upper sanity bound, never to expiry or the cache lifetime.
const EXCHANGE_CLOCK_SKEW: f64 = 30.0;
const EXCHANGE_REFRESH_SKEW: Duration = Duration::from_secs(30);
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(30);
const ALLOWED_JWT_ALGS: [&str; 2] = ["RS256", "ES256"];

#[derive(Clone)]
pub struct ProductJwt {
    pub token: String,
}

impl fmt::Debug for ProductJwt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProductJwt")
            .field("token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("not connected — no durable user credential in the canonical slot")]
    NotConnected,
    #[error("accountd rejected the credential")]
    Unauthorized,
    #[error("exchanged JWT audience does not match product identity")]
    WrongAudience,
    #[error("exchanged JWT is malformed or missing required claims")]
    Malformed,
    #[error("{0}")]
    Http(HttpError),
}

impl From<HttpError> for AuthError {
    fn from(value: HttpError) -> Self {
        AuthError::Http(value)
    }
}

#[derive(Clone)]
struct CachedExchange {
    token: String,
    refresh_at: Instant,
}

static EXCHANGE_CACHE: OnceLock<Mutex<HashMap<String, CachedExchange>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, CachedExchange>> {
    EXCHANGE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cache_key(accountd_base: &str, durable_token: &str, aud: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(accountd_base.trim_end_matches('/').as_bytes());
    hasher.update([0]);
    hasher.update(durable_token.as_bytes());
    let digest = hasher.finalize();
    let mut hex_digest = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex_digest, "{byte:02x}");
    }
    format!("{aud}:{hex_digest}")
}

/// Read the durable user credential from the canonical slot.
pub fn read_durable_token() -> Result<Option<String>, AuthError> {
    home::read_durable_token().map_err(|_| AuthError::Malformed)
}

/// Exchange the durable slot credential for a short-lived product JWT.
///
/// Validates JWT shape and claims (aud, exp, alg). **Does not verify the
/// cryptographic signature** — trust is anchored on the TLS-protected
/// accountd exchange response.
pub fn exchange(identity: &PluginIdentity, accountd_base: &str) -> Result<ProductJwt, AuthError> {
    match identity.auth_mode {
        AuthMode::Introspect => {
            let raw = read_durable_token()?.ok_or(AuthError::NotConnected)?;
            return Ok(ProductJwt { token: raw });
        }
        AuthMode::Exchange => {}
    }
    let aud = identity.aud.ok_or(AuthError::Malformed)?;
    let base = accountd_base.trim_end_matches('/');
    let durable = read_durable_token()?.ok_or(AuthError::NotConnected)?;
    if let Some(token) = get_valid_cached(base, &durable, aud) {
        return Ok(ProductJwt { token });
    }
    exchange_with_retry(identity, base, aud, &durable, false)
}

/// Validate an explicitly supplied login credential before committing it to the slot.
/// Always contacts exchange; never falls back to the current slot on rejection.
/// The caller owns saving the candidate only after its full login checks succeed.
pub fn exchange_candidate(
    identity: &PluginIdentity,
    accountd_base: &str,
    candidate: &str,
) -> Result<ProductJwt, AuthError> {
    if identity.auth_mode != AuthMode::Exchange {
        return Err(AuthError::Malformed);
    }
    let aud = identity.aud.ok_or(AuthError::Malformed)?;
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return Err(AuthError::NotConnected);
    }
    match perform_exchange(
        identity,
        accountd_base.trim_end_matches('/'),
        aud,
        candidate,
    ) {
        Err(AuthError::Http(HttpError::Status { code: 401, .. })) => Err(AuthError::Unauthorized),
        result => result,
    }
}

/// POST JSON to a product route with one 401 re-exchange retry.
pub fn post_product_json(
    identity: &PluginIdentity,
    accountd_base: &str,
    url: &str,
    body: &Value,
) -> Result<http::Response<Value>, AuthError> {
    product_request(identity, accountd_base, |token| {
        http::post_json_value(url, Some(token), body)
    })
}

/// POST JSON to a product route with typed request/response bodies.
pub fn post_product<T, R>(
    identity: &PluginIdentity,
    accountd_base: &str,
    url: &str,
    body: &T,
) -> Result<http::Response<R>, AuthError>
where
    T: Serialize,
    R: DeserializeOwned,
{
    product_request(identity, accountd_base, |token| {
        http::send_json("POST", url, Some(token), Some(body))
    })
}

/// GET JSON from a product route with one 401 re-exchange retry.
pub fn get_product_json(
    identity: &PluginIdentity,
    accountd_base: &str,
    url: &str,
) -> Result<http::Response<Value>, AuthError> {
    product_request(identity, accountd_base, |token| {
        http::send_json_value("GET", url, Some(token), None)
    })
}

/// A bounded product POST for transport operations that retry on disconnect.
pub fn post_product_with_timeout<T: Serialize, R: DeserializeOwned>(
    identity: &PluginIdentity,
    accountd_base: &str,
    url: &str,
    body: &T,
    timeout: Duration,
) -> Result<http::Response<R>, AuthError> {
    product_request(identity, accountd_base, |token| {
        http::send_json_with_timeout("POST", url, Some(token), Some(body), timeout)
    })
}

/// GET JSON from a product route with a typed response body.
pub fn get_product<R>(
    identity: &PluginIdentity,
    accountd_base: &str,
    url: &str,
) -> Result<http::Response<R>, AuthError>
where
    R: DeserializeOwned,
{
    product_request(identity, accountd_base, |token| {
        http::send_json::<R, Value>("GET", url, Some(token), None)
    })
}

/// GET JSON from a product route with one 401 re-exchange retry and an
/// operation-local read timeout. Pair long polls use this instead of changing
/// the timeout for every product request in the shared client.
pub fn get_product_with_timeout<R>(
    identity: &PluginIdentity,
    accountd_base: &str,
    url: &str,
    read_timeout: Duration,
) -> Result<http::Response<R>, AuthError>
where
    R: DeserializeOwned,
{
    product_request(identity, accountd_base, |token| {
        http::send_json_with_timeout::<R, Value>("GET", url, Some(token), None, read_timeout)
    })
}

/// POST to a product route that must return an empty HTTP 204 response. A
/// malformed successful response is surfaced as an error so callers cannot
/// acknowledge a message without a real receipt.
pub fn post_product_empty(
    identity: &PluginIdentity,
    accountd_base: &str,
    url: &str,
    body: &Value,
) -> Result<http::Response<()>, AuthError> {
    product_request(identity, accountd_base, |token| {
        http::send_empty_value_with_timeout(
            "POST",
            url,
            Some(token),
            Some(body),
            Duration::from_secs(30),
        )
    })
}

fn product_request<F, T>(
    identity: &PluginIdentity,
    accountd_base: &str,
    send: F,
) -> Result<T, AuthError>
where
    F: Fn(&str) -> Result<T, HttpError>,
{
    let mut auth_retry = false;
    loop {
        let jwt = exchange(identity, accountd_base)?;
        match send(&jwt.token) {
            Ok(response) => return Ok(response),
            Err(HttpError::Status { code: 401, .. }) if !auth_retry => {
                auth_retry = true;
                invalidate_exchange_cache(identity, accountd_base)?;
            }
            Err(HttpError::Status { code: 401, .. }) => return Err(AuthError::Unauthorized),
            Err(error) => return Err(error.into()),
        }
    }
}

/// Drop any cached JWT for the current durable slot + accountd origin + aud.
pub fn invalidate_exchange_cache(
    identity: &PluginIdentity,
    accountd_base: &str,
) -> Result<(), AuthError> {
    let aud = identity.aud.ok_or(AuthError::Malformed)?;
    let base = accountd_base.trim_end_matches('/');
    let durable = read_durable_token()?.ok_or(AuthError::NotConnected)?;
    invalidate_cache_key(base, &durable, aud);
    Ok(())
}

fn exchange_with_retry(
    identity: &PluginIdentity,
    accountd_base: &str,
    aud: &str,
    durable: &str,
    is_retry: bool,
) -> Result<ProductJwt, AuthError> {
    invalidate_cache_key(accountd_base, durable, aud);
    match perform_exchange(identity, accountd_base, aud, durable) {
        Ok(jwt) => Ok(jwt),
        Err(AuthError::Http(HttpError::Status { code: 401, .. })) if !is_retry => {
            let refreshed = read_durable_token()?.ok_or(AuthError::NotConnected)?;
            exchange_with_retry(identity, accountd_base, aud, &refreshed, true)
        }
        Err(AuthError::Http(HttpError::Status { code: 401, .. })) => Err(AuthError::Unauthorized),
        Err(error) => Err(error),
    }
}

fn perform_exchange(
    identity: &PluginIdentity,
    accountd_base: &str,
    aud: &str,
    durable: &str,
) -> Result<ProductJwt, AuthError> {
    let base = accountd_base.trim_end_matches('/');
    let url = format!("{base}/v1/token/exchange");
    let scopes: Vec<&str> = identity.product_scopes.to_vec();
    let body = json!({ "aud": aud, "scopes": scopes });
    let response = http::send_json_with_timeout::<Value, Value>(
        "POST",
        &url,
        Some(durable),
        Some(&body),
        EXCHANGE_TIMEOUT,
    )?;
    let token = response
        .body
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty() && !token.chars().any(char::is_whitespace))
        .ok_or(AuthError::Malformed)?;
    let expires_in = response.body.get("expires_in").and_then(Value::as_f64);
    validate_product_jwt(token, aud, base)?;
    if let Some(refresh_at) = compute_refresh_at(token, expires_in)? {
        store_cache(base, durable, aud, token.to_string(), refresh_at);
    }
    Ok(ProductJwt {
        token: token.to_string(),
    })
}

fn validate_product_jwt(token: &str, expected_aud: &str, issuer: &str) -> Result<(), AuthError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(AuthError::Malformed);
    }
    let header: JwtHeader = decode_json_part(parts[0]).map_err(|_| AuthError::Malformed)?;
    if !ALLOWED_JWT_ALGS.contains(&header.alg.as_str()) {
        return Err(AuthError::Malformed);
    }
    let claims: JwtClaims = decode_json_part(parts[1]).map_err(|_| AuthError::Malformed)?;
    if claims.aud.as_deref() != Some(expected_aud) {
        return Err(AuthError::WrongAudience);
    }
    if claims.iss.as_deref() != Some(issuer) {
        return Err(AuthError::Malformed);
    }
    let remaining = jwt_remaining_secs(token)?;
    if remaining <= 0.0 {
        return Err(AuthError::Malformed);
    }
    if remaining > MAX_EXCHANGED_TOKEN_TTL + EXCHANGE_CLOCK_SKEW {
        return Err(AuthError::Malformed);
    }
    Ok(())
}

fn jwt_remaining_secs(token: &str) -> Result<f64, AuthError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(AuthError::Malformed);
    }
    let claims: JwtClaims = decode_json_part(parts[1]).map_err(|_| AuthError::Malformed)?;
    let exp = claims.exp.ok_or(AuthError::Malformed)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AuthError::Malformed)?
        .as_secs_f64();
    Ok(exp - now)
}

fn compute_refresh_at(token: &str, expires_in: Option<f64>) -> Result<Option<Instant>, AuthError> {
    let claim_ttl = jwt_remaining_secs(token)?;
    let ttl = match expires_in {
        Some(metadata) if metadata.is_finite() && metadata > 0.0 => metadata.min(claim_ttl),
        _ => claim_ttl,
    };
    let skewed = ttl - EXCHANGE_REFRESH_SKEW.as_secs_f64();
    if skewed <= 0.0 {
        return Ok(None);
    }
    Ok(Some(
        Instant::now() + Duration::from_secs_f64(skewed.min(MAX_EXCHANGED_TOKEN_TTL)),
    ))
}

#[derive(Debug, Deserialize)]
struct JwtHeader {
    alg: String,
}

#[derive(Debug, Deserialize)]
struct JwtClaims {
    aud: Option<String>,
    iss: Option<String>,
    exp: Option<f64>,
}

fn decode_json_part<T: for<'de> Deserialize<'de>>(part: &str) -> Result<T, ()> {
    let decoded = URL_SAFE_NO_PAD
        .decode(part.as_bytes())
        .or_else(|_| URL_SAFE.decode(part.as_bytes()))
        .map_err(|_| ())?;
    serde_json::from_slice(&decoded).map_err(|_| ())
}

fn get_valid_cached(accountd_base: &str, durable: &str, aud: &str) -> Option<String> {
    let key = cache_key(accountd_base, durable, aud);
    let guard = cache().lock().ok()?;
    let cached = guard.get(&key)?;
    if Instant::now() >= cached.refresh_at {
        return None;
    }
    jwt_remaining_secs(&cached.token)
        .ok()
        .filter(|ttl| *ttl > 0.0)?;
    Some(cached.token.clone())
}

fn store_cache(accountd_base: &str, durable: &str, aud: &str, token: String, refresh_at: Instant) {
    if refresh_at <= Instant::now() {
        return;
    }
    let key = cache_key(accountd_base, durable, aud);
    if let Ok(mut guard) = cache().lock() {
        guard.insert(key, CachedExchange { token, refresh_at });
    }
}

fn invalidate_cache_key(accountd_base: &str, durable: &str, aud: &str) {
    let key = cache_key(accountd_base, durable, aud);
    if let Ok(mut guard) = cache().lock() {
        guard.remove(&key);
    }
}

#[cfg(test)]
pub(crate) fn clear_exchange_cache_for_tests() {
    if let Ok(mut guard) = cache().lock() {
        guard.clear();
    }
}

/// Extend cached refresh deadline so JWT `exp` re-verification is the invalidation path.
#[cfg(test)]
pub(crate) fn patch_cached_refresh_at_for_tests(
    accountd_base: &str,
    durable: &str,
    aud: &str,
    refresh_at: Instant,
) {
    let key = cache_key(accountd_base, durable, aud);
    if let Ok(mut guard) = cache().lock() {
        if let Some(entry) = guard.get_mut(&key) {
            entry.refresh_at = refresh_at;
        }
    }
}

#[cfg(test)]
mod tests;
