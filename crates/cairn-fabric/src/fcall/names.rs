pub const FF_CREATE_EXECUTION: &str = "ff_create_execution";
pub const FF_COMPLETE_EXECUTION: &str = "ff_complete_execution";
pub const FF_FAIL_EXECUTION: &str = "ff_fail_execution";
pub const FF_CANCEL_EXECUTION: &str = "ff_cancel_execution";
pub const FF_SUSPEND_EXECUTION: &str = "ff_suspend_execution";
pub const FF_RESUME_EXECUTION: &str = "ff_resume_execution";
pub const FF_DELIVER_SIGNAL: &str = "ff_deliver_signal";
pub const FF_ISSUE_CLAIM_GRANT: &str = "ff_issue_claim_grant";
pub const FF_CLAIM_EXECUTION: &str = "ff_claim_execution";
pub const FF_RENEW_LEASE: &str = "ff_renew_lease";
pub const FF_CREATE_FLOW: &str = "ff_create_flow";
pub const FF_CANCEL_FLOW: &str = "ff_cancel_flow";
pub const FF_CREATE_BUDGET: &str = "ff_create_budget";
pub const FF_REPORT_USAGE_AND_CHECK: &str = "ff_report_usage_and_check";
pub const FF_CREATE_QUOTA_POLICY: &str = "ff_create_quota_policy";
pub const FF_CHECK_ADMISSION_AND_RECORD: &str = "ff_check_admission_and_record";
pub const FF_RESET_BUDGET: &str = "ff_reset_budget";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_names_start_with_ff_prefix() {
        let names = [
            FF_CREATE_EXECUTION,
            FF_COMPLETE_EXECUTION,
            FF_FAIL_EXECUTION,
            FF_CANCEL_EXECUTION,
            FF_SUSPEND_EXECUTION,
            FF_RESUME_EXECUTION,
            FF_DELIVER_SIGNAL,
            FF_ISSUE_CLAIM_GRANT,
            FF_CLAIM_EXECUTION,
            FF_RENEW_LEASE,
            FF_CREATE_FLOW,
            FF_CANCEL_FLOW,
            FF_CREATE_BUDGET,
            FF_REPORT_USAGE_AND_CHECK,
            FF_CREATE_QUOTA_POLICY,
            FF_CHECK_ADMISSION_AND_RECORD,
            FF_RESET_BUDGET,
        ];
        for name in names {
            assert!(name.starts_with("ff_"), "{name} missing ff_ prefix");
        }
    }
}
