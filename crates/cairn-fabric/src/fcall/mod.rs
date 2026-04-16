pub mod budget;
pub mod claim;
pub mod execution;
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
        name: names::FF_DELIVER_SIGNAL,
        expected_keys: suspension::DELIVER_SIGNAL_KEYS,
        expected_args: suspension::DELIVER_SIGNAL_ARGS,
    },
    FcallContract {
        name: names::FF_CLAIM_EXECUTION,
        expected_keys: claim::CLAIM_EXECUTION_KEYS,
        expected_args: claim::CLAIM_EXECUTION_ARGS,
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
            if args.len() != contract.expected_args {
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
    fn all_critical_contracts_have_nonzero_counts() {
        for c in CRITICAL_CONTRACTS {
            assert!(c.expected_keys > 0, "{} has 0 keys", c.name);
            assert!(c.expected_args > 0, "{} has 0 args", c.name);
        }
    }
}
