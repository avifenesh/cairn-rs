use std::sync::Arc;

use crate::error::FabricError;
use ff_core::keys::{quota_policies_index, QuotaKeyContext};
use ff_core::partition::quota_partition;
use ff_core::types::{ExecutionId, QuotaPolicyId, TimestampMs};

use crate::boot::FabricRuntime;
use crate::id_map;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdmissionResult {
    Admitted,
    AlreadyAdmitted,
    RateExceeded { retry_after_ms: u64 },
    ConcurrencyExceeded,
}

pub struct FabricQuotaService {
    runtime: Arc<FabricRuntime>,
}

impl FabricQuotaService {
    pub fn new(runtime: Arc<FabricRuntime>) -> Self {
        Self { runtime }
    }

    pub async fn create_quota_policy(
        &self,
        scope_type: &str,
        scope_id: &str,
        window_seconds: u64,
        max_requests_per_window: u64,
        max_concurrent: u64,
    ) -> Result<QuotaPolicyId, FabricError> {
        let qid = QuotaPolicyId::new();
        let partition = quota_partition(&qid, &self.runtime.partition_config);
        let ctx = QuotaKeyContext::new(&partition, &qid);
        let now = TimestampMs::now();

        let dimension = "default";
        let policies_index = quota_policies_index(&partition.hash_tag());

        let (keys, args) = crate::fcall::quota::build_create_quota_policy(
            &ctx,
            &policies_index,
            &qid,
            window_seconds,
            max_requests_per_window,
            max_concurrent,
            now,
            dimension,
        );
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let _: ferriskey::Value = self
            .runtime
            .fcall(
                crate::fcall::names::FF_CREATE_QUOTA_POLICY,
                &key_refs,
                &arg_refs,
            )
            .await?;

        let def_key = ctx.definition();
        self.runtime
            .client
            .hset(&def_key, "scope_type", scope_type)
            .await
            .map_err(|e| FabricError::Valkey(format!("HSET scope_type: {e}")))?;
        self.runtime
            .client
            .hset(&def_key, "scope_id", scope_id)
            .await
            .map_err(|e| FabricError::Valkey(format!("HSET scope_id: {e}")))?;

        Ok(qid)
    }

    pub async fn create_tenant_quota(
        &self,
        tenant_id: &cairn_domain::TenantId,
        window_seconds: u64,
        max_requests_per_window: u64,
        max_concurrent: u64,
    ) -> Result<QuotaPolicyId, FabricError> {
        self.create_quota_policy(
            "tenant",
            tenant_id.as_str(),
            window_seconds,
            max_requests_per_window,
            max_concurrent,
        )
        .await
    }

    pub async fn create_workspace_quota(
        &self,
        workspace_id: &str,
        window_seconds: u64,
        max_requests_per_window: u64,
        max_concurrent: u64,
    ) -> Result<QuotaPolicyId, FabricError> {
        self.create_quota_policy(
            "workspace",
            workspace_id,
            window_seconds,
            max_requests_per_window,
            max_concurrent,
        )
        .await
    }

    pub async fn create_user_quota(
        &self,
        user_id: &str,
        window_seconds: u64,
        max_requests_per_window: u64,
        max_concurrent: u64,
    ) -> Result<QuotaPolicyId, FabricError> {
        self.create_quota_policy(
            "user",
            user_id,
            window_seconds,
            max_requests_per_window,
            max_concurrent,
        )
        .await
    }

    pub async fn check_admission(
        &self,
        quota_policy_id: &QuotaPolicyId,
        execution_id: &ExecutionId,
        window_seconds: u64,
        rate_limit: u64,
        concurrency_cap: u64,
    ) -> Result<AdmissionResult, FabricError> {
        let partition = quota_partition(quota_policy_id, &self.runtime.partition_config);
        let ctx = QuotaKeyContext::new(&partition, quota_policy_id);
        let now = TimestampMs::now();
        let dimension = "default";

        let (keys, args) = crate::fcall::quota::build_check_admission(
            &ctx,
            execution_id,
            now,
            window_seconds,
            rate_limit,
            concurrency_cap,
            dimension,
        );
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime
            .fcall(
                crate::fcall::names::FF_CHECK_ADMISSION_AND_RECORD,
                &key_refs,
                &arg_refs,
            )
            .await?;

        parse_admission_result(&raw)
    }

    pub async fn check_admission_for_run(
        &self,
        quota_policy_id: &QuotaPolicyId,
        project: &cairn_domain::tenancy::ProjectKey,
        run_id: &cairn_domain::RunId,
        window_seconds: u64,
        rate_limit: u64,
        concurrency_cap: u64,
    ) -> Result<AdmissionResult, FabricError> {
        let eid = id_map::run_to_execution_id(project, run_id);
        self.check_admission(
            quota_policy_id,
            &eid,
            window_seconds,
            rate_limit,
            concurrency_cap,
        )
        .await
    }
}

fn parse_admission_result(raw: &ferriskey::Value) -> Result<AdmissionResult, FabricError> {
    let arr = match raw {
        ferriskey::Value::Array(arr) => arr,
        _ => {
            return Err(FabricError::Internal(
                "ff_check_admission_and_record: expected Array".to_owned(),
            ))
        }
    };

    let status = match arr.first() {
        Some(Ok(ferriskey::Value::BulkString(b))) => String::from_utf8_lossy(b).into_owned(),
        Some(Ok(ferriskey::Value::SimpleString(s))) => s.clone(),
        _ => {
            return Err(FabricError::Internal(
                "ff_check_admission_and_record: missing status".to_owned(),
            ))
        }
    };

    match status.as_str() {
        "ADMITTED" => Ok(AdmissionResult::Admitted),
        "ALREADY_ADMITTED" => Ok(AdmissionResult::AlreadyAdmitted),
        "RATE_EXCEEDED" => {
            let retry_str = match arr.get(1) {
                Some(Ok(ferriskey::Value::BulkString(b))) => {
                    String::from_utf8_lossy(b).into_owned()
                }
                Some(Ok(ferriskey::Value::Int(n))) => n.to_string(),
                _ => "0".to_owned(),
            };
            let retry_after_ms = retry_str.parse().unwrap_or(0);
            Ok(AdmissionResult::RateExceeded { retry_after_ms })
        }
        "CONCURRENCY_EXCEEDED" => Ok(AdmissionResult::ConcurrencyExceeded),
        _ => Err(FabricError::Internal(format!(
            "ff_check_admission_and_record: unknown status: {status}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_result_admitted() {
        let raw = ferriskey::Value::Array(vec![Ok(ferriskey::Value::SimpleString(
            "ADMITTED".to_owned(),
        ))]);
        let result = parse_admission_result(&raw).unwrap();
        assert_eq!(result, AdmissionResult::Admitted);
    }

    #[test]
    fn admission_result_already_admitted() {
        let raw = ferriskey::Value::Array(vec![Ok(ferriskey::Value::SimpleString(
            "ALREADY_ADMITTED".to_owned(),
        ))]);
        let result = parse_admission_result(&raw).unwrap();
        assert_eq!(result, AdmissionResult::AlreadyAdmitted);
    }

    #[test]
    fn admission_result_rate_exceeded() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::SimpleString("RATE_EXCEEDED".to_owned())),
            Ok(ferriskey::Value::Int(5000)),
        ]);
        let result = parse_admission_result(&raw).unwrap();
        assert_eq!(
            result,
            AdmissionResult::RateExceeded {
                retry_after_ms: 5000
            }
        );
    }

    #[test]
    fn admission_result_rate_exceeded_bulk_string() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::BulkString(
                b"RATE_EXCEEDED".to_vec().into(),
            )),
            Ok(ferriskey::Value::BulkString(b"3000".to_vec().into())),
        ]);
        let result = parse_admission_result(&raw).unwrap();
        assert_eq!(
            result,
            AdmissionResult::RateExceeded {
                retry_after_ms: 3000
            }
        );
    }

    #[test]
    fn admission_result_concurrency_exceeded() {
        let raw = ferriskey::Value::Array(vec![Ok(ferriskey::Value::SimpleString(
            "CONCURRENCY_EXCEEDED".to_owned(),
        ))]);
        let result = parse_admission_result(&raw).unwrap();
        assert_eq!(result, AdmissionResult::ConcurrencyExceeded);
    }

    #[test]
    fn admission_result_unknown_errors() {
        let raw = ferriskey::Value::Array(vec![Ok(ferriskey::Value::SimpleString(
            "GARBAGE".to_owned(),
        ))]);
        let result = parse_admission_result(&raw);
        assert!(result.is_err());
    }

    #[test]
    fn admission_result_non_array_errors() {
        let raw = ferriskey::Value::SimpleString("not an array".to_owned());
        let result = parse_admission_result(&raw);
        assert!(result.is_err());
    }
}
