# STATUS: cost_aggregation_accuracy

**Task:** RFC 009 cost aggregation accuracy  
**Tests passed:** 6/6  
**File:** `crates/cairn-store/tests/cost_aggregation_accuracy.rs`

Tests:
- `five_calls_sum_to_1500_micros` — 100+200+300+400+500=1500 via RunCostReadModel + cost_summary()
- `token_counts_accumulate_correctly` — 750 tokens_in, 300 tokens_out
- `per_binding_cost_breakdown_is_accurate` — gpt-4o=6000, claude=1250, combined=7250
- `zero_cost_and_none_cost_dont_inflate_totals` — only paid call (1000µ) counts; all 3 calls counted; tokens from all
- `cost_micros_precision_no_floating_point_loss` — primes + near-round values = 11_000_011 exactly
- `cost_summary_aggregates_across_multiple_runs` — run1(1000)+run2(2500)=3500 cross-run total
