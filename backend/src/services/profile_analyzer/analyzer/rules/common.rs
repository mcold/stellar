//! Common diagnostic rules (G001-G003)
//!
//! These rules apply to all operator types.

use super::*;
use crate::services::profile_analyzer::analyzer::thresholds::defaults::{
    MIN_OPERATOR_TIME_MS, MOST_CONSUMING_PERCENTAGE, SECOND_CONSUMING_PERCENTAGE,
};

/// G001: Time percentage too high (most consuming node)
/// Threshold: > 30% AND > 500ms (aligned with StarRocks ExplainAnalyzer.java)
/// P0.2: Added absolute time threshold to avoid false positives on fast operators
pub struct G001MostConsuming;

impl DiagnosticRule for G001MostConsuming {
    fn id(&self) -> &str {
        "G001"
    }
    fn name(&self) -> &str {
        "Operator time share too high"
    }

    fn applicable_to(&self, _node: &ExecutionTreeNode) -> bool {
        true
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let percentage = context.get_time_percentage()?;
        let operator_time_ms = context.get_operator_time_ms()?;

        if percentage > MOST_CONSUMING_PERCENTAGE && operator_time_ms > MIN_OPERATOR_TIME_MS {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Error,
                node_path: format!("{} (plan_node_id={})", 
                    context.node.operator_name,
                    context.node.plan_node_id.unwrap_or(-1)),
                plan_node_id: context.node.plan_node_id,
                message: format!(
                    "🔴 Operator {} consumes {:.1}% of the execution time (most time-consuming node)",
                    context.node.operator_name, percentage
                ),
                suggestions: get_operator_suggestions(&context.node.operator_name),
                reason: "This operator takes too large a share of the overall query time and is the main bottleneck. Optimizing it yields the largest gain.".to_string(),
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// G001b: Time percentage high (second most consuming node)
/// Threshold: > 15% AND > 500ms (aligned with StarRocks ExplainAnalyzer.java)
/// P0.2: Added absolute time threshold to avoid false positives on fast operators
pub struct G001bSecondConsuming;

impl DiagnosticRule for G001bSecondConsuming {
    fn id(&self) -> &str {
        "G001b"
    }
    fn name(&self) -> &str {
        "Operator time share is high"
    }

    fn applicable_to(&self, _node: &ExecutionTreeNode) -> bool {
        true
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let percentage = context.get_time_percentage()?;
        let operator_time_ms = context.get_operator_time_ms()?;

        if percentage > SECOND_CONSUMING_PERCENTAGE
            && percentage <= MOST_CONSUMING_PERCENTAGE
            && operator_time_ms > MIN_OPERATOR_TIME_MS
        {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Warning,
                node_path: format!("{} (plan_node_id={})", 
                    context.node.operator_name,
                    context.node.plan_node_id.unwrap_or(-1)),
                plan_node_id: context.node.plan_node_id,
                message: format!(
                    "🟠 Operator {} consumes {:.1}% of the execution time (second most time-consuming node)",
                    context.node.operator_name, percentage
                ),
                suggestions: get_operator_suggestions(&context.node.operator_name),
                reason: "This operator takes too large a share of the overall query time and is the main bottleneck. Optimizing it yields the largest gain.".to_string(),
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// G002: Memory usage too high
/// Threshold: dynamic based on BE memory (10% of BE memory, clamped to 1GB-10GB)
/// v2.0: Uses dynamic memory threshold based on cluster configuration
pub struct G002HighMemory;

impl DiagnosticRule for G002HighMemory {
    fn id(&self) -> &str {
        "G002"
    }
    fn name(&self) -> &str {
        "Operator memory usage too high"
    }

    fn applicable_to(&self, _node: &ExecutionTreeNode) -> bool {
        true
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let memory = context.get_memory_usage()?;

        let memory_threshold = context.thresholds.get_operator_memory_threshold();

        if memory > memory_threshold {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Warning,
                node_path: format!("{} (plan_node_id={})", 
                    context.node.operator_name,
                    context.node.plan_node_id.unwrap_or(-1)),
                plan_node_id: context.node.plan_node_id,
                message: format!(
                    "Operator {} memory usage too high: {} (threshold: {})",
                    context.node.operator_name, format_bytes(memory), format_bytes(memory_threshold)
                ),
                reason: "Operator memory usage is too high, which may fail the query or trigger a spill. Check for data inflation or oversized intermediate results.".to_string(),
                suggestions: vec![
                    "Check for data inflation".to_string(),
                    "Consider processing in batches".to_string(),
                    "Check whether the HashTable or intermediate results are too large".to_string(),
                ],
                parameter_suggestions: {
                    let mut suggestions = Vec::new();
                    if let Some(s) = context.suggest_parameter_smart("query_mem_limit") {
                        suggestions.push(s);
                    }
                    suggestions
                },
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// G003: Execution time skew across instances
/// Threshold: max/avg > threshold (dynamic based on cluster size)
/// P0.2: Added absolute value protection (min 500ms execution time)
/// v2.0: Uses dynamic skew threshold based on cluster parallelism
pub struct G003ExecutionSkew;

impl DiagnosticRule for G003ExecutionSkew {
    fn id(&self) -> &str {
        "G003"
    }
    fn name(&self) -> &str {
        "Operator execution time skew"
    }

    fn applicable_to(&self, _node: &ExecutionTreeNode) -> bool {
        true
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let max_time = context.node.metrics.operator_total_time_max?;
        let _min_time = context.node.metrics.operator_total_time_min.unwrap_or(0);
        let avg_time = context.node.metrics.operator_total_time?;

        if avg_time == 0 {
            return None;
        }

        use crate::services::profile_analyzer::analyzer::thresholds::defaults::MIN_EXEC_TIME_NS;
        if avg_time < MIN_EXEC_TIME_NS {
            return None;
        }

        let ratio = max_time as f64 / avg_time as f64;

        let skew_threshold = context.thresholds.get_skew_threshold();

        if ratio > skew_threshold {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Warning,
                node_path: format!(
                    "{} (plan_node_id={})",
                    context.node.operator_name,
                    context.node.plan_node_id.unwrap_or(-1)
                ),
                plan_node_id: context.node.plan_node_id,
                message: format!(
                    "Operator {} has skewed execution time, max/avg ratio is {:.2} (threshold: {:.1})",
                    context.node.operator_name, ratio, skew_threshold
                ),
                reason:
                    "Execution time varies widely across the instances of the operator, and some instances become the bottleneck. Usually caused by uneven data distribution."
                        .to_string(),
                suggestions: vec![
                    "Check whether the data distribution is even".to_string(),
                    "Check whether the data partitions or buckets are reasonable".to_string(),
                    "Consider increasing parallelism".to_string(),
                ],
                parameter_suggestions: {
                    let mut suggestions = Vec::new();
                    if let Some(s) = context.suggest_parameter_smart("pipeline_dop") {
                        suggestions.push(s);
                    }
                    suggestions
                },
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// Get operator-specific suggestions based on operator name
fn get_operator_suggestions(operator_name: &str) -> Vec<String> {
    let name = operator_name.to_uppercase();

    if name.contains("SCAN") {
        vec![
            "Check whether filter conditions can be added to reduce the scanned data volume"
                .to_string(),
            "Check whether partition pruning is effective".to_string(),
            "Run ANALYZE TABLE to refresh statistics".to_string(),
        ]
    } else if name.contains("JOIN") {
        vec![
            "Check whether the JOIN order is optimal".to_string(),
            "Consider using a Runtime Filter".to_string(),
            "Check for data skew".to_string(),
            "Run ANALYZE TABLE to refresh statistics".to_string(),
        ]
    } else if name.contains("AGGREGATE") || name.contains("AGG") {
        vec![
            "Check whether the aggregation mode is appropriate".to_string(),
            "Consider pre-aggregation or a materialized view".to_string(),
            "Check the choice of GROUP BY keys".to_string(),
        ]
    } else if name.contains("EXCHANGE") {
        vec![
            "Check whether the data distribution is even".to_string(),
            "Consider adjusting parallelism".to_string(),
            "Check whether network bandwidth is sufficient".to_string(),
        ]
    } else if name.contains("SORT") {
        vec![
            "Add LIMIT to bound the result set size".to_string(),
            "Check whether Top-N optimization applies".to_string(),
            "Consider a pre-sorted materialized view".to_string(),
        ]
    } else {
        vec![
            "Check whether this operator processes too much data".to_string(),
            "Consider optimizing the query plan".to_string(),
        ]
    }
}

/// Get all common rules
pub fn get_rules() -> Vec<Box<dyn DiagnosticRule>> {
    vec![
        Box::new(G001MostConsuming),
        Box::new(G001bSecondConsuming),
        Box::new(G002HighMemory),
        Box::new(G003ExecutionSkew),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::profile_analyzer::analyzer::thresholds::{DynamicThresholds, QueryType};
    use crate::services::profile_analyzer::models::{
        ExecutionTreeNode, HotSeverity, NodeType, OperatorMetrics,
    };
    use std::collections::HashMap;

    #[test]
    fn test_g001_threshold() {
        let rule = G001MostConsuming;
        assert_eq!(rule.id(), "G001");
    }

    #[test]
    fn test_g001_triggers_on_high_percentage() {
        let rule = G001MostConsuming;

        let metrics =
            OperatorMetrics { operator_total_time: Some(1_000_000_000), ..Default::default() };

        let node = ExecutionTreeNode {
            id: "test_node".to_string(),
            operator_name: "OLAP_SCAN".to_string(),
            node_type: NodeType::OlapScan,
            plan_node_id: Some(0),
            parent_plan_node_id: None,
            metrics,
            children: vec![],
            depth: 0,
            is_hotspot: false,
            hotspot_severity: HotSeverity::Normal,
            fragment_id: None,
            pipeline_id: None,
            time_percentage: Some(99.84),
            rows: None,
            is_most_consuming: true,
            is_second_most_consuming: false,
            unique_metrics: HashMap::new(),
            has_diagnostic: false,
            diagnostic_ids: vec![],
        };

        let session_variables = std::collections::HashMap::new();
        let context = RuleContext {
            node: &node,
            session_variables: &session_variables,
            cluster_info: None,
            cluster_variables: None,
            default_db: None,
            thresholds: DynamicThresholds::with_defaults(QueryType::Select),
        };
        let result = rule.evaluate(&context);

        assert!(
            result.is_some(),
            "G001 should trigger for 99.84% time percentage with 1s operator time"
        );
        let diag = result.unwrap();
        assert_eq!(diag.rule_id, "G001");
        assert_eq!(diag.plan_node_id, Some(0));
    }

    #[test]
    fn test_g001_skips_fast_operator() {
        let rule = G001MostConsuming;

        let metrics =
            OperatorMetrics { operator_total_time: Some(100_000_000), ..Default::default() };

        let node = ExecutionTreeNode {
            id: "test_node".to_string(),
            operator_name: "OLAP_SCAN".to_string(),
            node_type: NodeType::OlapScan,
            plan_node_id: Some(0),
            parent_plan_node_id: None,
            metrics,
            children: vec![],
            depth: 0,
            is_hotspot: false,
            hotspot_severity: HotSeverity::Normal,
            fragment_id: None,
            pipeline_id: None,
            time_percentage: Some(50.0),
            rows: None,
            is_most_consuming: true,
            is_second_most_consuming: false,
            unique_metrics: HashMap::new(),
            has_diagnostic: false,
            diagnostic_ids: vec![],
        };

        let session_variables = std::collections::HashMap::new();
        let context = RuleContext {
            node: &node,
            session_variables: &session_variables,
            cluster_info: None,
            cluster_variables: None,
            default_db: None,
            thresholds: DynamicThresholds::with_defaults(QueryType::Select),
        };
        let result = rule.evaluate(&context);

        assert!(
            result.is_none(),
            "G001 should NOT trigger for fast operator (100ms < 500ms threshold)"
        );
    }
}
