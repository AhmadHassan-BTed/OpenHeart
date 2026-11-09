//! Web Repository Adapter Module
//!
//! Validates GitHub repository URLs and fetches them into the server workspace without
//! coupling to core SCPG graph structures.
//!
//! Only `https://github.com/<owner>/<repo>` (optionally `/tree/<ref>/<path>`) is accepted.
//! Clones run `git` with every transport except HTTPS disabled, no credential helpers or
//! prompts, symlinks checked out as plain files, the URL passed after `--`, a wall-clock
//! limit and a size limit, into a scratch directory that is renamed into place only after
//! a complete, verified clone. Cached checkouts are reused only after their `origin` URL
//! is verified to match the request.

#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::adapters::runner::{canonicalize_clean, unique_suffix};

/// Bitmask and configuration options for selecting any of the 14 UML diagram types.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UmlDiagramType {
    // Structural (7)
    /// Class diagram.
    ClassDiagram,
    /// Object diagram.
    ObjectDiagram,
    /// Component diagram.
    ComponentDiagram,
    /// Deployment diagram.
    DeploymentDiagram,
    /// Package diagram.
    PackageDiagram,
    /// Composite structure diagram.
    CompositeStructure,
    /// Profile diagram.
    ProfileDiagram,
    // Behavioral & Interaction (7)
    /// Use case diagram.
    UseCaseDiagram,
    /// Activity diagram.
    ActivityDiagram,
    /// State machine diagram.
    StateMachine,
    /// Sequence diagram.
    SequenceDiagram,
    /// Communication diagram.
    CommunicationDiagram,
    /// Interaction overview diagram.
    InteractionOverview,
    /// Timing diagram.
    TimingDiagram,
}

impl UmlDiagramType {
    /// All 14 diagram types.
    pub fn fill_all() -> HashSet<Self> {
        let mut set = HashSet::new();
        set.insert(Self::ClassDiagram);
        set.insert(Self::ObjectDiagram);
        set.insert(Self::ComponentDiagram);
        set.insert(Self::DeploymentDiagram);
        set.insert(Self::PackageDiagram);
        set.insert(Self::CompositeStructure);
        set.insert(Self::ProfileDiagram);
        set.insert(Self::UseCaseDiagram);
        set.insert(Self::ActivityDiagram);
        set.insert(Self::StateMachine);
        set.insert(Self::SequenceDiagram);
        set.insert(Self::CommunicationDiagram);
        set.insert(Self::InteractionOverview);
        set.insert(Self::TimingDiagram);
        set
    }

    /// Human-readable diagram name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::ClassDiagram => "Class Diagram",
            Self::ObjectDiagram => "Object Diagram",
            Self::ComponentDiagram => "Component Diagram",
            Self::DeploymentDiagram => "Deployment Diagram",
            Self::PackageDiagram => "Package Diagram",
            Self::CompositeStructure => "Composite Structure Diagram",
            Self::ProfileDiagram => "Profile Diagram",
            Self::UseCaseDiagram => "Use Case Diagram",
            Self::ActivityDiagram => "Activity Diagram",
            Self::StateMachine => "State Machine Diagram",
            Self::SequenceDiagram => "Sequence Diagram",
            Self::CommunicationDiagram => "Communication Diagram",
            Self::InteractionOverview => "Interaction Overview Diagram",
            Self::TimingDiagram => "Timing Diagram",
        }
    }

    /// Canonical diagram identifier used by the exporters and the HTTP API.
    pub fn identifier(&self) -> &'static str {
        match self {
            Self::ClassDiagram => "class",
            Self::ObjectDiagram => "object",
            Self::ComponentDiagram => "component",
            Self::DeploymentDiagram => "deployment",
            Self::PackageDiagram => "package",
            Self::CompositeStructure => "composite",
            Self::ProfileDiagram => "profile",
            Self::UseCaseDiagram => "usecase",
            Self::ActivityDiagram => "activity",
            Self::StateMachine => "statemachine",
            Self::SequenceDiagram => "sequence",
            Self::CommunicationDiagram => "communication",
            Self::InteractionOverview => "interaction",
            Self::TimingDiagram => "timing",
        }
    }
}

/// Web Repository Configuration for deployed portal processing.
#[derive(Debug, Clone)]
pub struct WebRepoConfig {
    /// Repository URL as submitted.
    pub repo_url: String,
    /// Branch or tag to check out.
    pub branch: String,
    /// Diagrams to produce.
    pub selected_diagrams: HashSet<UmlDiagramType>,
}

impl WebRepoConfig {
    /// Configuration selecting all 14 diagrams.
    pub fn new(repo_url: impl Into<String>, branch: impl Into<String>) -> Self {
        Self {
            repo_url: repo_url.into(),
            branch: branch.into(),
            selected_diagrams: UmlDiagramType::fill_all(),
        }
    }

    /// Replaces the diagram selection.
    pub fn with_diagrams(mut self, diagrams: HashSet<UmlDiagramType>) -> Self {
        self.selected_diagrams = diagrams;
        self
    }
}

/// A validated reference to a GitHub repository, optionally narrowed to a ref and path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubRepoRef {
    /// Account or organization name.
    pub owner: String,
    /// Repository name, without `.git`.
    pub repo: String,
    /// Branch or tag from a `/tree/<ref>` URL.
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
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
        "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
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

/// Where and how repositories are fetched.
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
}

/// A local checkout ready for analysis.
#[derive(Debug, Clone)]
pub struct CheckedOutRepo {
    /// Canonical checkout directory.
    pub dir: PathBuf,
    /// `HEAD` commit, or `None` if git could not report it.
    pub commit: Option<String>,
    /// Whether an existing cached checkout was reused.
    pub reused_cache: bool,
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
            Self::CacheMismatch { .. } => 409,
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
    /// `https://github.com/<owner>/<repo>/tree/<ref>[/<path>]`.
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

    /// Makes `repo` available under `options.repos_dir`, reusing a verified cached
    /// checkout unless `refresh` is set, and cloning when allowed.
    pub fn checkout(
        repo: &GitHubRepoRef,
        options: &FetchOptions,
        refresh: bool,
    ) -> Result<CheckedOutRepo, FetchError> {
        let dir = repo.cache_dir(&options.repos_dir);
        if dir.exists() && refresh {
            if !options.allow_clone {
                return Err(FetchError::CloneDisabled);
            }
            fs::remove_dir_all(&dir).map_err(io_failure("remove the cached checkout"))?;
        }
        if dir.exists() {
            verify_origin(&dir, repo)?;
            return Ok(CheckedOutRepo {
                dir: canonicalize_clean(&dir).map_err(io_failure("resolve the checkout"))?,
                commit: head_commit(&dir),
                reused_cache: true,
            });
        }
        if !options.allow_clone {
            return Err(FetchError::CloneDisabled);
        }

        let parent = dir.parent().unwrap_or(&options.repos_dir);
        fs::create_dir_all(parent).map_err(io_failure("create the checkout directory"))?;
        let scratch = options
            .repos_dir
            .join(format!(".partial-{}", unique_suffix()));
        if let Err(err) = clone_into(repo, &scratch, options) {
            remove_quietly(&scratch);
            return Err(err);
        }
        if let Err(err) = fs::rename(&scratch, &dir) {
            remove_quietly(&scratch);
            // A concurrent request may have completed the same clone first.
            if !dir.exists() {
                return Err(io_failure("move the clone into place")(err));
            }
            verify_origin(&dir, repo)?;
        }
        Ok(CheckedOutRepo {
            dir: canonicalize_clean(&dir).map_err(io_failure("resolve the checkout"))?,
            commit: head_commit(&dir),
            reused_cache: false,
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
        && !repo.ends_with('.');
    if valid {
        Ok(())
    } else {
        Err(format!("'{}' is not a valid GitHub repository name", repo))
    }
}

fn validate_ref(git_ref: &str) -> Result<(), String> {
    let valid = !git_ref.is_empty()
        && git_ref.len() <= 255
        && !git_ref.starts_with('-')
        && !git_ref.starts_with('.')
        && !git_ref.ends_with(".lock")
        && !git_ref.contains("..")
        && !git_ref.contains("@{")
        && git_ref
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.+".contains(&byte));
    if valid {
        Ok(())
    } else {
        Err(format!("'{}' is not an accepted branch or tag name", git_ref))
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
        Err(format!("'{}' is not an accepted repository path segment", segment))
    }
}

/// A `git` invocation with every transport but HTTPS disabled and no interactive or
/// credential-helper behavior.
fn git_command() -> Command {
    let mut command = Command::new("git");
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ALLOW_PROTOCOL", "https")
        .env("GIT_LFS_SKIP_SMUDGE", "1")
        .env("GCM_INTERACTIVE", "never")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .stdin(Stdio::null())
        .args([
            "-c",
            "protocol.allow=never",
            "-c",
            "protocol.https.allow=always",
            "-c",
            "credential.helper=",
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
    let mut command = git_command();
    command.args(["clone", "--depth", "1", "--single-branch", "--no-tags", "--quiet"]);
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
        let _ = fs::remove_dir_all(path);
    }
}

fn run_git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = git_command()
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

fn head_commit(dir: &Path) -> Option<String> {
    run_git(dir, &["rev-parse", "HEAD"]).filter(|sha| {
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

fn verify_origin(dir: &Path, repo: &GitHubRepoRef) -> Result<(), FetchError> {
    if !dir.join(".git").is_dir() {
        return Err(FetchError::CacheMismatch {
            detail: format!("'{}' exists but is not a git checkout", dir.display()),
        });
    }
    let origin = run_git(dir, &["config", "--get", "remote.origin.url"]).ok_or_else(|| {
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

    #[test]
    fn test_uml_diagram_all_count() {
        let all = UmlDiagramType::fill_all();
        assert_eq!(all.len(), 14);
    }
}
