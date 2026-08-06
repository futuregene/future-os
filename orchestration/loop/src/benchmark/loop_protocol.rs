//! Benchmark loop protocol (G-18) — reference `benchmark_core/loop_protocol.py`
//! (689 lines), the minimal deterministic core.
//!
//! The contract answers: which route is being benchmarked, under which
//! protocol id, with what round budget, and whether any comparison claim is
//! allowed. A route is classified into blind-loop / product-mode /
//! packet-only-observation; `max_rounds_budget` defaults to the LoopX
//! blind-loop budget of 5; routes that cannot support the strict treatment
//! claim carry a `claim_blocker`.

/// Loop protocol schema versions (reference constants).
pub const BENCHMARK_LOOP_PROTOCOL_SCHEMA_VERSION: &str = "benchmark_loop_protocol_v0";
pub const BENCHMARK_PRODUCT_MODE_COMPARISON_SCHEMA_VERSION: &str =
    "benchmark_product_mode_comparison_v0";

/// Protocol ids (reference `MAX5_BLIND_LOOP_NO_FEEDBACK_PROTOCOL_ID` etc).
pub const MAX5_BLIND_LOOP_NO_FEEDBACK_PROTOCOL_ID: &str = "max5_blind_loop_no_feedback";
pub const PRODUCT_MODE_MAX5_NO_FEEDBACK_PROTOCOL_ID: &str = "product_mode_max5_no_feedback";
pub const PACKET_ONLY_OBSERVATION_PROTOCOL_ID: &str = "packet_only_observation";

/// Default blind-loop round budget (reference BLIND_LOOP_DEFAULT_MAX_ROUNDS).
pub const BLIND_LOOP_DEFAULT_MAX_ROUNDS: u32 = 5;

/// Known routes (reference constants).
pub const CODEX_ACP_BLIND_LOOP_BASELINE_ROUTE: &str = "codex-acp-blind-loop-baseline";
pub const CODEX_CLI_GOAL_BASELINE_ROUTE: &str = "codex-cli-goal-baseline";
pub const RAW_CODEX_AUTONOMOUS_MAX5_ROUTE: &str = "raw-codex-autonomous-max5";
pub const LOOPX_PRODUCT_MODE_ROUTE: &str = "future-loop-product-mode";
pub const LOOPX_GOAL_START_PRODUCT_MODE_ROUTE: &str = "future-loop-goal-start-product-mode";
pub const LOOPX_TURN_AGENT_CLI_ROUTE: &str = "future-loop-turn-agent-cli";
pub const CODEX_APP_SERVER_GOAL_BASELINE_ROUTE: &str = "codex-app-server-goal-baseline";
pub const LOOPX_PACKET_ONLY_OBSERVATION_ROUTE: &str = "future-loop-packet-only-observation";

/// Routes that pre-date the product-mode controller and are invalid for a
/// strict comparison claim (reference LEGACY_NONPRODUCT_PROMPT_POLLING_ROUTES).
pub const LEGACY_NONPRODUCT_PROMPT_POLLING_ROUTES: &[&str] = &[
    "future-loop-blind-loop-treatment",
    "future-loop-prompt-polling-test",
];

/// Blind-loop routes: no official feedback is forwarded during the loop.
pub fn blind_loop_routes() -> &'static [&'static str] {
    &[
        CODEX_ACP_BLIND_LOOP_BASELINE_ROUTE,
        CODEX_CLI_GOAL_BASELINE_ROUTE,
        "future-loop-blind-loop-treatment",
        "future-loop-prompt-polling-test",
    ]
}

/// Product-mode routes: the reference state/todo/replan CLI surface is the agent
/// surface (reference PRODUCT_MODE_ROUTES).
pub fn product_mode_routes() -> &'static [&'static str] {
    &[
        RAW_CODEX_AUTONOMOUS_MAX5_ROUTE,
        LOOPX_PRODUCT_MODE_ROUTE,
        LOOPX_GOAL_START_PRODUCT_MODE_ROUTE,
        LOOPX_TURN_AGENT_CLI_ROUTE,
    ]
}

/// The benchmark loop contract: route + protocol + budget + claim validity.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BenchmarkLoopContract {
    pub schema_version: String,
    pub route: String,
    pub protocol_id: String,
    pub max_rounds_budget: u32,
    /// Official benchmark feedback (the verifier's reward) is never forwarded
    /// to the agent mid-loop — the loop stays blind.
    pub official_feedback_forwarded: bool,
    pub official_feedback_blinded: bool,
    pub blind_loop: bool,
    pub product_mode: bool,
    /// A strict treatment comparison claim (product-mode main table) requires
    /// a product-mode route without a claim blocker.
    pub strict_treatment_claim_allowed: bool,
    /// Non-empty when the route cannot support the strict treatment claim
    /// (historical non-product routes, packet-only observation).
    pub claim_blocker: String,
}

impl BenchmarkLoopContract {
    /// Render as the reference `as_dict()` surface (schema-versioned).
    pub fn as_dict(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }
}

fn route_in(routes: &[&str], route: &str) -> bool {
    routes.contains(&route)
}

/// Build the loop contract for a route (reference `build_benchmark_loop_contract`).
/// `max_rounds` defaults to the blind-loop budget; a positive non-default
/// budget resolves to `custom_or_legacy_loop` unless a protocol is given.
pub fn build_benchmark_loop_contract(
    route: &str,
    max_rounds: Option<u32>,
    protocol_id: Option<&str>,
) -> BenchmarkLoopContract {
    let budget = match max_rounds {
        Some(n) if n > 0 => n,
        _ => BLIND_LOOP_DEFAULT_MAX_ROUNDS,
    };
    let blind = route_in(blind_loop_routes(), route);
    let product = route_in(product_mode_routes(), route);
    let resolved = match protocol_id {
        Some(id) => id.to_string(),
        None => {
            if blind && budget == BLIND_LOOP_DEFAULT_MAX_ROUNDS {
                MAX5_BLIND_LOOP_NO_FEEDBACK_PROTOCOL_ID.to_string()
            } else if product && budget == BLIND_LOOP_DEFAULT_MAX_ROUNDS {
                PRODUCT_MODE_MAX5_NO_FEEDBACK_PROTOCOL_ID.to_string()
            } else if route == LOOPX_PACKET_ONLY_OBSERVATION_ROUTE {
                PACKET_ONLY_OBSERVATION_PROTOCOL_ID.to_string()
            } else {
                "custom_or_legacy_loop".to_string()
            }
        }
    };
    let (claim_blocker, strict_allowed) =
        if route_in(LEGACY_NONPRODUCT_PROMPT_POLLING_ROUTES, route) {
            (
                "historical_nonproduct_invalid_for_comparison".to_string(),
                false,
            )
        } else if route == LOOPX_PACKET_ONLY_OBSERVATION_ROUTE {
            ("packet_only_no_max5_controller".to_string(), false)
        } else {
            (String::new(), product)
        };
    BenchmarkLoopContract {
        schema_version: BENCHMARK_LOOP_PROTOCOL_SCHEMA_VERSION.to_string(),
        route: route.to_string(),
        protocol_id: resolved,
        max_rounds_budget: budget,
        official_feedback_forwarded: false,
        official_feedback_blinded: true,
        blind_loop: blind,
        product_mode: product,
        strict_treatment_claim_allowed: strict_allowed,
        claim_blocker,
    }
}

/// The product-mode main-table comparison contract: baseline arm (raw codex
/// autonomous max5) vs treatment arm (loopx product mode), with the policy
/// gate reference requires before a headline comparison is allowed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProductModeComparisonContract {
    pub schema_version: String,
    pub comparison_id: String,
    pub benchmark_id: String,
    pub protocol_id: String,
    pub max_rounds_budget: u32,
    pub baseline_arm: serde_json::Value,
    pub treatment_arm: serde_json::Value,
    pub policy_gate: serde_json::Value,
}

/// Build the skillsbench product-mode main-table comparison contract
/// (reference `build_product_mode_main_table_comparison_contract`, minimal).
pub fn build_product_mode_main_table_comparison_contract(
    benchmark_id: &str,
    max_rounds: Option<u32>,
    baseline_route: &str,
    treatment_route: &str,
) -> ProductModeComparisonContract {
    let budget = match max_rounds {
        Some(n) if n > 0 => n,
        _ => BLIND_LOOP_DEFAULT_MAX_ROUNDS,
    };
    let baseline = build_benchmark_loop_contract(
        baseline_route,
        Some(budget),
        if baseline_route == RAW_CODEX_AUTONOMOUS_MAX5_ROUTE {
            Some(PRODUCT_MODE_MAX5_NO_FEEDBACK_PROTOCOL_ID)
        } else {
            None
        },
    );
    let treatment = build_benchmark_loop_contract(
        treatment_route,
        Some(budget),
        if matches!(
            treatment_route,
            LOOPX_PRODUCT_MODE_ROUTE | LOOPX_GOAL_START_PRODUCT_MODE_ROUTE
        ) {
            Some(PRODUCT_MODE_MAX5_NO_FEEDBACK_PROTOCOL_ID)
        } else {
            None
        },
    );
    let treatment_arm_id = if treatment_route == LOOPX_GOAL_START_PRODUCT_MODE_ROUTE {
        "future_loop_goal_start_product_mode"
    } else {
        "future_loop_product_mode"
    };
    let treatment_agent_surface = if treatment_route == LOOPX_GOAL_START_PRODUCT_MODE_ROUTE {
        "future_loop_goal_start_plan_todo_lifecycle_cli"
    } else {
        "future_loop_state_todo_replan_cli"
    };
    ProductModeComparisonContract {
        schema_version: BENCHMARK_PRODUCT_MODE_COMPARISON_SCHEMA_VERSION.to_string(),
        comparison_id: "skillsbench_product_mode_main_table_v0".to_string(),
        benchmark_id: benchmark_id.to_string(),
        protocol_id: PRODUCT_MODE_MAX5_NO_FEEDBACK_PROTOCOL_ID.to_string(),
        max_rounds_budget: budget,
        baseline_arm: serde_json::json!({
            "route": baseline_route,
            "arm_id": "raw_codex_autonomous_max5",
            "contract": baseline.as_dict(),
            "future_loop_cli_allowed": false,
            "agent_surface": "raw_codex_autonomous",
        }),
        treatment_arm: serde_json::json!({
            "route": treatment_route,
            "arm_id": treatment_arm_id,
            "contract": treatment.as_dict(),
            "future_loop_state_todo_replan_cli_required": true,
            "case_local_future_loop_state_required": true,
            "future_loop_cli_required": true,
            "agent_surface": treatment_agent_surface,
        }),
        policy_gate: serde_json::json!({
            "same_benchmark_and_case_required": true,
            "official_feedback_forwarded_to_agent": false,
            "official_feedback_blinded": true,
            "reward_feedback_forwarded": false,
            "stop_on_reward_one": true,
            "stop_on_agent_declared_done_no_remaining_goals": true,
            "stop_on_max_rounds_budget": true,
            "headline_metrics": [
                "best_score",
                "final_score",
                "first_success_round",
                "declared_done_score",
            ],
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_mode_route_resolves_product_protocol_and_allows_claim() {
        let c = build_benchmark_loop_contract(LOOPX_PRODUCT_MODE_ROUTE, None, None);
        assert_eq!(c.protocol_id, PRODUCT_MODE_MAX5_NO_FEEDBACK_PROTOCOL_ID);
        assert_eq!(c.max_rounds_budget, 5);
        assert!(c.product_mode);
        assert!(!c.blind_loop);
        assert!(c.official_feedback_blinded);
        assert!(!c.official_feedback_forwarded);
        assert!(c.strict_treatment_claim_allowed);
        assert!(c.claim_blocker.is_empty());
    }

    #[test]
    fn blind_loop_baseline_resolves_blind_protocol() {
        let c = build_benchmark_loop_contract(CODEX_ACP_BLIND_LOOP_BASELINE_ROUTE, None, None);
        assert_eq!(c.protocol_id, MAX5_BLIND_LOOP_NO_FEEDBACK_PROTOCOL_ID);
        assert!(c.blind_loop);
        assert!(!c.product_mode);
        assert!(!c.strict_treatment_claim_allowed);
    }

    #[test]
    fn packet_only_observation_gets_claim_blocker() {
        let c = build_benchmark_loop_contract(LOOPX_PACKET_ONLY_OBSERVATION_ROUTE, None, None);
        assert_eq!(c.protocol_id, PACKET_ONLY_OBSERVATION_PROTOCOL_ID);
        assert_eq!(c.claim_blocker, "packet_only_no_max5_controller");
        assert!(!c.strict_treatment_claim_allowed);
    }

    #[test]
    fn legacy_nonproduct_route_is_blocked_for_comparison() {
        let c = build_benchmark_loop_contract("future-loop-blind-loop-treatment", None, None);
        assert_eq!(
            c.claim_blocker,
            "historical_nonproduct_invalid_for_comparison"
        );
        assert!(!c.strict_treatment_claim_allowed);
    }

    #[test]
    fn custom_budget_resolves_to_custom_protocol() {
        let c = build_benchmark_loop_contract(LOOPX_PRODUCT_MODE_ROUTE, Some(3), None);
        assert_eq!(c.max_rounds_budget, 3);
        assert_eq!(c.protocol_id, "custom_or_legacy_loop");
        // explicit protocol wins
        let c = build_benchmark_loop_contract(
            LOOPX_PRODUCT_MODE_ROUTE,
            Some(3),
            Some(PRODUCT_MODE_MAX5_NO_FEEDBACK_PROTOCOL_ID),
        );
        assert_eq!(c.protocol_id, PRODUCT_MODE_MAX5_NO_FEEDBACK_PROTOCOL_ID);
    }

    #[test]
    fn invalid_budget_falls_back_to_default() {
        let c = build_benchmark_loop_contract(LOOPX_PRODUCT_MODE_ROUTE, Some(0), None);
        assert_eq!(c.max_rounds_budget, BLIND_LOOP_DEFAULT_MAX_ROUNDS);
    }

    #[test]
    fn product_mode_comparison_contract_arms_and_gate() {
        let c = build_product_mode_main_table_comparison_contract(
            "skillsbench@1.1",
            None,
            RAW_CODEX_AUTONOMOUS_MAX5_ROUTE,
            LOOPX_PRODUCT_MODE_ROUTE,
        );
        assert_eq!(c.comparison_id, "skillsbench_product_mode_main_table_v0");
        assert_eq!(c.policy_gate["headline_metrics"][0], "best_score");
        assert_eq!(c.baseline_arm["route"], RAW_CODEX_AUTONOMOUS_MAX5_ROUTE);
        assert_eq!(c.treatment_arm["arm_id"], "future_loop_product_mode");
        assert_eq!(
            c.treatment_arm["agent_surface"],
            "future_loop_state_todo_replan_cli"
        );
        // goal-start variant
        let c2 = build_product_mode_main_table_comparison_contract(
            "skillsbench@1.1",
            None,
            RAW_CODEX_AUTONOMOUS_MAX5_ROUTE,
            LOOPX_GOAL_START_PRODUCT_MODE_ROUTE,
        );
        assert_eq!(
            c2.treatment_arm["arm_id"],
            "future_loop_goal_start_product_mode"
        );
        assert_eq!(
            c2.treatment_arm["agent_surface"],
            "future_loop_goal_start_plan_todo_lifecycle_cli"
        );
    }
}
