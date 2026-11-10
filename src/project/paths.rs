//! Project-relative path helpers (forward slashes, no leading `./`, no trailing `/`).

/// Normalizes separators and resolves `.` and `..` segments. A leading `/` is kept (an
/// absolute path, such as a path inside a container image).
pub fn normalize(path: &str) -> String {
    let unified = path.replace('\\', "/");
    let absolute = unified.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for seg in unified.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.last().map(|p| *p != "..").unwrap_or(false) {
                    parts.pop();
                } else if !absolute {
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{}", joined)
    } else {
        joined
    }
}

/// Directory of a path (`""` for a top-level file).
pub fn parent(path: &str) -> String {
    let p = normalize(path);
    match p.rfind('/') {
        Some(0) => "/".to_string(),
        Some(i) => p[..i].to_string(),
        None => String::new(),
    }
}

/// Last segment of a path.
pub fn file_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches(|c| c == '/' || c == '\\');
    match trimmed.rfind(|c| c == '/' || c == '\\') {
        Some(i) => &trimmed[i + 1..],
        None => trimmed,
    }
}

/// `rel` resolved against `dir`; an absolute `rel` is returned normalized.
pub fn join(dir: &str, rel: &str) -> String {
    let rel_unified = rel.replace('\\', "/");
    if rel_unified.starts_with('/') || dir.is_empty() {
        return normalize(&rel_unified);
    }
    normalize(&format!("{}/{}", dir, rel_unified))
}

/// True when `path` is `dir` or lies below it (`dir == ""` contains every relative path).
pub fn is_under(path: &str, dir: &str) -> bool {
    if dir.is_empty() {
        return !path.starts_with('/');
    }
    path == dir || (path.starts_with(dir) && path[dir.len()..].starts_with('/'))
}

/// `path` relative to `dir` (`None` when it does not lie below it).
pub fn relative_to<'p>(path: &'p str, dir: &str) -> Option<&'p str> {
    if dir.is_empty() {
        return Some(path);
    }
    if path == dir {
        return Some("");
    }
    if is_under(path, dir) {
        return Some(&path[dir.len() + 1..]);
    }
    None
}

/// Segment-wise glob match: `*` matches within one segment, `**` matches any number of
/// segments, `?` one character.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let pnorm = normalize(pattern);
    let pat: Vec<&str> = pnorm.split('/').filter(|s| !s.is_empty()).collect();
    let norm = normalize(path);
    let segs: Vec<&str> = norm.split('/').filter(|s| !s.is_empty()).collect();
    match_segments(&pat, &segs)
}

fn match_segments(pat: &[&str], segs: &[&str]) -> bool {
    match pat.split_first() {
        None => segs.is_empty(),
        Some((&"**", rest)) => (0..=segs.len()).any(|k| match_segments(rest, &segs[k..])),
        Some((p, rest)) => match segs.split_first() {
            Some((s, srest)) => match_segment(p.as_bytes(), s.as_bytes()) && match_segments(rest, srest),
            None => false,
        },
    }
}

fn match_segment(p: &[u8], s: &[u8]) -> bool {
    match p.split_first() {
        None => s.is_empty(),
        Some((&b'*', rest)) => (0..=s.len()).any(|k| match_segment(rest, &s[k..])),
        Some((&b'?', rest)) => !s.is_empty() && match_segment(rest, &s[1..]),
        Some((c, rest)) => s.first() == Some(c) && match_segment(rest, &s[1..]),
    }
}

/// True when a path contains a glob metacharacter.
pub fn has_glob(path: &str) -> bool {
    path.contains('*') || path.contains('?') || path.contains('[')
}

/// The longest leading part of a path without glob metacharacters, cut at a segment
/// boundary.
pub fn literal_prefix(path: &str) -> String {
    let norm = normalize(path);
    let mut out: Vec<&str> = Vec::new();
    for seg in norm.split('/') {
        if has_glob(seg) {
            break;
        }
        out.push(seg);
    }
    out.join("/")
}

/// 1-based line of a byte offset.
pub fn line_of(text: &str, offset: usize) -> u32 {
    let end = offset.min(text.len());
    text.as_bytes()[..end].iter().filter(|b| **b == b'\n').count() as u32 + 1
}
