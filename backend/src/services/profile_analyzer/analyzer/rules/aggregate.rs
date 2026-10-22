//! Aggregate operator diagnostic rules (A001-A005)
//!
//! Rules for AGGREGATE operators.

use super::*;

/// A001: Aggregation skew
/// Condition: max(AggComputeTime)/avg > threshold (dynamic based on cluster size)
/// P0.2: Added absolute value protection (min 100k rows aggregated)
/// v2.0: Uses dynamic skew threshold based on cluster parallelism
pub struct A001AggregationSkew;

impl DiagnosticRule for A001AggregationSkew {
    fn id(&self) -> &str {
        "A001"
    }
    fn name(&self) -> &str {
        "Aggregation data skew"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("AGG")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let max_time = context.node.metrics.operator_total_time_max?;
        let avg_time = context.node.metrics.operator_total_time?;

        if avg_time == 0 {
            return None;
        }

        let min_rows_threshold = context.thresholds.get_min_rows_for_skew();
        let input_rows = context.get_metric("PushRowNum").unwrap_or(0.0);

        if input_rows < min_rows_threshold {
            return None;
        }

        let ratio = max_time as f64 / avg_time as f64;

        let skew_threshold = context.thresholds.get_skew_threshold();

        if ratio > skew_threshold {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Warning,
                node_path: format!("{} (plan_node_id={})",
                    context.node.operator_name,
                    context.node.plan_node_id.unwrap_or(-1)),
                plan_node_id: context.node.plan_node_id,
                message: format!(
                    "The aggregation has data skew, max/avg ratio is {:.2} (threshold: {:.1})",
                    ratio, skew_threshold
                ),
                reason: "The instances of the aggregate operator process clearly different data volumes, and some instances become the bottleneck. This is usually caused by an uneven distribution of the GROUP BY keys.".to_string(),
                suggestions: vec![
                    "Check the data distribution of the GROUP BY keys".to_string(),
                    "Consider a two-phase aggregation".to_string(),
                    "Check whether hot keys exist".to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// A002: HashTable memory too large
/// Condition: HashTableMemoryUsage > threshold (dynamic based on BE memory)
/// v2.0: Uses dynamic hash table memory threshold (5% of BE memory, clamped to 512MB-5GB)
pub struct A002HashTableTooLarge;

impl DiagnosticRule for A002HashTableTooLarge {
    fn id(&self) -> &str {
        "A002"
    }
    fn name(&self) -> &str {
        "Aggregation HashTable too large"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("AGG")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let memory = context.get_memory_usage()?;

        let memory_threshold = context.thresholds.get_hash_table_memory_threshold();

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
                    "Aggregation HashTable memory usage {} (threshold: {})",
                    format_bytes(memory),
                    format_bytes(memory_threshold)
                ),
                reason: "The HashTable takes too much memory, which may cause memory pressure or trigger a spill. This is usually caused by a high GROUP BY key cardinality or by an oversized aggregation state.".to_string(),
                suggestions: vec![
                    "Check whether the GROUP BY cardinality is too high".to_string(),
                    "Consider pre-aggregating into a materialized view".to_string(),
                    "Enable the spill feature to avoid OOM".to_string(),
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

/// A004: High cardinality GROUP BY
/// Condition: HashTableSize > 10M
pub struct A004HighCardinality;

impl DiagnosticRule for A004HighCardinality {
    fn id(&self) -> &str {
        "A004"
    }
    fn name(&self) -> &str {
        "High cardinality GROUP BY"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("AGG")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let hash_size = context
            .get_metric("HashTableSize")
            .or_else(|| context.node.metrics.pull_row_num.map(|v| v as f64))?;

        if hash_size > 10_000_000.0 {
            let group_keys = context
                .get_group_by_keys()
                .unwrap_or_else(|| "Unknown".to_string());
            let memory = context.get_memory_usage().unwrap_or(0);

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
                message: format!("The GROUP BY cardinality is too high ({:.0} groups)", hash_size),
                reason: format!(
                    "The GROUP BY key 「{}」 has too high a cardinality ({:.0} distinct values), so the HashTable takes {} of memory and the aggregation slows down.",
                    group_keys,
                    hash_size,
                    format_bytes(memory)
                ),
                suggestions: vec![
                    format!("Check whether all the GROUP BY keys 「{}」 are needed and drop the unnecessary grouping columns", group_keys),
                    "Consider a streaming aggregation: SET streaming_preaggregation_mode = 'force_streaming'"
                        .to_string(),
                    "Consider a materialized view that pre-aggregates the common groupings".to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// A003: Aggregation data skew
pub struct A003DataSkew;

impl DiagnosticRule for A003DataSkew {
    fn id(&self) -> &str {
        "A003"
    }
    fn name(&self) -> &str {
        "Aggregation data skew"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("AGGREGATE")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let max_input = context.get_metric("__MAX_OF_InputRowCount")?;
        let min_input = context.get_metric("__MIN_OF_InputRowCount").unwrap_or(0.0);
        if min_input == 0.0 {
            return None;
        }
        let ratio = max_input / ((max_input + min_input) / 2.0);
        if ratio > 2.0 {
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
                message: format!("The aggregation has data skew, max/avg ratio is {:.2}", ratio),
                reason: "The input data of the aggregate operator is unevenly distributed across the instances, so some instances process much more data than the others."
                    .to_string(),
                suggestions: vec!["Optimize the choice of grouping keys".to_string(), "Consider handling the hot keys separately".to_string()],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// A005: Expensive key expression
pub struct A005ExpensiveKeyExpr;

impl DiagnosticRule for A005ExpensiveKeyExpr {
    fn id(&self) -> &str {
        "A005"
    }
    fn name(&self) -> &str {
        "GROUP BY key expression is expensive to compute"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("AGGREGATE")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let expr_time = context.get_metric("ExprComputeTime")?;
        let agg_time = context.get_metric("AggFuncComputeTime").unwrap_or(1.0);
        if agg_time == 0.0 {
            return None;
        }
        let ratio = expr_time / agg_time;
        if ratio > 0.5 && expr_time > 100_000_000.0 {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Info,
                node_path: format!("{} (plan_node_id={})", context.node.operator_name, context.node.plan_node_id.unwrap_or(-1)),
                plan_node_id: context.node.plan_node_id,
                message: format!("The GROUP BY key expression takes {:.1}% of the time", ratio * 100.0),
                reason: "The GROUP BY keys contain complex expressions that have to be computed for every row, which adds CPU overhead. Compute the expression beforehand or use a generated column.".to_string(),
                suggestions: vec![
                    "Materialize the complex expression in a subquery".to_string(),
                    "Promote the expression to a generated column".to_string(),
                    "Avoid complex functions in the GROUP BY".to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// A006: Low local aggregation efficiency
/// Condition: InputRowCount / OutputRowCount < 2.0 (aggregation reduces less than 50%)
pub struct A006LowLocalAggregation;

impl DiagnosticRule for A006LowLocalAggregation {
    fn id(&self) -> &str {
        "A006"
    }
    fn name(&self) -> &str {
        "Low local aggregation ratio"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        let name = node.operator_name.to_uppercase();

        name.contains("AGGREGATE") || name.contains("AGG")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let input_rows = context
            .get_metric("InputRowCount")
            .or_else(|| context.node.metrics.pull_row_num.map(|v| v as f64))?;
        let output_rows = context
            .get_metric("OutputRowCount")
            .or_else(|| context.node.metrics.push_row_num.map(|v| v as f64))?;

        if output_rows == 0.0 || input_rows == 0.0 {
            return None;
        }

        let agg_ratio = input_rows / output_rows;

        if agg_ratio < 2.0 && input_rows > 10_000.0 {
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
                    "Poor local aggregation, the aggregation ratio is {:.2}:1 (input {:.0} rows → output {:.0} rows)",
                    agg_ratio, input_rows, output_rows
                ),
                reason: {
                    let group_keys = context
                        .get_group_by_keys()
                        .unwrap_or_else(|| "Unknown".to_string());
                    format!(
                        "The local aggregation of GROUP BY「{}」 turned {:.0} input rows into only {:.0} output rows (reduction ratio {:.2}:1),\
                        so the data volume is not really reduced. This adds network traffic and downstream compute cost.",
                        group_keys, input_rows, output_rows, agg_ratio
                    )
                },
                suggestions: {
                    let group_keys = context
                        .get_group_by_keys()
                        .unwrap_or_else(|| "Unknown".to_string());
                    vec![
                        format!(
                            "The cardinality of GROUP BY「{}」 may be too high, consider disabling the two-phase aggregation: SET new_planner_agg_stage = 1",
                            group_keys
                        ),
                        "Check whether the GROUP BY keys contain high-cardinality columns (e.g. ID, timestamp)".to_string(),
                        "Consider pre-aggregating at write time or using a materialized view".to_string(),
                    ]
                },
                parameter_suggestions: vec![ParameterSuggestion::new(
                    "new_planner_agg_stage",
                    ParameterType::Session,
                    None,
                    "1",
                    "SET new_planner_agg_stage = 1; -- 关闭二阶段聚合",
                )],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// Get all aggregate rules
pub fn get_rules() -> Vec<Box<dyn DiagnosticRule>> {
    vec![
        Box::new(A001AggregationSkew),
        Box::new(A002HashTableTooLarge),
        Box::new(A003DataSkew),
        Box::new(A004HighCardinality),
        Box::new(A005ExpensiveKeyExpr),
        Box::new(A006LowLocalAggregation),
    ]
}
