//! Access control for the OpenHeart HTTP server.
//!
//! * Host validation defeats DNS rebinding: only loopback names, IP literals and
//!   explicitly allowed host names are served.
//! * Origin and Fetch-Metadata checks reject cross-site requests (CSRF and cross-origin
//!   reads); cross-origin access is granted only to an explicit CORS allowlist.
//! * An optional access token (always required on non-loopback binds) gates every route
//!   except `/api/health`; browsers obtain it once through `/?token=...`, which stores it
//!   in an `HttpOnly`, `SameSite=Strict` cookie.

#![forbid(unsafe_code)]

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::adapters::http::{Request, Response};

/// How the caller relates to this server, derived from `Origin` and `Sec-Fetch-Site`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Caller {
    /// A page served by this server (or a top-level, user-initiated navigation).
    SameOrigin,
    /// A browser page on an origin from the CORS allowlist.
    AllowedCrossOrigin(String),
    /// A client that sends no browser metadata (curl, scripts, health probes).
    NonBrowser,
    /// A page on another site, including other ports of the same host.
    CrossSite,
}

/// A request refused by the access policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Denial {
    /// HTTP/1.1 requires a Host header.
    MissingHost,
    /// The Host header names a host this server does not answer for.
    ForeignHost,
    /// The request comes from another site.
    CrossSite,
    /// The access token is missing or wrong.
    Unauthenticated,
}

impl Denial {
    /// The response sent for this denial.
    pub fn response(self) -> Response {
        match self {
            Self::MissingHost => Response::error(400, "missing Host header"),
            Self::ForeignHost => Response::error(
                403,
                "this server does not answer for the requested host name; \
                 add it to OPENHEART_ALLOWED_HOSTS to allow it",
            ),
            Self::CrossSite => Response::error(
                403,
                "cross-site requests are refused; add the calling origin to \
                 OPENHEART_CORS_ORIGINS to allow it",
            ),
            Self::Unauthenticated => Response::error(
                401,
                "an access token is required: open the login URL printed at server startup \
                 (/?token=...) or send 'Authorization: Bearer <token>'",
            )
            .header("WWW-Authenticate", "Bearer realm=\"openheart\""),
        }
    }
}

/// The server's access rules.
#[derive(Debug, Clone)]
pub(crate) struct AccessPolicy {
    allowed_hosts: Vec<String>,
    cors_origins: Vec<String>,
    token: Option<String>,
    token_cookie: String,
}

impl AccessPolicy {
    /// Builds a policy; host names and origins are compared case-insensitively.
    pub fn new(
        allowed_hosts: Vec<String>,
        cors_origins: Vec<String>,
        token: Option<String>,
        port: u16,
    ) -> Self {
        Self {
            allowed_hosts: allowed_hosts
                .into_iter()
                .map(|host| host.trim().trim_end_matches('.').to_ascii_lowercase())
                .filter(|host| !host.is_empty())
                .collect(),
            cors_origins: cors_origins
                .into_iter()
                .map(|origin| origin.trim().trim_end_matches('/').to_ascii_lowercase())
                .filter(|origin| !origin.is_empty())
                .collect(),
            token,
            token_cookie: format!("openheart_token_{}", port),
        }
    }

    /// Whether requests must present the access token.
    pub fn token_required(&self) -> bool {
        self.token.is_some()
    }

    /// Validates the Host header and returns it.
    pub fn check_host<'a>(&self, request: &'a Request) -> Result<&'a str, Denial> {
        let host = request.header("host").ok_or(Denial::MissingHost)?;
        let name = host_name(host).ok_or(Denial::ForeignHost)?;
        let lower = name.trim_end_matches('.').to_ascii_lowercase();
        let allowed = lower.parse::<IpAddr>().is_ok()
            || lower == "localhost"
            || lower.ends_with(".localhost")
            || self.allowed_hosts.iter().any(|allowed| *allowed == lower);
        if allowed {
            Ok(host)
        } else {
            Err(Denial::ForeignHost)
        }
    }

    /// Classifies the caller of `request`, whose validated Host header is `host`.
    pub fn classify(&self, request: &Request, host: &str) -> Caller {
        if let Some(origin) = request.header("origin") {
            let normalized = origin.trim_end_matches('/').to_ascii_lowercase();
            if normalized == format!("http://{}", host.to_ascii_lowercase()) {
                return Caller::SameOrigin;
            }
            if self.cors_origins.iter().any(|allowed| *allowed == normalized) {
                return Caller::AllowedCrossOrigin(origin.to_string());
            }
            return Caller::CrossSite;
        }
        match request.header("sec-fetch-site") {
            Some(site) if site.eq_ignore_ascii_case("cross-site") => Caller::CrossSite,
            Some(site) if site.eq_ignore_ascii_case("same-site") => Caller::CrossSite,
            Some(_) => Caller::SameOrigin,
            None => Caller::NonBrowser,
        }
    }

    /// Whether `request` presents the access token (always true when none is configured).
    pub fn is_authenticated(&self, request: &Request) -> bool {
        let Some(expected) = &self.token else {
            return true;
        };
        let bearer = request.header("authorization").and_then(|value| {
            value
                .split_once(' ')
                .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
                .map(|(_, token)| token.trim())
        });
        [
            bearer,
            request.header("x-openheart-token"),
            request.cookie(&self.token_cookie),
        ]
        .into_iter()
        .flatten()
        .any(|candidate| constant_time_eq(candidate.as_bytes(), expected.as_bytes()))
    }

    /// Whether `candidate` equals the configured token.
    pub fn token_matches(&self, candidate: &str) -> bool {
        self.token
            .as_ref()
            .is_some_and(|expected| constant_time_eq(candidate.as_bytes(), expected.as_bytes()))
    }

    /// `Set-Cookie` value that stores the token in the browser.
    pub fn token_cookie(&self, token: &str) -> String {
        format!(
            "{}={}; Path=/; HttpOnly; SameSite=Strict",
            self.token_cookie, token
        )
    }

    /// CORS response headers for `caller` (empty unless it is an allowlisted origin).
    pub fn cors_headers(caller: &Caller) -> Vec<(String, String)> {
        match caller {
            Caller::AllowedCrossOrigin(origin) => vec![
                ("Access-Control-Allow-Origin".to_string(), origin.clone()),
                ("Vary".to_string(), "Origin".to_string()),
            ],
            _ => Vec::new(),
        }
    }
}

/// Extracts the host name from a Host header value (`name`, `name:port`, `[v6]:port`).
fn host_name(authority: &str) -> Option<&str> {
    if let Some(rest) = authority.strip_prefix('[') {
        let end = rest.find(']')?;
        let after = &rest[end + 1..];
        if !after.is_empty() && !after.starts_with(':') {
            return None;
        }
        return Some(&rest[..end]);
    }
    let name = match authority.rsplit_once(':') {
        Some((name, port)) if port.bytes().all(|byte| byte.is_ascii_digit()) => name,
        Some(_) => return None,
        None => authority,
    };
    if name.is_empty() || name.contains(':') {
        None
    } else {
        Some(name)
    }
}

/// Compares two byte strings in time independent of where they differ.
pub(crate) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

/// Returns `bytes` random bytes as lower-case hex.
///
/// Each 64-bit block is a SipHash-1-3 output keyed by a fresh [`RandomState`], whose keys
/// std seeds from the operating system's CSPRNG; the counter, clock and process id inputs
/// only guarantee distinct blocks within the process.
pub(crate) fn random_hex(bytes: usize) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut out = String::with_capacity(bytes * 2);
    while out.len() < bytes * 2 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        hasher.write_u128(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0),
        );
        hasher.write_u32(std::process::id());
        for byte in hasher.finish().to_le_bytes() {
            if out.len() >= bytes * 2 {
                break;
            }
            out.push_str(&format!("{:02x}", byte));
        }
    }
    out
}
