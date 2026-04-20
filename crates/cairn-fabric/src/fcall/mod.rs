pub mod budget;
pub mod claim;
pub mod execution;
pub mod flow_edges;
pub mod names;
pub mod quota;
pub mod session;
pub mod suspension;

use crate::error::FabricError;

pub struct FcallContract {
    pub name: &'static str,
    pub expected_keys: usize,
    pub expected_args: usize,
}

pub const CRITICAL_CONTRACTS: &[FcallContract] = &[
    FcallContract {
        name: names::FF_CREATE_EXECUTION,
        expected_keys: execution::CREATE_EXECUTION_KEYS,
        expected_args: execution::CREATE_EXECUTION_ARGS,
    },
    FcallContract {
        name: names::FF_COMPLETE_EXECUTION,
        expected_keys: execution::COMPLETE_EXECUTION_KEYS,
        expected_args: execution::COMPLETE_EXECUTION_ARGS,
    },
    FcallContract {
        name: names::FF_FAIL_EXECUTION,
        expected_keys: execution::FAIL_EXECUTION_KEYS,
        expected_args: execution::FAIL_EXECUTION_ARGS,
    },
    FcallContract {
        name: names::FF_CANCEL_EXECUTION,
        expected_keys: execution::CANCEL_EXECUTION_KEYS,
        expected_args: execution::CANCEL_EXECUTION_ARGS,
    },
    FcallContract {
        name: names::FF_SUSPEND_EXECUTION,
        expected_keys: suspension::SUSPEND_EXECUTION_KEYS,
        expected_args: suspension::SUSPEND_EXECUTION_ARGS,
    },
    FcallContract {
        name: names::FF_RESUME_EXECUTION,
        expected_keys: suspension::RESUME_EXECUTION_KEYS,
        expected_args: suspension::RESUME_EXECUTION_ARGS,
    },
    FcallContract {
        name: names::FF_DELIVER_SIGNAL,
        expected_keys: suspension::DELIVER_SIGNAL_KEYS,
        expected_args: suspension::DELIVER_SIGNAL_ARGS,
    },
    FcallContract {
        name: names::FF_ISSUE_CLAIM_GRANT,
        expected_keys: claim::ISSUE_CLAIM_GRANT_KEYS,
        expected_args: claim::ISSUE_CLAIM_GRANT_ARGS,
    },
    FcallContract {
        name: names::FF_CLAIM_EXECUTION,
        expected_keys: claim::CLAIM_EXECUTION_KEYS,
        expected_args: claim::CLAIM_EXECUTION_ARGS,
    },
    FcallContract {
        name: names::FF_RENEW_LEASE,
        expected_keys: claim::RENEW_LEASE_KEYS,
        expected_args: claim::RENEW_LEASE_ARGS,
    },
    FcallContract {
        name: names::FF_CREATE_FLOW,
        expected_keys: session::CREATE_FLOW_KEYS,
        expected_args: session::CREATE_FLOW_ARGS,
    },
    FcallContract {
        name: names::FF_CANCEL_FLOW,
        expected_keys: session::CANCEL_FLOW_KEYS,
        expected_args: session::CANCEL_FLOW_ARGS,
    },
    FcallContract {
        name: names::FF_CREATE_BUDGET,
        expected_keys: budget::CREATE_BUDGET_KEYS,
        expected_args: 0, // variable: 9 + dim_count * 3
    },
    // NOTE: ff_report_usage_and_check has variable-length args (depends on
    // dimension count). Cannot be statically verified — KEYS-only check.
    FcallContract {
        name: names::FF_REPORT_USAGE_AND_CHECK,
        expected_keys: budget::REPORT_USAGE_KEYS,
        expected_args: 0, // variable: 1 + dim_count * 2 + 1
    },
    FcallContract {
        name: names::FF_CREATE_QUOTA_POLICY,
        expected_keys: quota::CREATE_QUOTA_POLICY_KEYS,
        expected_args: quota::CREATE_QUOTA_POLICY_ARGS,
    },
    FcallContract {
        name: names::FF_CHECK_ADMISSION_AND_RECORD,
        expected_keys: quota::CHECK_ADMISSION_KEYS,
        expected_args: quota::CHECK_ADMISSION_ARGS,
    },
    FcallContract {
        name: names::FF_RESET_BUDGET,
        expected_keys: budget::RESET_BUDGET_KEYS,
        expected_args: budget::RESET_BUDGET_ARGS,
    },
    FcallContract {
        name: names::FF_ADD_EXECUTION_TO_FLOW,
        expected_keys: flow_edges::ADD_EXECUTION_TO_FLOW_KEYS,
        expected_args: flow_edges::ADD_EXECUTION_TO_FLOW_ARGS,
    },
    FcallContract {
        name: names::FF_STAGE_DEPENDENCY_EDGE,
        expected_keys: flow_edges::STAGE_DEPENDENCY_EDGE_KEYS,
        expected_args: flow_edges::STAGE_DEPENDENCY_EDGE_ARGS,
    },
    FcallContract {
        name: names::FF_APPLY_DEPENDENCY_TO_CHILD,
        expected_keys: flow_edges::APPLY_DEPENDENCY_TO_CHILD_KEYS,
        expected_args: flow_edges::APPLY_DEPENDENCY_TO_CHILD_ARGS,
    },
    FcallContract {
        name: names::FF_EVALUATE_FLOW_ELIGIBILITY,
        expected_keys: flow_edges::EVALUATE_FLOW_ELIGIBILITY_KEYS,
        expected_args: 0,
    },
];

pub fn verify_builder_counts(
    function_name: &str,
    keys: &[String],
    args: &[String],
) -> Result<(), FabricError> {
    for contract in CRITICAL_CONTRACTS {
        if contract.name == function_name {
            if keys.len() != contract.expected_keys {
                return Err(FabricError::Internal(format!(
                    "{function_name}: expected {expected} KEYS, got {actual}",
                    expected = contract.expected_keys,
                    actual = keys.len(),
                )));
            }
            if contract.expected_args > 0 && args.len() != contract.expected_args {
                return Err(FabricError::Internal(format!(
                    "{function_name}: expected {expected} ARGS, got {actual}",
                    expected = contract.expected_args,
                    actual = args.len(),
                )));
            }
            return Ok(());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_correct_counts_passes() {
        let keys = vec!["k".to_owned(); execution::CREATE_EXECUTION_KEYS];
        let args = vec!["a".to_owned(); execution::CREATE_EXECUTION_ARGS];
        assert!(verify_builder_counts(names::FF_CREATE_EXECUTION, &keys, &args).is_ok());
    }

    #[test]
    fn verify_wrong_key_count_fails() {
        let keys = vec!["k".to_owned(); 3];
        let args = vec!["a".to_owned(); execution::CREATE_EXECUTION_ARGS];
        let err = verify_builder_counts(names::FF_CREATE_EXECUTION, &keys, &args);
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("KEYS"));
    }

    #[test]
    fn verify_wrong_arg_count_fails() {
        let keys = vec!["k".to_owned(); execution::CREATE_EXECUTION_KEYS];
        let args = vec!["a".to_owned(); 2];
        let err = verify_builder_counts(names::FF_CREATE_EXECUTION, &keys, &args);
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("ARGS"));
    }

    #[test]
    fn verify_unknown_function_passes() {
        let keys = vec![];
        let args = vec![];
        assert!(verify_builder_counts("ff_unknown", &keys, &args).is_ok());
    }

    #[test]
    fn all_critical_contracts_have_nonzero_key_counts() {
        for c in CRITICAL_CONTRACTS {
            assert!(c.expected_keys > 0, "{} has 0 keys", c.name);
        }
    }

    #[test]
    fn fixed_arg_contracts_have_nonzero_arg_counts() {
        for c in CRITICAL_CONTRACTS {
            if c.expected_args > 0 {
                assert!(c.expected_args >= 2, "{} has suspiciously few args", c.name);
            }
        }
    }
}
