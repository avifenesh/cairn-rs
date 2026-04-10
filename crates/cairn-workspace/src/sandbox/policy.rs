use std::fmt::{Display, Formatter};
use std::path::PathBuf;
use std::time::Duration;

use cairn_domain::OnExhaustion;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RepoId(String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidRepoId {
    value: String,
    reason: &'static str,
}

impl InvalidRepoId {
    pub fn new(value: impl Into<String>, reason: &'static str) -> Self {
        Self {
            value: value.into(),
            reason,
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn reason(&self) -> &'static str {
        self.reason
    }
}

impl Display for InvalidRepoId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid repo_id `{}`: {}", self.value, self.reason)
    }
}

impl std::error::Error for InvalidRepoId {}

impl RepoId {
    pub const MAX_LENGTH: usize = 200;
    pub const MAX_SEGMENT_LENGTH: usize = 100;

    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn parse(value: impl Into<String>) -> Result<Self, InvalidRepoId> {
        let value = value.into();
        let normalized = value.trim();
        Self::validate_str(normalized)?;
        Ok(Self(normalized.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn validate(&self) -> Result<(), InvalidRepoId> {
        Self::validate_str(self.as_str())
    }

    pub fn owner_and_repo(&self) -> (&str, &str) {
        self.0
            .split_once('/')
            .expect("RepoId must be validated before path use")
    }

    fn validate_str(value: &str) -> Result<(), InvalidRepoId> {
        if value.is_empty() {
            return Err(InvalidRepoId::new(value, "must not be empty"));
        }

        if value.len() > Self::MAX_LENGTH {
            return Err(InvalidRepoId::new(value, "must be 200 characters or fewer"));
        }

        let Some((owner, repo)) = value.split_once('/') else {
            return Err(InvalidRepoId::new(value, "must be in owner/repo form"));
        };

        if repo.contains('/') {
            return Err(InvalidRepoId::new(value, "must be in owner/repo form"));
        }

        Self::validate_segment(value, owner, "owner")?;
        Self::validate_segment(value, repo, "repo")?;
        Ok(())
    }

    fn validate_segment(
        value: &str,
        segment: &str,
        label: &'static str,
    ) -> Result<(), InvalidRepoId> {
        if segment.is_empty() {
            return Err(InvalidRepoId::new(value, "must be in owner/repo form"));
        }

        if segment == "." || segment == ".." {
            return Err(InvalidRepoId::new(
                value,
                "must not contain dot path segments",
            ));
        }

        if segment.len() > Self::MAX_SEGMENT_LENGTH {
            return Err(InvalidRepoId::new(
                value,
                "owner and repo segments must be 100 characters or fewer",
            ));
        }

        if !segment
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
        {
            return Err(InvalidRepoId::new(
                value,
                match label {
                    "owner" => "owner contains unsupported characters",
                    _ => "repo contains unsupported characters",
                },
            ));
        }

        Ok(())
    }
}

impl Display for RepoId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for RepoId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for RepoId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SandboxStrategy {
    #[serde(rename = "overlay_fs")]
    Overlay,
    #[serde(rename = "reflink")]
    Reflink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SandboxStrategyRequest {
    Preferred(SandboxStrategy),
    Force(SandboxStrategy),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostCapabilityRequirements {
    pub requires_user_namespaces: bool,
    pub requires_reflink_support: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialReference {
    Named(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SandboxBase {
    Repo {
        repo_id: RepoId,
        starting_ref: Option<String>,
    },
    Directory {
        path: PathBuf,
    },
    Empty,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxPolicy {
    pub strategy: SandboxStrategyRequest,
    pub base: SandboxBase,
    pub credentials: Vec<CredentialReference>,
    pub network_egress: Option<Vec<String>>,
    pub memory_limit_bytes: Option<u64>,
    pub cpu_weight: Option<u32>,
    pub disk_quota_bytes: Option<u64>,
    pub wall_clock_limit: Option<Duration>,
    pub on_resource_exhaustion: OnExhaustion,
    pub preserve_on_failure: bool,
    pub required_host_caps: HostCapabilityRequirements,
}
