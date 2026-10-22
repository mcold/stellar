//! Project and LocalExchange operator rules (P001, P002, L001)
//!
//! P001: Project 表达式计算耗时高
//! P002: 公共子表达式计算耗时高 (CASE WHEN 等重复计算)
//! L001: LocalExchange 内存使用过高

use super::*;

/// P001: Project expression compute time high
pub struct P001ProjectExprHigh;

impl DiagnosticRule for P001ProjectExprHigh {
    fn id(&self) -> &str {
        "P001"
    }
    fn name(&self) -> &str {
        "Project expression compute time high"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("PROJECT")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let expr_time = context.get_metric("ExprComputeTime")?;
        let op_time = context.get_operator_time_ms()? * 1_000_000.0;
        if op_time == 0.0 {
            return None;
        }
        let ratio = expr_time / op_time;
        if ratio > 0.5 && expr_time > 100_000_000.0 {
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
                message: format!("The Project expression compute takes {:.1}% of the time", ratio * 100.0),
                reason: "The Project operator takes too long, probably because the expressions are complex or the data volume is large.".to_string(),
                suggestions: vec![
                    "Simplify the complex expressions in the SELECT".to_string(),
                    "Move the complex computation into a materialized view".to_string(),
                    "Check for unnecessary type casts".to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// P002: Common sub-expression compute time high (CASE WHEN repeated calculation)
/// Source: project_operator.cpp - CommonSubExprComputeTime
pub struct P002CommonSubExprHigh;

impl DiagnosticRule for P002CommonSubExprHigh {
    fn id(&self) -> &str {
        "P002"
    }
    fn name(&self) -> &str {
        "Common sub-expression compute time high"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("PROJECT")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let common_sub_expr_time = context
            .get_metric("CommonSubExprComputeTime")
            .or_else(|| context.get_metric_duration("CommonSubExprComputeTime"))?;

        let time_ms = if common_sub_expr_time > 1_000_000.0 {
            common_sub_expr_time / 1_000_000.0
        } else {
            common_sub_expr_time
        };

        if time_ms < 500.0 {
            return None;
        }

        let expr_time = context
            .get_metric("ExprComputeTime")
            .or_else(|| context.get_metric_duration("ExprComputeTime"))
            .unwrap_or(0.0);
        let expr_time_ms =
            if expr_time > 1_000_000.0 { expr_time / 1_000_000.0 } else { expr_time };

        let total_expr_time = time_ms + expr_time_ms;
        let common_ratio =
            if total_expr_time > 0.0 { time_ms / total_expr_time * 100.0 } else { 0.0 };

        let severity = if time_ms > 5000.0 { RuleSeverity::Error } else { RuleSeverity::Warning };

        Some(Diagnostic {
            rule_id: self.id().to_string(),
            rule_name: self.name().to_string(),
            severity,
            node_path: format!(
                "{} (plan_node_id={})",
                context.node.operator_name,
                context.node.plan_node_id.unwrap_or(-1)
            ),
            plan_node_id: context.node.plan_node_id,
            message: format!(
                "The common sub-expression compute took {} ({:.1}% of the total expression time)",
                format_duration_ms(time_ms), common_ratio
            ),
            reason: "Complex CASE WHEN expressions or repeated sub-expressions make the compute cost high. StarRocks tries to extract the common sub-expressions to avoid the repeated computation, but the extraction itself also costs time.".to_string(),
            suggestions: vec![
                "Simplify the CASE WHEN expressions and reduce the number of branches".to_string(),
                "Move the complex condition evaluation into a materialized view".to_string(),
                "Check whether a lot of expressions are computed repeatedly".to_string(),
                "Consider IF() instead of a simple CASE WHEN".to_string(),
            ],
            parameter_suggestions: vec![],
                threshold_metadata: None,
        })
    }
}

/// L001: LocalExchange memory too high
pub struct L001LocalExchangeMemory;

impl DiagnosticRule for L001LocalExchangeMemory {
    fn id(&self) -> &str {
        "L001"
    }
    fn name(&self) -> &str {
        "LocalExchange memory usage too high"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("LOCAL")
            && node.operator_name.to_uppercase().contains("EXCHANGE")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let memory = context
            .get_metric("LocalExchangePeakMemoryUsage")
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
                message: format!("LocalExchange memory usage {}", format_bytes(memory as u64)),
                reason: "See the StarRocks official documentation for more information.".to_string(),
                suggestions: vec![
                    "Check whether the data flow between the upstream and the downstream operators is balanced".to_string(),
                    "Adjust the pipeline_dop parameter".to_string(),
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

pub fn get_rules() -> Vec<Box<dyn DiagnosticRule>> {
    vec![
        Box::new(P001ProjectExprHigh),
        Box::new(P002CommonSubExprHigh),
        Box::new(L001LocalExchangeMemory),
    ]
}
