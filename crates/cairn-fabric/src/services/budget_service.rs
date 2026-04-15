use std::collections::HashMap;
use std::sync::Arc;

use crate::error::FabricError;
use ff_core::keys::{budget_resets_key, BudgetKeyContext};
use ff_core::partition::budget_partition;
use ff_core::types::{BudgetId, TimestampMs};

use crate::boot::FabricRuntime;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpendResult {
    Ok,
    SoftBreach {
        dimension: String,
        action: String,
    },
    HardBreach {
        dimension: String,
        action: String,
        current_usage: u64,
        hard_limit: u64,
    },
}

#[derive(Clone, Debug)]
pub struct BudgetStatus {
    pub budget_id: String,
    pub scope_type: String,
    pub scope_id: String,
    pub enforcement_mode: String,
    pub usage: HashMap<String, u64>,
    pub hard_limits: HashMap<String, u64>,
    pub soft_limits: HashMap<String, u64>,
    pub breach_count: u64,
    pub soft_breach_count: u64,
}

pub struct FabricBudgetService {
    runtime: Arc<FabricRuntime>,
}

impl FabricBudgetService {
    pub fn new(runtime: Arc<FabricRuntime>) -> Self {
        Self { runtime }
    }

    pub async fn create_budget(
        &self,
        scope_type: &str,
        scope_id: &str,
        dimensions: &[&str],
        hard_limits: &[u64],
        soft_limits: &[u64],
        reset_interval_ms: u64,
        enforcement_mode: &str,
    ) -> Result<BudgetId, FabricError> {
        if dimensions.len() != hard_limits.len() || dimensions.len() != soft_limits.len() {
            return Err(FabricError::Validation {
                reason: "dimensions, hard_limits, soft_limits must have equal length".to_owned(),
            });
        }

        let budget_id = BudgetId::new();
        let partition = budget_partition(&budget_id, &self.runtime.partition_config);
        let ctx = BudgetKeyContext::new(&partition, &budget_id);
        let resets_zset = budget_resets_key(&partition.hash_tag());
        let now = TimestampMs::now();

        let dim_count = dimensions.len();
        let mut argv: Vec<String> = Vec::with_capacity(9 + dim_count * 3);
        argv.push(budget_id.to_string());
        argv.push(scope_type.to_owned());
        argv.push(scope_id.to_owned());
        argv.push(enforcement_mode.to_owned());
        argv.push("block".to_owned());
        argv.push("log".to_owned());
        argv.push(reset_interval_ms.to_string());
        argv.push(now.to_string());
        argv.push(dim_count.to_string());
        for dim in dimensions {
            argv.push((*dim).to_owned());
        }
        for &hard in hard_limits {
            argv.push(hard.to_string());
        }
        for &soft in soft_limits {
            argv.push(soft.to_string());
        }

        let keys: Vec<String> = vec![ctx.definition(), ctx.limits(), ctx.usage(), resets_zset];

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();

        let _: ferriskey::Value = self
            .runtime
            .client
            .fcall("ff_create_budget", &key_refs, &argv_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_create_budget: {e}")))?;

        Ok(budget_id)
    }

    pub async fn create_run_budget(
        &self,
        run_id: &cairn_domain::RunId,
        token_limit: u64,
        cost_limit_microdollars: u64,
        api_call_limit: u64,
    ) -> Result<BudgetId, FabricError> {
        self.create_budget(
            "run",
            run_id.as_str(),
            &["tokens", "cost_microdollars", "api_calls"],
            &[token_limit, cost_limit_microdollars, api_call_limit],
            &[
                token_limit / 100 * 80,
                cost_limit_microdollars / 100 * 80,
                api_call_limit / 100 * 80,
            ],
            0,
            "enforce",
        )
        .await
    }

    pub async fn create_tenant_budget(
        &self,
        tenant_id: &cairn_domain::TenantId,
        token_limit: u64,
        cost_limit_microdollars: u64,
        api_call_limit: u64,
        reset_interval_ms: u64,
    ) -> Result<BudgetId, FabricError> {
        self.create_budget(
            "tenant",
            tenant_id.as_str(),
            &["tokens", "cost_microdollars", "api_calls"],
            &[token_limit, cost_limit_microdollars, api_call_limit],
            &[
                token_limit / 100 * 80,
                cost_limit_microdollars / 100 * 80,
                api_call_limit / 100 * 80,
            ],
            reset_interval_ms,
            "enforce",
        )
        .await
    }

    pub async fn record_spend(
        &self,
        budget_id: &BudgetId,
        dimension_deltas: &[(&str, u64)],
    ) -> Result<SpendResult, FabricError> {
        let partition = budget_partition(budget_id, &self.runtime.partition_config);
        let ctx = BudgetKeyContext::new(&partition, budget_id);
        let now = TimestampMs::now();

        let dim_count = dimension_deltas.len();
        let mut argv: Vec<String> = Vec::with_capacity(2 + dim_count * 2);
        argv.push(dim_count.to_string());
        for (dim, _) in dimension_deltas {
            argv.push((*dim).to_owned());
        }
        for (_, delta) in dimension_deltas {
            argv.push(delta.to_string());
        }
        argv.push(now.to_string());

        let keys: Vec<String> = vec![ctx.usage(), ctx.limits(), ctx.definition()];

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime
            .client
            .fcall("ff_report_usage_and_check", &key_refs, &argv_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_report_usage_and_check: {e}")))?;

        parse_spend_result(&raw)
    }

    pub async fn get_budget_status(
        &self,
        budget_id: &BudgetId,
    ) -> Result<BudgetStatus, FabricError> {
        let partition = budget_partition(budget_id, &self.runtime.partition_config);
        let ctx = BudgetKeyContext::new(&partition, budget_id);

        let def_fields: HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&ctx.definition())
            .await
            .map_err(|e| FabricError::Internal(format!("HGETALL budget def: {e}")))?;

        if def_fields.is_empty() {
            return Err(FabricError::NotFound {
                entity: "budget",
                id: budget_id.to_string(),
            });
        }

        let usage_fields: HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&ctx.usage())
            .await
            .unwrap_or_default();

        let limits_fields: HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&ctx.limits())
            .await
            .unwrap_or_default();

        let mut usage = HashMap::new();
        let mut hard_limits = HashMap::new();
        let mut soft_limits = HashMap::new();

        for (k, v) in &usage_fields {
            if let Ok(val) = v.parse::<u64>() {
                usage.insert(k.clone(), val);
            }
        }

        for (k, v) in &limits_fields {
            if let Some(dim) = k.strip_suffix(":hard") {
                if let Ok(val) = v.parse::<u64>() {
                    hard_limits.insert(dim.to_owned(), val);
                }
            } else if let Some(dim) = k.strip_suffix(":soft") {
                if let Ok(val) = v.parse::<u64>() {
                    soft_limits.insert(dim.to_owned(), val);
                }
            }
        }

        let breach_count = def_fields
            .get("breach_count")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let soft_breach_count = def_fields
            .get("soft_breach_count")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        Ok(BudgetStatus {
            budget_id: budget_id.to_string(),
            scope_type: def_fields.get("scope_type").cloned().unwrap_or_default(),
            scope_id: def_fields.get("scope_id").cloned().unwrap_or_default(),
            enforcement_mode: def_fields
                .get("enforcement_mode")
                .cloned()
                .unwrap_or_default(),
            usage,
            hard_limits,
            soft_limits,
            breach_count,
            soft_breach_count,
        })
    }
}

fn parse_spend_result(raw: &ferriskey::Value) -> Result<SpendResult, FabricError> {
    let arr = match raw {
        ferriskey::Value::Array(arr) => arr,
        _ => {
            return Err(FabricError::Internal(
                "ff_report_usage_and_check: expected Array".to_owned(),
            ))
        }
    };

    let status = match arr.first() {
        Some(Ok(ferriskey::Value::BulkString(b))) => String::from_utf8_lossy(b).into_owned(),
        Some(Ok(ferriskey::Value::SimpleString(s))) => s.clone(),
        _ => {
            return Err(FabricError::Internal(
                "ff_report_usage_and_check: missing status".to_owned(),
            ))
        }
    };

    match status.as_str() {
        "OK" => Ok(SpendResult::Ok),
        "SOFT_BREACH" => {
            let dimension = field_str(arr, 1);
            let action = field_str(arr, 2);
            Ok(SpendResult::SoftBreach { dimension, action })
        }
        "HARD_BREACH" => {
            let dimension = field_str(arr, 1);
            let action = field_str(arr, 2);
            let current_usage = field_str(arr, 3).parse().unwrap_or(0);
            let hard_limit = field_str(arr, 4).parse().unwrap_or(0);
            Ok(SpendResult::HardBreach {
                dimension,
                action,
                current_usage,
                hard_limit,
            })
        }
        _ => Err(FabricError::Internal(format!(
            "ff_report_usage_and_check: unknown status: {status}"
        ))),
    }
}

fn field_str(arr: &[Result<ferriskey::Value, ferriskey::Error>], index: usize) -> String {
    match arr.get(index) {
        Some(Ok(ferriskey::Value::BulkString(b))) => String::from_utf8_lossy(b).into_owned(),
        Some(Ok(ferriskey::Value::SimpleString(s))) => s.clone(),
        Some(Ok(ferriskey::Value::Int(n))) => n.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spend_result_ok_variant() {
        let raw =
            ferriskey::Value::Array(vec![Ok(ferriskey::Value::SimpleString("OK".to_owned()))]);
        let result = parse_spend_result(&raw).unwrap();
        assert_eq!(result, SpendResult::Ok);
    }

    #[test]
    fn spend_result_soft_breach() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::SimpleString("SOFT_BREACH".to_owned())),
            Ok(ferriskey::Value::BulkString(b"tokens".to_vec().into())),
            Ok(ferriskey::Value::BulkString(b"log".to_vec().into())),
        ]);
        let result = parse_spend_result(&raw).unwrap();
        assert_eq!(
            result,
            SpendResult::SoftBreach {
                dimension: "tokens".to_owned(),
                action: "log".to_owned(),
            }
        );
    }

    #[test]
    fn spend_result_hard_breach() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::SimpleString("HARD_BREACH".to_owned())),
            Ok(ferriskey::Value::BulkString(
                b"cost_microdollars".to_vec().into(),
            )),
            Ok(ferriskey::Value::BulkString(b"block".to_vec().into())),
            Ok(ferriskey::Value::Int(5000)),
            Ok(ferriskey::Value::Int(4000)),
        ]);
        let result = parse_spend_result(&raw).unwrap();
        assert_eq!(
            result,
            SpendResult::HardBreach {
                dimension: "cost_microdollars".to_owned(),
                action: "block".to_owned(),
                current_usage: 5000,
                hard_limit: 4000,
            }
        );
    }

    #[test]
    fn spend_result_unknown_status_errors() {
        let raw = ferriskey::Value::Array(vec![Ok(ferriskey::Value::SimpleString(
            "GARBAGE".to_owned(),
        ))]);
        let result = parse_spend_result(&raw);
        assert!(result.is_err());
    }

    #[test]
    fn field_str_extracts_bulk_string() {
        let arr = vec![
            Ok(ferriskey::Value::BulkString(b"hello".to_vec().into())),
            Ok(ferriskey::Value::Int(42)),
        ];
        assert_eq!(field_str(&arr, 0), "hello");
        assert_eq!(field_str(&arr, 1), "42");
        assert_eq!(field_str(&arr, 99), "");
    }

    #[test]
    fn budget_status_default_values() {
        let status = BudgetStatus {
            budget_id: "test".to_owned(),
            scope_type: "run".to_owned(),
            scope_id: "run_1".to_owned(),
            enforcement_mode: "enforce".to_owned(),
            usage: HashMap::new(),
            hard_limits: HashMap::new(),
            soft_limits: HashMap::new(),
            breach_count: 0,
            soft_breach_count: 0,
        };
        assert_eq!(status.scope_type, "run");
        assert!(status.usage.is_empty());
    }

    #[test]
    fn standard_dimensions() {
        let dims = ["tokens", "cost_microdollars", "api_calls"];
        assert_eq!(dims.len(), 3);
        assert!(dims.contains(&"tokens"));
        assert!(dims.contains(&"cost_microdollars"));
        assert!(dims.contains(&"api_calls"));
    }
}
