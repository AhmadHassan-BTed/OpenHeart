//! Web Repository Adapter Module
//!
//! Validates GitHub repository URLs and fetches them into the server workspace without
//! coupling to core SCPG graph structures.
//!
//! Only `https://github.com/<owner>/<repo>` (optionally `/tree/<ref>/<path>`) is accepted;
//! URLs carrying credentials are refused rather than rewritten. Clones run `git` in an
//! isolated credential context: no system or user configuration is read (so no credential
//! helper, `insteadOf` rewrite or extra HTTP header of the operator's account applies),
//! every transport except anonymous HTTPS is disabled, prompts and askpass programs are
//! off, no template hooks are installed, symlinks are checked out as plain files, the URL
//! is passed after `--`, and a wall-clock and a size limit are enforced. A clone lands in a
//! scratch directory that is renamed into place only when complete. Cached checkouts are
//! reused only after their `origin` URL is verified, are leased while an analysis or a
//! session reads them, re-cloned once older than the configured age, and evicted least
//! recently used first beyond the configured count and size.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use thiserror::Error;

use crate::adapters::runner::{canonicalize_clean, unique_suffix};
use crate::core::logger::{log_info, log_warn};

/// A validated reference to a GitHub repository, optionally narrowed to a ref and path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubRepoRef {
    /// Account or organization name.
    pub owner: String,
    /// Repository name, without `.git`.
    pub repo: String,
    /// Branch or tag from a `/tree/<ref>` URL or an explicit request field.
    pub git_ref: Option<String>,
    /// Sub-directory from a `/tree/<ref>/<path>` URL, `/`-separated.
    pub subpath: Option<String>,
}

impl GitHubRepoRef {
    /// `https://github.com/<owner>/<repo>`.
    pub fn canonical_url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.repo)
    }

    /// `https://github.com/<owner>/<repo>.git`, the URL handed to `git clone`.
    pub fn clone_url(&self) -> String {
        format!("{}.git", self.canonical_url())
    }

    /// Applies an explicitly requested branch or tag. It must agree with a ref already
    /// taken from a `/tree/<ref>` URL.
    pub fn with_ref(mut self, git_ref: Option<&str>) -> Result<Self, String> {
        let Some(requested) = git_ref.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(self);
        };
        validate_ref(requested)?;
        match &self.git_ref {
            Some(existing) if existing != requested => Err(format!(
                "the URL names ref '{}' but the request asks for '{}'",
                existing, requested
            )),
            _ => {
                self.git_ref = Some(requested.to_string());
                Ok(self)
            }
        }
    }

    /// Workspace-relative cache directory: `<owner>/<repo>/<ref-key>`, lower-cased because
    /// GitHub names are case-insensitive.
    fn cache_dir(&self, repos_dir: &Path) -> PathBuf {
        let ref_key = match &self.git_ref {
            None => "default".to_string(),
            Some(git_ref) => {
                let sanitized: String = git_ref
                    .chars()
                    .take(64)
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                format!("ref-{}-{:08x}", sanitized, fnv1a32(git_ref.as_bytes()))
            }
        };
        repos_dir
            .join(safe_component(&self.owner.to_ascii_lowercase()))
            .join(safe_component(&self.repo.to_ascii_lowercase()))
            .join(ref_key)
    }
}

fn fnv1a32(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811C_9DC5u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    })
}

/// Appends `_` to names Windows reserves for devices, so cache paths are portable.
fn safe_component(name: &str) -> String {
    const RESERVED: [&str; 22] = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    let stem = name.split('.').next().unwrap_or(name);
    if RESERVED.contains(&stem) {
        format!("{}_", name)
    } else {
        name.to_string()
    }
}

/// Decoupled Web Repository Fetcher and SCPG Adapter.
pub struct WebRepoFetcher;

/// Directory (inside the repository cache) that git uses as its home and global
/// configuration, so nothing of the operator's account configuration is read.
const SANDBOX_DIR: &str = ".git-sandbox";
/// Empty global configuration file inside [`SANDBOX_DIR`].
const SANDBOX_CONFIG: &str = "gitconfig";
/// Prefix of in-progress clone directories.
const PARTIAL_PREFIX: &str = ".partial-";
/// Extension of the per-checkout metadata file kept next to each checkout.
const META_EXTENSION: &str = "meta";

/// Leases held on cached checkouts, keyed by checkout directory.
#[derive(Debug, Clone, Default)]
struct LeaseTable {
    counts: Arc<Mutex<HashMap<PathBuf, usize>>>,
}

impl LeaseTable {
    fn lock(&self) -> MutexGuard<'_, HashMap<PathBuf, usize>> {
        self.counts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Keeps a cached checkout from being refreshed or evicted while it is read. Held by the
/// analysis that checked it out and, afterwards, by the session that serves its sources.
pub struct CheckoutLease {
    dir: PathBuf,
    table: LeaseTable,
}

impl fmt::Debug for CheckoutLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CheckoutLease")
            .field("dir", &self.dir)
            .finish()
    }
}

impl Drop for CheckoutLease {
    fn drop(&mut self) {
        let mut counts = self.table.lock();
        if let Some(count) = counts.get_mut(&self.dir) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                counts.remove(&self.dir);
            }
        }
    }
}

/// Where and how repositories are fetched and how long they are cached.
#[derive(Debug, Clone)]
pub struct FetchOptions {
    /// Directory holding cached checkouts.
    pub repos_dir: PathBuf,
    /// Whether missing repositories may be cloned.
    pub allow_clone: bool,
    /// Wall-clock limit for one clone.
    pub clone_timeout: Duration,
    /// Maximum on-disk size of one clone.
    pub max_repo_bytes: u64,
    /// Cached checkouts kept; the least recently used beyond this are removed.
    pub max_cached_repos: usize,
    /// Combined on-disk size of the cached checkouts kept.
    pub max_cache_bytes: u64,
    /// Checkouts cloned longer ago than this are re-cloned on their next use (when cloning
    /// is allowed); `None` reuses them indefinitely.
    pub max_checkout_age: Option<Duration>,
    leases: LeaseTable,
}

impl FetchOptions {
    /// Options with default cache limits (32 checkouts, 8 GiB, no age limit).
    pub fn new(
        repos_dir: PathBuf,
        allow_clone: bool,
        clone_timeout: Duration,
        max_repo_bytes: u64,
    ) -> Self {
        Self {
            repos_dir,
            allow_clone,
            clone_timeout,
            max_repo_bytes,
            max_cached_repos: 32,
            max_cache_bytes: 8 * 1024 * 1024 * 1024,
            max_checkout_age: None,
            leases: LeaseTable::default(),
        }
    }

    /// Replaces the cache limits.
    pub fn with_cache_limits(
        mut self,
        max_cached_repos: usize,
        max_cache_bytes: u64,
        max_checkout_age: Option<Duration>,
    ) -> Self {
        self.max_cached_repos = max_cached_repos.max(1);
        self.max_cache_bytes = max_cache_bytes.max(1);
        self.max_checkout_age = max_checkout_age;
        self
    }

    /// Creates the cache directory and the git sandbox, and removes clones left half-done
    /// by an earlier process.
    pub fn prepare(&self) -> Result<(), FetchError> {
        fs::create_dir_all(&self.repos_dir).map_err(io_failure("create the repository cache"))?;
        let sandbox = self.sandbox_dir();
        fs::create_dir_all(&sandbox).map_err(io_failure("create the git sandbox"))?;
        let config = sandbox.join(SANDBOX_CONFIG);
        fs::write(&config, b"").map_err(io_failure("write the git sandbox configuration"))?;
        let entries = fs::read_dir(&self.repos_dir).map_err(io_failure("list the cache"))?;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(PARTIAL_PREFIX) {
                remove_quietly(&entry.path());
            }
        }
        Ok(())
    }

    fn sandbox_dir(&self) -> PathBuf {
        self.repos_dir.join(SANDBOX_DIR)
    }

    /// Number of checkouts currently leased.
    pub fn leased_checkouts(&self) -> usize {
        self.leases.lock().len()
    }
}

/// A local checkout ready for analysis. Its lease keeps it in place until dropped.
#[derive(Debug)]
pub struct CheckedOutRepo {
    /// Canonical checkout directory.
    pub dir: PathBuf,
    /// `HEAD` commit, or `None` if git could not report it.
    pub commit: Option<String>,
    /// Whether an existing cached checkout was reused.
    pub reused_cache: bool,
    /// When the checkout was cloned, if recorded.
    pub cloned_unix_secs: Option<u64>,
    /// Lease on the cache directory.
    pub lease: CheckoutLease,
}

/// Why a repository could not be made available. `Display` is safe to show to clients;
/// [`FetchError::detail`] carries server-side diagnostics.
#[derive(Debug, Error)]
pub enum FetchError {
    /// Cloning is disabled and no cached checkout exists.
    #[error(
        "server-side cloning is disabled; only repositories already present in the \
         workspace can be analyzed"
    )]
    CloneDisabled,
    /// The `git` executable could not be started.
    #[error("git is not available on the server")]
    GitUnavailable(#[source] io::Error),
    /// `git clone` exited unsuccessfully.
    #[error("git clone failed; the repository or ref may be private, missing or unreachable")]
    CloneFailed {
        /// Exit status and captured stderr, for server logs only.
        detail: String,
    },
    /// The clone did not finish in time.
    #[error("git clone did not finish within {0} seconds")]
    TimedOut(u64),
    /// The clone grew beyond the size limit.
    #[error("the repository exceeds the size limit of {limit_mb} MiB")]
    TooLarge {
        /// The limit in MiB.
        limit_mb: u64,
    },
    /// A cached checkout exists but its origin does not match the request.
    #[error("the cached checkout does not belong to the requested repository")]
    CacheMismatch {
        /// What was found, for server logs only.
        detail: String,
    },
    /// A refresh was requested while another analysis or session reads the checkout.
    #[error(
        "the cached checkout is in use by another analysis or session; retry the refresh \
         once it is released"
    )]
    Busy,
    /// The requested sub-directory does not exist in the checkout.
    #[error("the path '{0}' does not exist in the repository")]
    SubpathMissing(String),
    /// A workspace filesystem operation failed.
    #[error("workspace I/O failure while trying to {action}")]
    Io {
        /// What was being attempted.
        action: &'static str,
        /// The underlying error.
        source: io::Error,
    },
}

impl FetchError {
    /// HTTP status that best describes the failure.
    pub fn http_status(&self) -> u16 {
        match self {
            Self::CloneDisabled => 403,
            Self::GitUnavailable(_) | Self::Io { .. } => 500,
            Self::CloneFailed { .. } => 502,
            Self::TimedOut(_) => 504,
            Self::TooLarge { .. } => 422,
            Self::CacheMismatch { .. } | Self::Busy => 409,
            Self::SubpathMissing(_) => 404,
        }
    }

    /// Server-side diagnostic text (may contain paths and git output).
    pub fn detail(&self) -> String {
        match self {
            Self::GitUnavailable(err) => format!("cannot start git: {}", err),
            Self::CloneFailed { detail } | Self::CacheMismatch { detail } => detail.clone(),
            Self::Io { action, source } => format!("{}: {}", action, source),
            other => other.to_string(),
        }
    }
}

fn io_failure(action: &'static str) -> impl FnOnce(io::Error) -> FetchError {
    move |source| FetchError::Io { action, source }
}

impl WebRepoFetcher {
    /// Validates a remote GitHub repository URL and returns `(owner, repo)`.
    pub fn validate_url(url: &str) -> Result<(String, String), String> {
        Self::parse_repo_url(url).map(|repo| (repo.owner, repo.repo))
    }

    /// Parses `https://github.com/<owner>/<repo>[.git][/]` or
    /// `https://github.com/<owner>/<repo>/tree/<ref>[/<path>]`. Error messages never echo
    /// the URL, which may carry secrets.
    pub fn parse_repo_url(url: &str) -> Result<GitHubRepoRef, String> {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return Err("the repository URL is empty".to_string());
        }
        if trimmed.len() > 2048 {
            return Err("the repository URL is too long".to_string());
        }
        if trimmed.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err("the repository URL contains whitespace or control characters".into());
        }
        if has_userinfo(trimmed) {
            return Err(
                "repository URLs with embedded credentials are not accepted; only public \
                 repositories are cloned, anonymously"
                    .to_string(),
            );
        }
        let lower = trimmed.to_ascii_lowercase();
        let prefix_len = ["https://github.com/", "https://www.github.com/"]
            .iter()
            .find(|prefix| lower.starts_with(*prefix))
            .map(|prefix| prefix.len())
            .ok_or_else(|| {
                "only https://github.com/<owner>/<repo> URLs are accepted".to_string()
            })?;
        let path = trimmed[prefix_len..]
            .split(['?', '#'])
            .next()
            .unwrap_or_default();
        let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();

        let owner = segments
            .first()
            .copied()
            .ok_or_else(|| "the URL has no repository owner".to_string())?;
        let raw_repo = segments
            .get(1)
            .copied()
            .ok_or_else(|| "the URL has no repository name".to_string())?;
        let repo = raw_repo.strip_suffix(".git").unwrap_or(raw_repo);
        validate_owner(owner)?;
        validate_repo(repo)?;

        let (git_ref, subpath) = match segments.get(2).copied() {
            None => (None, None),
            Some("tree") => {
                let git_ref = segments
                    .get(3)
                    .copied()
                    .ok_or_else(|| "a /tree/ URL must name a branch or tag".to_string())?;
                validate_ref(git_ref)?;
                let rest = &segments[4..];
                for segment in rest {
                    validate_path_segment(segment)?;
                }
                let subpath = if rest.is_empty() {
                    None
                } else {
                    Some(rest.join("/"))
                };
                (Some(git_ref.to_string()), subpath)
            }
            Some(_) => {
                return Err(
                    "unsupported GitHub URL; use https://github.com/<owner>/<repo> or \
                     https://github.com/<owner>/<repo>/tree/<ref>/<path>"
                        .to_string(),
                )
            }
        };

        Ok(GitHubRepoRef {
            owner: owner.to_string(),
            repo: repo.to_string(),
            git_ref,
            subpath,
        })
    }

    /// Makes `repo` available under `options.repos_dir` and leases it. A verified cached
    /// checkout is reused unless `refresh` is set or it is older than the configured age;
    /// otherwise it is cloned when allowed. After a clone, the cache is pruned to its
    /// limits (leased checkouts are never removed).
    pub fn checkout(
        repo: &GitHubRepoRef,
        options: &FetchOptions,
        refresh: bool,
    ) -> Result<CheckedOutRepo, FetchError> {
        let dir = repo.cache_dir(&options.repos_dir);
        let meta_path = meta_path(&dir);
        let lease = {
            let mut counts = options.leases.lock();
            let in_use = counts.get(&dir).copied().unwrap_or(0) > 0;
            if dir.exists() {
                let expired = options
                    .max_checkout_age
                    .is_some_and(|age| checkout_age(&meta_path).is_some_and(|actual| actual > age));
                if refresh && !options.allow_clone {
                    return Err(FetchError::CloneDisabled);
                }
                if refresh || (expired && options.allow_clone) {
                    if in_use {
                        return Err(FetchError::Busy);
                    }
                    log_info(&format!(
                        "[REPO] Removing cached checkout of {} ({})",
                        repo.canonical_url(),
                        if refresh {
                            "refresh requested"
                        } else {
                            "older than the age limit"
                        }
                    ));
                    fs::remove_dir_all(&dir).map_err(io_failure("remove the cached checkout"))?;
                    remove_file_quietly(&meta_path);
                }
            }
            *counts.entry(dir.clone()).or_insert(0) += 1;
            CheckoutLease {
                dir: dir.clone(),
                table: options.leases.clone(),
            }
        };

        if dir.exists() {
            verify_origin(&dir, repo, options)?;
            touch(&meta_path);
            return Ok(CheckedOutRepo {
                dir: canonicalize_clean(&dir).map_err(io_failure("resolve the checkout"))?,
                commit: head_commit(&dir, options),
                reused_cache: true,
                cloned_unix_secs: read_meta(&meta_path),
                lease,
            });
        }
        if !options.allow_clone {
            return Err(FetchError::CloneDisabled);
        }

        let parent = dir.parent().unwrap_or(&options.repos_dir);
        fs::create_dir_all(parent).map_err(io_failure("create the checkout directory"))?;
        let scratch = options
            .repos_dir
            .join(format!("{}{}", PARTIAL_PREFIX, unique_suffix()));
        if let Err(err) = clone_into(repo, &scratch, options) {
            remove_quietly(&scratch);
            return Err(err);
        }
        let mut reused_cache = false;
        if let Err(err) = fs::rename(&scratch, &dir) {
            remove_quietly(&scratch);
            // A concurrent request may have completed the same clone first.
            if !dir.exists() {
                return Err(io_failure("move the clone into place")(err));
            }
            verify_origin(&dir, repo, options)?;
            reused_cache = true;
        } else {
            write_meta(&meta_path, now_unix_secs());
        }
        prune_cache(options, &dir);
        Ok(CheckedOutRepo {
            dir: canonicalize_clean(&dir).map_err(io_failure("resolve the checkout"))?,
            commit: head_commit(&dir, options),
            reused_cache,
            cloned_unix_secs: read_meta(&meta_path),
            lease,
        })
    }

    /// Resolves `subpath` inside `checkout`, refusing anything that escapes it.
    pub fn resolve_subpath(checkout: &Path, subpath: Option<&str>) -> Result<PathBuf, FetchError> {
        let Some(subpath) = subpath else {
            return Ok(checkout.to_path_buf());
        };
        let missing = || FetchError::SubpathMissing(subpath.to_string());
        let resolved = canonicalize_clean(&checkout.join(subpath)).map_err(|_| missing())?;
        let inside_git_dir = resolved.starts_with(checkout.join(".git"));
        if !resolved.starts_with(checkout) || inside_git_dir || !resolved.is_dir() {
            return Err(missing());
        }
        Ok(resolved)
    }

    /// The version reported by `git --version`, or `None` when git cannot be started.
    pub fn git_version() -> Option<String> {
        let output = Command::new("git")
            .arg("--version")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Some(
            text.strip_prefix("git version ")
                .map(str::to_string)
                .unwrap_or(text),
        )
    }
}

/// `scheme://user[:password]@host/...`: an `@` before the first `/` of the authority.
fn has_userinfo(url: &str) -> bool {
    let authority = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let authority = authority.split(['/', '?', '#']).next().unwrap_or_default();
    authority.contains('@')
}

fn validate_owner(owner: &str) -> Result<(), String> {
    let valid = !owner.is_empty()
        && owner.len() <= 39
        && owner
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && !owner.starts_with('-')
        && !owner.ends_with('-');
    if valid {
        Ok(())
    } else {
        Err(format!("'{}' is not a valid GitHub owner name", owner))
    }
}

fn validate_repo(repo: &str) -> Result<(), String> {
    let valid = !repo.is_empty()
        && repo.len() <= 100
        && repo
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && !repo.starts_with('-')
        && !repo.ends_with('.');
    if valid {
        Ok(())
    } else {
        Err(format!("'{}' is not a valid GitHub repository name", repo))
    }
}

/// Branch and tag names under git's ref-name rules (`git check-ref-format`), restricted to
/// a portable character set: `/`-separated components, none empty, none starting with `.`
/// or ending with `.lock`, no `..` or `@{`, and no leading `-`.
fn validate_ref(git_ref: &str) -> Result<(), String> {
    let valid = !git_ref.is_empty()
        && git_ref.len() <= 255
        && !git_ref.starts_with('-')
        && !git_ref.contains("..")
        && !git_ref.contains("@{")
        && git_ref
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.+/".contains(&byte))
        && git_ref.split('/').all(|component| {
            !component.is_empty() && !component.starts_with('.') && !component.ends_with(".lock")
        });
    if valid {
        Ok(())
    } else {
        Err(format!(
            "'{}' is not an accepted branch or tag name",
            git_ref
        ))
    }
}

fn validate_path_segment(segment: &str) -> Result<(), String> {
    const FORBIDDEN: [char; 8] = ['\\', ':', '*', '?', '"', '<', '>', '|'];
    let valid = segment != "."
        && segment != ".."
        && !segment.eq_ignore_ascii_case(".git")
        && segment
            .chars()
            .all(|c| !c.is_control() && !FORBIDDEN.contains(&c));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "'{}' is not an accepted repository path segment",
            segment
        ))
    }
}

/// Environment variables that could redirect git to another repository, configuration,
/// credential source or program; they are removed from every git invocation.
const GIT_ENV_REMOVED: [&str; 16] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_NAMESPACE",
    "GIT_ASKPASS",
    "SSH_ASKPASS",
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "GIT_PROXY_COMMAND",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_SYSTEM",
    "GIT_TEMPLATE_DIR",
];

/// A `git` invocation in the credential sandbox: system and user configuration are not
/// read, only anonymous HTTPS is allowed, and nothing can prompt.
fn git_command(options: &FetchOptions) -> Command {
    let sandbox = options.sandbox_dir();
    let mut command = Command::new("git");
    for name in GIT_ENV_REMOVED {
        command.env_remove(name);
    }
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ALLOW_PROTOCOL", "https")
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", sandbox.join(SANDBOX_CONFIG))
        .env("HOME", &sandbox)
        .env("XDG_CONFIG_HOME", &sandbox)
        .stdin(Stdio::null())
        .args([
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.https.allow=always",
            "-c",
            "credential.helper=",
            "-c",
            "core.askPass=",
            "-c",
            "http.extraHeader=",
            "-c",
            "core.symlinks=false",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "submodule.recurse=false",
        ]);
    command
}

fn clone_into(
    repo: &GitHubRepoRef,
    target: &Path,
    options: &FetchOptions,
) -> Result<(), FetchError> {
    let mut command = git_command(options);
    command.args([
        "clone",
        "--depth",
        "1",
        "--single-branch",
        "--no-tags",
        "--quiet",
        "--template=",
    ]);
    if let Some(git_ref) = &repo.git_ref {
        command.arg(format!("--branch={}", git_ref));
    }
    command
        .arg("--")
        .arg(repo.clone_url())
        .arg(target)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(FetchError::GitUnavailable)?;
    let stderr = child.stderr.take().map(spawn_capped_reader);
    let outcome = supervise(&mut child, target, options);
    let stderr_text = stderr
        .and_then(|reader| reader.join().ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_string())
        .unwrap_or_default();

    let status = outcome?;
    if !status.success() {
        return Err(FetchError::CloneFailed {
            detail: format!("git clone exited with {}: {}", status, stderr_text),
        });
    }
    if dir_size(target) > options.max_repo_bytes {
        return Err(FetchError::TooLarge {
            limit_mb: options.max_repo_bytes / (1024 * 1024),
        });
    }
    Ok(())
}

/// Waits for `child`, killing it when the time or size limit is exceeded.
fn supervise(
    child: &mut Child,
    target: &Path,
    options: &FetchOptions,
) -> Result<ExitStatus, FetchError> {
    let started = Instant::now();
    let mut last_size_check = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(io_failure("wait for git"))? {
            return Ok(status);
        }
        if started.elapsed() > options.clone_timeout {
            terminate(child);
            return Err(FetchError::TimedOut(options.clone_timeout.as_secs()));
        }
        if last_size_check.elapsed() >= Duration::from_secs(2) {
            last_size_check = Instant::now();
            if dir_size(target) > options.max_repo_bytes {
                terminate(child);
                return Err(FetchError::TooLarge {
                    limit_mb: options.max_repo_bytes / (1024 * 1024),
                });
            }
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn terminate(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Drains a pipe on its own thread, keeping at most 16 KiB, so git never blocks on a
/// full stderr pipe.
fn spawn_capped_reader<R: Read + Send + 'static>(mut pipe: R) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        const CAP: usize = 16 * 1024;
        let mut kept = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    let room = CAP.saturating_sub(kept.len());
                    kept.extend_from_slice(&chunk[..read.min(room)]);
                }
            }
        }
        kept
    })
}

/// Total size of regular files under `root`, without following symlinks.
fn dir_size(root: &Path) -> u64 {
    let mut total = 0u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            match entry.path().symlink_metadata() {
                Ok(meta) if meta.is_dir() => pending.push(entry.path()),
                Ok(meta) => total = total.saturating_add(meta.len()),
                Err(_) => {}
            }
        }
    }
    total
}

fn remove_quietly(path: &Path) {
    if path.exists() {
        if let Err(err) = fs::remove_dir_all(path) {
            log_warn(&format!(
                "[REPO] Could not remove '{}': {}",
                path.display(),
                err
            ));
        }
    }
}

fn remove_file_quietly(path: &Path) {
    if let Err(err) = fs::remove_file(path) {
        if err.kind() != io::ErrorKind::NotFound {
            log_warn(&format!(
                "[REPO] Could not remove '{}': {}",
                path.display(),
                err
            ));
        }
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// `<checkout>.meta`, next to the checkout so it is never part of the analyzed tree.
fn meta_path(dir: &Path) -> PathBuf {
    dir.with_extension(META_EXTENSION)
}

/// Records the clone time; the file's modification time is the last-use time.
fn write_meta(path: &Path, cloned_unix_secs: u64) {
    if let Err(err) = fs::write(path, format!("cloned_unix_secs={}\n", cloned_unix_secs)) {
        log_warn(&format!(
            "[REPO] Could not record checkout metadata '{}': {}",
            path.display(),
            err
        ));
    }
}

/// The recorded clone time, or `None` when no metadata was recorded.
fn read_meta(path: &Path) -> Option<u64> {
    let text = fs::read_to_string(path).ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix("cloned_unix_secs="))
        .and_then(|value| value.trim().parse().ok())
}

/// Age of a checkout from its recorded clone time (`None` when unrecorded).
fn checkout_age(meta: &Path) -> Option<Duration> {
    let cloned = read_meta(meta)?;
    Some(Duration::from_secs(now_unix_secs().saturating_sub(cloned)))
}

/// Marks a checkout as used now.
fn touch(meta: &Path) {
    let touched = fs::OpenOptions::new()
        .write(true)
        .open(meta)
        .and_then(|file| file.set_modified(SystemTime::now()));
    if let Err(err) = touched {
        if err.kind() != io::ErrorKind::NotFound {
            log_warn(&format!(
                "[REPO] Could not update the last-use time of '{}': {}",
                meta.display(),
                err
            ));
        }
    }
}

/// Last-use time of a checkout (`None` when no metadata was recorded).
fn last_used(meta: &Path) -> Option<SystemTime> {
    fs::metadata(meta).and_then(|meta| meta.modified()).ok()
}

/// Every cached checkout directory (`<owner>/<repo>/<ref-key>`).
fn cached_checkouts(repos_dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let visible_dirs = |dir: &Path| -> Vec<PathBuf> {
        fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                    .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
                    .map(|entry| entry.path())
                    .collect()
            })
            .unwrap_or_default()
    };
    for owner in visible_dirs(repos_dir) {
        for repo in visible_dirs(&owner) {
            found.extend(visible_dirs(&repo));
        }
    }
    found
}

/// Removes least-recently-used checkouts until the cache is within its count and size
/// limits. Leased checkouts and `keep` are never removed; checkouts without recorded
/// metadata are treated as the least recently used.
fn prune_cache(options: &FetchOptions, keep: &Path) {
    let mut entries: Vec<(Option<SystemTime>, PathBuf, u64)> = cached_checkouts(&options.repos_dir)
        .into_iter()
        .map(|dir| {
            let used = last_used(&meta_path(&dir));
            let size = dir_size(&dir);
            (used, dir, size)
        })
        .collect();
    entries.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let mut count = entries.len();
    let mut total: u64 = entries.iter().map(|(_, _, size)| *size).sum();
    for (_, dir, size) in entries {
        if count <= options.max_cached_repos && total <= options.max_cache_bytes {
            break;
        }
        if dir == keep {
            continue;
        }
        let counts = options.leases.lock();
        if counts.get(&dir).copied().unwrap_or(0) > 0 {
            continue;
        }
        match fs::remove_dir_all(&dir) {
            Ok(()) => {
                remove_file_quietly(&meta_path(&dir));
                count -= 1;
                total = total.saturating_sub(size);
                log_info(&format!(
                    "[REPO] Evicted cached checkout '{}'",
                    dir.display()
                ));
            }
            Err(err) => log_warn(&format!(
                "[REPO] Could not evict cached checkout '{}': {}",
                dir.display(),
                err
            )),
        }
        drop(counts);
        for ancestor in [dir.parent(), dir.parent().and_then(Path::parent)]
            .into_iter()
            .flatten()
        {
            // Removes the owner and repository directories once they are empty.
            let _ = fs::remove_dir(ancestor);
        }
    }
}

fn run_git(dir: &Path, options: &FetchOptions, args: &[&str]) -> Option<String> {
    let output = git_command(options)
        .arg("-C")
        .arg(dir)
        .args(args)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn head_commit(dir: &Path, options: &FetchOptions) -> Option<String> {
    run_git(dir, options, &["rev-parse", "HEAD"]).filter(|sha| {
        (sha.len() == 40 || sha.len() == 64) && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn normalize_remote(url: &str) -> String {
    let lower = url.trim().to_ascii_lowercase();
    let without_scheme = lower
        .strip_prefix("https://")
        .unwrap_or(&lower)
        .trim_start_matches("www.");
    let trimmed = without_scheme.trim_end_matches('/');
    trimmed.strip_suffix(".git").unwrap_or(trimmed).to_string()
}

fn verify_origin(
    dir: &Path,
    repo: &GitHubRepoRef,
    options: &FetchOptions,
) -> Result<(), FetchError> {
    if !dir.join(".git").is_dir() {
        return Err(FetchError::CacheMismatch {
            detail: format!("'{}' exists but is not a git checkout", dir.display()),
        });
    }
    let origin =
        run_git(dir, options, &["config", "--get", "remote.origin.url"]).ok_or_else(|| {
            FetchError::CacheMismatch {
                detail: format!("cannot read remote.origin.url of '{}'", dir.display()),
            }
        })?;
    if normalize_remote(&origin) != normalize_remote(&repo.canonical_url()) {
        return Err(FetchError::CacheMismatch {
            detail: format!(
                "'{}' tracks '{}', expected '{}'",
                dir.display(),
                origin,
                repo.canonical_url()
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_web_repo_url_validation() {
        let res = WebRepoFetcher::validate_url("https://github.com/AhmadHassan-BTed/OpenHeart");
        assert!(res.is_ok());
        let (owner, repo) = res.unwrap();
        assert_eq!(owner, "AhmadHassan-BTed");
        assert_eq!(repo, "OpenHeart");
    }
}
