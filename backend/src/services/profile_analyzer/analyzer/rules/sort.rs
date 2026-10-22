//! Sort/Merge operator diagnostic rules (T001-T005, W001)
//!
//! Rules for SORT and ANALYTIC (Window) operators.

use super::*;

/// T001: Sort rows too large
/// Condition: InputRowNum > 10M without LIMIT
pub struct T001SortRowsTooLarge;

impl DiagnosticRule for T001SortRowsTooLarge {
    fn id(&self) -> &str {
        "T001"
    }
    fn name(&self) -> &str {
        "Too many rows sorted"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("SORT")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let input_rows = context.node.metrics.push_row_num? as f64;

        if input_rows > 10_000_000.0 {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Warning,
                node_path: format!("{} (plan_node_id={})", 
                    context.node.operator_name,
                    context.node.plan_node_id.unwrap_or(-1)),
                plan_node_id: context.node.plan_node_id,
                message: format!(
                    "Too many rows sorted ({:.0} rows), which may cause performance problems",
                    input_rows
                ),
                reason: "The sorted data volume is too large and consumes a lot of CPU and memory. Consider adding filter conditions to reduce the sorted volume or use a TopN optimization.".to_string(),
                suggestions: vec![
                    "Add LIMIT to bound the result set size".to_string(),
                    "Check whether Top-N optimization applies".to_string(),
                    "Consider a pre-sorted materialized view".to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// T002: Sort spill detected
/// Condition: SpillBytes > 0
pub struct T002SortSpill;

impl DiagnosticRule for T002SortSpill {
    fn id(&self) -> &str {
        "T002"
    }
    fn name(&self) -> &str {
        "Sort spilled to disk"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("SORT")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let spill_bytes = context
            .get_metric("SpillBytes")
            .or_else(|| context.get_metric("OperatorSpillBytes"))?;

        if spill_bytes > 0.0 {
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
                    "The sort spilled to disk, {} written",
                    format_bytes(spill_bytes as u64)
                ),
                reason: "The sorted data volume exceeds the memory limit and triggers a disk spill. A spill slows the sort down considerably."
                    .to_string(),
                suggestions: vec![
                    "Raise the memory limit to avoid the spill".to_string(),
                    "Add LIMIT to reduce the sorted data volume".to_string(),
                    "Check whether the query can be optimized to sort less data".to_string(),
                ],
                parameter_suggestions: {
                    let mut suggestions = Vec::new();
                    if let Some(s) = context.suggest_parameter_smart("query_mem_limit") {
                        suggestions.push(s);
                    }
                    if let Some(s) = context.suggest_parameter_smart("enable_spill") {
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

/// T003: Sort memory too high
/// Condition: OperatorPeakMemoryUsage > 1GB
pub struct T003SortMemoryHigh;

impl DiagnosticRule for T003SortMemoryHigh {
    fn id(&self) -> &str {
        "T003"
    }
    fn name(&self) -> &str {
        "Sort memory usage too high"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("SORT")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let memory = context
            .get_metric("OperatorPeakMemoryUsage")
            .or_else(|| context.get_memory_usage().map(|v| v as f64))?;

        const ONE_GB: f64 = 1024.0 * 1024.0 * 1024.0;

        if memory > ONE_GB {
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
                message: format!("Sort memory usage {}", format_bytes(memory as u64)),
                reason: "The sort takes too much memory, which may cause memory pressure or hurt the other operators.".to_string(),
                suggestions: vec![
                    "Add LIMIT to reduce the sorted data volume".to_string(),
                    "Enable the spill feature to avoid OOM".to_string(),
                    "Consider processing in batches".to_string(),
                ],
                parameter_suggestions: {
                    let mut suggestions = Vec::new();
                    if let Some(s) = context.suggest_parameter_smart("enable_spill") {
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

/// W001: Window function memory too high
/// Condition: Memory > 500MB for ANALYTIC operator
pub struct W001WindowMemoryHigh;

impl DiagnosticRule for W001WindowMemoryHigh {
    fn id(&self) -> &str {
        "W001"
    }
    fn name(&self) -> &str {
        "Window function memory usage too high"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        let name = node.operator_name.to_uppercase();
        name.contains("ANALYTIC") || name.contains("WINDOW")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let memory = context.get_memory_usage()?;
        const THRESHOLD: u64 = 500 * 1024 * 1024;

        if memory > THRESHOLD {
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
                message: format!("Window function memory usage {}", format_bytes(memory)),
                reason: "The window function takes too much memory, probably because the window partition is too large or the window function state is too large.".to_string(),
                suggestions: vec![
                    "Check whether the PARTITION BY cardinality is too high".to_string(),
                    "Consider a smaller window".to_string(),
                    "Check whether an aggregate function can be used instead".to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// T004: Sort merging time too long
pub struct T004SortMergingTimeLong;

impl DiagnosticRule for T004SortMergingTimeLong {
    fn id(&self) -> &str {
        "T004"
    }
    fn name(&self) -> &str {
        "Sort merge phase takes too long"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("SORT")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let merging_time = context.get_metric("MergingTime")?;
        let op_time = context.get_operator_time_ms()? * 1_000_000.0;
        if op_time == 0.0 {
            return None;
        }
        let ratio = merging_time / op_time;
        if ratio > 0.3 {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Info,
                node_path: format!(
                    "{} (plan_node_id={})",
                    context.node.operator_name,
                    context.node.plan_node_id.unwrap_or(-1)
                ),
                plan_node_id: context.node.plan_node_id,
                message: format!("The sort merge phase takes {:.1}% of the time", ratio * 100.0),
                reason: "The multi-way merge sort takes too long, probably because there are too many merge streams or each of them holds too much data.".to_string(),
                suggestions: vec![
                    "Check whether the parallelism setting is reasonable".to_string(),
                    "Consider reducing the number of partitions".to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// T005: Merge waiting for upstream too long
pub struct T005MergeWaitingLong;

impl DiagnosticRule for T005MergeWaitingLong {
    fn id(&self) -> &str {
        "T005"
    }
    fn name(&self) -> &str {
        "Merge waits too long for the upstream"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("MERGE")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let pending_time = context.get_metric("6-PendingStageTime")?;
        let overall_time = context.get_metric("OverallStageTime").unwrap_or(1.0);
        if overall_time == 0.0 {
            return None;
        }
        let ratio = pending_time / overall_time;
        if ratio > 0.3 {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Info,
                node_path: format!(
                    "{} (plan_node_id={})",
                    context.node.operator_name,
                    context.node.plan_node_id.unwrap_or(-1)
                ),
                plan_node_id: context.node.plan_node_id,
                message: format!("The merge waits {:.1}% of the time for the upstream", ratio * 100.0),
                reason: "The Merge operator waits too long for the upstream data, so the upstream operator may have a performance bottleneck.".to_string(),
                suggestions: vec![
                    "Optimize the producer operator first".to_string(),
                    "Enlarge the pipeline buffer".to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// Get all sort rules
pub fn get_rules() -> Vec<Box<dyn DiagnosticRule>> {
    vec![
        Box::new(T001SortRowsTooLarge),
        Box::new(T002SortSpill),
        Box::new(T003SortMemoryHigh),
        Box::new(T004SortMergingTimeLong),
        Box::new(T005MergeWaitingLong),
        Box::new(W001WindowMemoryHigh),
    ]
}
