use std::fmt::{Display, Formatter};
use std::path::PathBuf;
use std::time::Duration;

use cairn_domain::OnExhaustion;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RepoId(String);

impl RepoId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn owner_and_repo(&self) -> (&str, &str) {
        self.0.split_once('/').unwrap_or(("_", self.0.as_str()))
    }
}

impl Display for RepoId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SandboxStrategy {
    Overlay,
    Reflink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxStrategyRequest {
    Preferred(SandboxStrategy),
    Force(SandboxStrategy),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostCapabilityRequirements {
    pub requires_user_namespaces: bool,
    pub requires_reflink_support: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CredentialReference {
    Named(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
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
