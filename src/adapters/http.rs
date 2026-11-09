//! Minimal, bounded HTTP/1.1 request reader and response writer for the OpenHeart server.
//!
//! Requests are framed by `Content-Length` (chunked request bodies are refused with 501),
//! the head and the body have hard size caps, and all reads of one request run against a
//! single deadline. One request is served per connection (`Connection: close`). HEAD
//! responses carry the headers of the equivalent GET, without a body. Every response gets
//! a fixed set of security headers.

#![forbid(unsafe_code)]

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

/// Upper bounds applied while reading one request.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RequestLimits {
    /// Maximum size of the request line plus headers.
    pub max_head_bytes: usize,
    /// Maximum `Content-Length` accepted.
    pub max_body_bytes: usize,
    /// Total time allowed to receive the whole request.
    pub read_timeout: Duration,
}

const MAX_HEADER_COUNT: usize = 100;

/// A parsed request. Header names are lower-cased; the path is percent-decoded.
#[derive(Debug)]
pub(crate) struct Request {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// First value of header `name` (lower-case).
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// First value of query parameter `name`.
    pub fn query_param(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Value of cookie `name` across all `Cookie` headers.
    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .filter(|(key, _)| key == "cookie")
            .flat_map(|(_, value)| value.split(';'))
            .filter_map(|pair| pair.trim().split_once('='))
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.trim())
    }

    /// Whether the request is `HEAD`.
    pub fn is_head(&self) -> bool {
        self.method == "HEAD"
    }

    /// Whether the body is declared as JSON (`application/json`, any parameters).
    pub fn has_json_body(&self) -> bool {
        self.header("content-type").is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"))
        })
    }
}

/// Why a request could not be read.
#[derive(Debug)]
pub(crate) enum RequestError {
    /// The peer closed the connection before sending anything.
    Closed,
    /// The request did not arrive before the deadline.
    TimedOut,
    /// The request violates HTTP/1.1 syntax.
    Malformed(&'static str),
    /// The request line and headers exceed the limit.
    HeadTooLarge,
    /// The declared body exceeds the limit.
    BodyTooLarge { limit: usize },
    /// The request uses `Transfer-Encoding`, which this server does not decode.
    UnsupportedTransferEncoding,
    /// The socket failed.
    Io(io::Error),
}

impl RequestError {
    /// Status code to answer with, or `None` when no response can be sent.
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Closed | Self::Io(_) => None,
            Self::TimedOut => Some(408),
            Self::Malformed(_) => Some(400),
            Self::HeadTooLarge => Some(431),
            Self::BodyTooLarge { .. } => Some(413),
            Self::UnsupportedTransferEncoding => Some(501),
        }
    }

    /// Client-facing description.
    pub fn message(&self) -> String {
        match self {
            Self::Closed => "connection closed".to_string(),
            Self::TimedOut => "the request was not received in time".to_string(),
            Self::Malformed(reason) => format!("malformed request: {}", reason),
            Self::HeadTooLarge => "request headers are too large".to_string(),
            Self::BodyTooLarge { limit } => {
                format!("request body exceeds the limit of {} bytes", limit)
            }
            Self::UnsupportedTransferEncoding => {
                "Transfer-Encoding is not supported; send a Content-Length body".to_string()
            }
            Self::Io(err) => format!("socket error: {}", err),
        }
    }
}

/// Reads and parses one request from `stream`.
pub(crate) fn read_request(
    stream: &mut TcpStream,
    limits: &RequestLimits,
) -> Result<Request, RequestError> {
    let deadline = Instant::now() + limits.read_timeout;
    let mut buffer: Vec<u8> = Vec::with_capacity(4096);
    let mut chunk = [0u8; 8192];

    let head_end = loop {
        if let Some(end) = find_head_end(&buffer) {
            if end > limits.max_head_bytes {
                return Err(RequestError::HeadTooLarge);
            }
            break end;
        }
        if buffer.len() > limits.max_head_bytes {
            return Err(RequestError::HeadTooLarge);
        }
        let read = read_some(stream, &mut chunk, deadline)?;
        if read == 0 {
            return Err(if buffer.is_empty() {
                RequestError::Closed
            } else {
                RequestError::Malformed("connection closed inside the request head")
            });
        }
        buffer.extend_from_slice(&chunk[..read]);
    };

    let head = std::str::from_utf8(&buffer[..head_end - 4])
        .map_err(|_| RequestError::Malformed("the request head is not valid UTF-8"))?;
    let (method, target, headers) = parse_head(head)?;
    let (path, query) = parse_target(&target)?;

    if headers.iter().any(|(name, _)| name == "transfer-encoding") {
        return Err(RequestError::UnsupportedTransferEncoding);
    }
    let content_length = content_length(&headers)?;
    if content_length > limits.max_body_bytes as u64 {
        return Err(RequestError::BodyTooLarge {
            limit: limits.max_body_bytes,
        });
    }
    let content_length = content_length as usize;

    let mut body = buffer[head_end..].to_vec();
    let expects_continue = headers
        .iter()
        .any(|(name, value)| name == "expect" && value.eq_ignore_ascii_case("100-continue"));
    if body.len() < content_length && expects_continue {
        stream
            .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
            .map_err(RequestError::Io)?;
    }
    while body.len() < content_length {
        let read = read_some(stream, &mut chunk, deadline)?;
        if read == 0 {
            return Err(RequestError::Malformed(
                "the body is shorter than its Content-Length",
            ));
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);

    Ok(Request {
        method,
        path,
        query,
        headers,
        body,
    })
}

fn find_head_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}

fn read_some(
    stream: &mut TcpStream,
    chunk: &mut [u8],
    deadline: Instant,
) -> Result<usize, RequestError> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(RequestError::TimedOut);
        }
        stream
            .set_read_timeout(Some(remaining.max(Duration::from_millis(1))))
            .map_err(RequestError::Io)?;
        match stream.read(chunk) {
            Ok(read) => return Ok(read),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err)
                if err.kind() == io::ErrorKind::WouldBlock
                    || err.kind() == io::ErrorKind::TimedOut =>
            {
                return Err(RequestError::TimedOut)
            }
            Err(err) => return Err(RequestError::Io(err)),
        }
    }
}

type ParsedHead = (String, String, Vec<(String, String)>);

fn parse_head(head: &str) -> Result<ParsedHead, RequestError> {
    let mut lines = head.split("\r\n");
    let request_line = lines
        .next()
        .ok_or(RequestError::Malformed("missing request line"))?;
    let mut parts = request_line.split(' ');
    let (method, target, version) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(method), Some(target), Some(version), None) => (method, target, version),
        _ => return Err(RequestError::Malformed("invalid request line")),
    };
    if method.is_empty() || !method.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(RequestError::Malformed("invalid method"));
    }
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return Err(RequestError::Malformed("unsupported HTTP version"));
    }

    let mut headers = Vec::new();
    for line in lines {
        if line.starts_with(' ') || line.starts_with('\t') {
            return Err(RequestError::Malformed("obsolete header line folding"));
        }
        let (name, value) = line
            .split_once(':')
            .ok_or(RequestError::Malformed("header line without a colon"))?;
        if name.is_empty() || name.bytes().any(|byte| !is_token_byte(byte)) {
            return Err(RequestError::Malformed("invalid header name"));
        }
        if headers.len() >= MAX_HEADER_COUNT {
            return Err(RequestError::HeadTooLarge);
        }
        headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
    }
    Ok((method.to_string(), target.to_string(), headers))
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

fn content_length(headers: &[(String, String)]) -> Result<u64, RequestError> {
    let mut length: Option<u64> = None;
    for (_, value) in headers.iter().filter(|(name, _)| name == "content-length") {
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(RequestError::Malformed("invalid Content-Length"));
        }
        let parsed: u64 = value
            .parse()
            .map_err(|_| RequestError::Malformed("invalid Content-Length"))?;
        if length.is_some_and(|previous| previous != parsed) {
            return Err(RequestError::Malformed("conflicting Content-Length headers"));
        }
        length = Some(parsed);
    }
    Ok(length.unwrap_or(0))
}

type ParsedTarget = (String, Vec<(String, String)>);

fn parse_target(target: &str) -> Result<ParsedTarget, RequestError> {
    if target == "*" {
        return Ok(("*".to_string(), Vec::new()));
    }
    let origin_form = if target.starts_with('/') {
        target
    } else if let Some(rest) = target
        .strip_prefix("http://")
        .or_else(|| target.strip_prefix("https://"))
    {
        rest.find('/').map(|slash| &rest[slash..]).unwrap_or("/")
    } else {
        return Err(RequestError::Malformed("unsupported request target"));
    };
    let without_fragment = origin_form.split('#').next().unwrap_or(origin_form);
    let (raw_path, raw_query) = match without_fragment.split_once('?') {
        Some((path, query)) => (path, query),
        None => (without_fragment, ""),
    };
    let path = percent_decode(raw_path, false)
        .filter(|decoded| !decoded.chars().any(char::is_control))
        .ok_or(RequestError::Malformed("invalid percent-encoding in the path"))?;
    let query = parse_query(raw_query)?;
    Ok((path, query))
}

fn parse_query(raw: &str) -> Result<Vec<(String, String)>, RequestError> {
    let mut pairs = Vec::new();
    for pair in raw.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let decode = |text: &str| {
            percent_decode(text, true)
                .ok_or(RequestError::Malformed("invalid percent-encoding in the query"))
        };
        pairs.push((decode(key)?, decode(value)?));
    }
    Ok(pairs)
}

/// Decodes `%XX` escapes (and `+` as space when `plus_as_space`), rejecting invalid
/// escapes, non-UTF-8 results and NUL bytes.
pub(crate) fn percent_decode(input: &str, plus_as_space: bool) -> Option<String> {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                let high = hex_value(*bytes.get(index + 1)?)?;
                let low = hex_value(*bytes.get(index + 2)?)?;
                decoded.push((high << 4) | low);
                index += 3;
            }
            b'+' if plus_as_space => {
                decoded.push(b' ');
                index += 1;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    let text = String::from_utf8(decoded).ok()?;
    if text.contains('\0') {
        return None;
    }
    Some(text)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Percent-encodes a decoded path for use in a `Location` header, keeping `/` and the
/// RFC 3986 unreserved characters.
pub(crate) fn percent_encode_path(path: &str) -> String {
    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{:02X}", byte));
        }
    }
    encoded
}

// ── Responses ────────────────────────────────────────────────────────────────

/// Which Content-Security-Policy a response carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContentPolicy {
    /// Data responses: nothing may load or frame them.
    Data,
    /// The web studio's HTML documents.
    StudioDocument,
}

const DATA_CSP: &str = "default-src 'none'; frame-ancestors 'none'; base-uri 'none'; \
                        form-action 'none'";

const STUDIO_CSP: &str = "default-src 'self'; \
     script-src 'self' 'unsafe-inline' https://cdnjs.cloudflare.com https://cdn.jsdelivr.net; \
     style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; \
     font-src 'self' data: https://fonts.gstatic.com; \
     img-src 'self' data: blob: https:; \
     connect-src 'self' https://api.github.com https://raw.githubusercontent.com \
     https://codeload.github.com https://github.com https://kroki.io; \
     worker-src 'self' blob:; object-src 'none'; base-uri 'self'; form-action 'self'; \
     frame-ancestors 'self'";

/// An HTTP response under construction.
#[derive(Debug, Clone)]
pub(crate) struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    policy: ContentPolicy,
}

impl Response {
    /// A response with an explicit content type and body.
    pub fn new(status: u16, content_type: &str, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: vec![("Content-Type".to_string(), content_type.to_string())],
            body,
            policy: ContentPolicy::Data,
        }
    }

    /// A JSON response; serialization failure becomes a 500.
    pub fn json<T: Serialize>(status: u16, value: &T) -> Self {
        match serde_json::to_vec(value) {
            Ok(body) => Self::new(status, "application/json; charset=utf-8", body)
                .header("Cache-Control", "no-store"),
            Err(_) => Self::new(
                500,
                "application/json; charset=utf-8",
                br#"{"status":"error","error":"response serialization failed"}"#.to_vec(),
            ),
        }
    }

    /// A JSON error body `{"status":"error","error":message}`.
    pub fn error(status: u16, message: &str) -> Self {
        Self::json(
            status,
            &serde_json::json!({ "status": "error", "error": message }),
        )
    }

    /// A response without a body.
    pub fn empty(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
            policy: ContentPolicy::Data,
        }
    }

    /// Switches to the studio document Content-Security-Policy.
    pub fn studio_document(mut self) -> Self {
        self.policy = ContentPolicy::StudioDocument;
        self
    }

    /// Adds a header; CR and LF are stripped so values can never split the head.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        let clean: String = value.chars().filter(|c| *c != '\r' && *c != '\n').collect();
        self.headers.push((name.to_string(), clean));
        self
    }

    /// Adds several headers.
    pub fn headers(mut self, extra: Vec<(String, String)>) -> Self {
        for (name, value) in extra {
            self = self.header(&name, &value);
        }
        self
    }

    /// The status code.
    pub fn status(&self) -> u16 {
        self.status
    }
}

/// Canonical reason phrase for `status`.
pub(crate) fn reason_phrase(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        303 => "See Other",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Unknown",
    }
}

/// Serializes `response`; the body is omitted when `include_body` is false (HEAD) and for
/// status codes that never carry one.
pub(crate) fn write_response(
    stream: &mut TcpStream,
    response: &Response,
    include_body: bool,
) -> io::Result<()> {
    let bodyless_status = response.status == 204 || response.status == 304;
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nDate: {}\r\nServer: OpenHeart\r\nConnection: close\r\n",
        response.status,
        reason_phrase(response.status),
        http_date(SystemTime::now())
    );
    if !bodyless_status {
        head.push_str(&format!("Content-Length: {}\r\n", response.body.len()));
    }
    let csp = match response.policy {
        ContentPolicy::Data => DATA_CSP,
        ContentPolicy::StudioDocument => STUDIO_CSP,
    };
    head.push_str(&format!("Content-Security-Policy: {}\r\n", csp));
    head.push_str("X-Content-Type-Options: nosniff\r\n");
    head.push_str("X-Frame-Options: SAMEORIGIN\r\n");
    head.push_str("Referrer-Policy: no-referrer\r\n");
    head.push_str("Cross-Origin-Resource-Policy: same-origin\r\n");
    for (name, value) in &response.headers {
        head.push_str(&format!("{}: {}\r\n", name, value));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    if include_body && !bodyless_status {
        stream.write_all(&response.body)?;
    }
    stream.flush()
}

const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Formats `time` as an RFC 9110 IMF-fixdate (`Sun, 06 Nov 1994 08:49:37 GMT`).
pub(crate) fn http_date(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let of_day = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        WEEKDAYS[(days % 7) as usize],
        day,
        MONTHS[(month - 1) as usize],
        year,
        of_day / 3600,
        (of_day % 3600) / 60,
        of_day % 60
    )
}

/// Converts days since 1970-01-01 to a proleptic Gregorian (year, month, day).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}
