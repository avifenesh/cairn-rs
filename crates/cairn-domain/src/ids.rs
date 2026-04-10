use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self(String::new())
            }
        }
    };
}

define_id!(ApprovalId);
define_id!(ChannelId);
define_id!(CheckpointId);
define_id!(ChunkId);
define_id!(CommandId);
define_id!(CorrelationId);
define_id!(CredentialId);
define_id!(DecisionId);
define_id!(EvalRunId);
define_id!(EventId);
define_id!(IngestJobId);
define_id!(KnowledgeDocumentId);
define_id!(KnowledgePackId);
define_id!(MailboxMessageId);
define_id!(OperatorId);
define_id!(OutcomeId);
define_id!(PolicyId);
define_id!(ProjectId);
define_id!(PromptAssetId);
define_id!(PromptReleaseId);
define_id!(PromptVersionId);
define_id!(ProviderBindingId);
define_id!(ProviderConnectionId);
define_id!(ProviderCallId);
define_id!(ProviderModelId);
define_id!(ProviderRouteTemplateId);
define_id!(ReleaseActionId);
define_id!(RouteAttemptId);
define_id!(RouteDecisionId);
define_id!(RunId);
define_id!(RunTemplateId);
define_id!(SessionId);
define_id!(SignalId);
define_id!(ScheduledTaskId);
define_id!(SourceId);
define_id!(TaskId);
define_id!(TenantId);
define_id!(ToolInvocationId);
define_id!(TriggerId);
define_id!(WorkerId);
define_id!(WorkspaceId);

#[cfg(test)]
mod tests {
    use super::{ProjectId, TenantId};

    #[test]
    fn ids_preserve_string_representation() {
        let tenant_id = TenantId::new("tenant_acme");
        let project_id = ProjectId::from("project_support");

        assert_eq!(tenant_id.as_str(), "tenant_acme");
        assert_eq!(project_id.to_string(), "project_support");
    }
}
