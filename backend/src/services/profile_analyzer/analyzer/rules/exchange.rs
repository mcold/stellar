//! Exchange operator diagnostic rules (E001-E003)
//!
//! Rules for EXCHANGE_SINK and EXCHANGE_SOURCE operators.

use super::*;

/// E001: Network transfer too large
/// Condition: BytesSent > 1GB
pub struct E001NetworkTransferLarge;

impl DiagnosticRule for E001NetworkTransferLarge {
    fn id(&self) -> &str {
        "E001"
    }
    fn name(&self) -> &str {
        "Large network transfer volume"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("EXCHANGE")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let bytes_sent = context
            .get_metric("BytesSent")
            .or_else(|| context.get_metric("NetworkBytesSent"))?;

        const ONE_GB: f64 = 1024.0 * 1024.0 * 1024.0;

        if bytes_sent > ONE_GB {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Warning,
                node_path: format!("{} (plan_node_id={})", 
                    context.node.operator_name,
                    context.node.plan_node_id.unwrap_or(-1)),
                plan_node_id: context.node.plan_node_id,
                message: format!(
                    "{} transferred over the network, there may be a network bottleneck",
                    format_bytes(bytes_sent as u64)
                ),
                reason: "The network transfer volume is too large and takes a lot of bandwidth and time. This is usually caused by a large Shuffle volume or by a lack of effective data pruning.".to_string(),
                suggestions: vec![
                    "Check whether the Shuffle volume can be reduced".to_string(),
                    "Consider a Colocate Join to avoid the Shuffle".to_string(),
                    "Check whether the network bandwidth is sufficient".to_string(),
                ],
                parameter_suggestions: {
                    let mut suggestions = Vec::new();

                    if let Some(s) = context.suggest_parameter_smart("parallel_fragment_exec_instance_num") {
                        suggestions.push(s);
                    }

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

/// E002: Network time too high
/// Condition: NetworkTime/OperatorTime > 0.5
pub struct E002NetworkTimeHigh;

impl DiagnosticRule for E002NetworkTimeHigh {
    fn id(&self) -> &str {
        "E002"
    }
    fn name(&self) -> &str {
        "High network time share"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("EXCHANGE")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let network_time = context
            .get_metric("NetworkTime")
            .or_else(|| context.get_metric("OverallThroughputSendTime"))?;
        let operator_time = context.get_operator_time_ms()?;

        if operator_time == 0.0 {
            return None;
        }

        let ratio = network_time / operator_time;

        if ratio > 0.5 {
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
                message: format!("Network time share {:.1}%, there may be a network bottleneck", ratio * 100.0),
                reason: "The network transfer takes too large a share of the time, so the query bottleneck is the network. This is usually caused by insufficient network bandwidth or by cross-datacenter traffic."
                    .to_string(),
                suggestions: vec![
                    "Check the network bandwidth and latency".to_string(),
                    "Consider a Colocate Join".to_string(),
                    "Reduce the data transferred across nodes".to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// E003: Data skew in shuffle
/// Condition: max(BytesSent)/avg > 2
pub struct E003ShuffleSkew;

impl DiagnosticRule for E003ShuffleSkew {
    fn id(&self) -> &str {
        "E003"
    }
    fn name(&self) -> &str {
        "Shuffle data skew"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("EXCHANGE")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let max_bytes = context.get_metric("__MAX_OF_BytesSent")?;
        let min_bytes = context.get_metric("__MIN_OF_BytesSent").unwrap_or(0.0);

        if min_bytes == 0.0 {
            return None;
        }

        let avg_bytes = (max_bytes + min_bytes) / 2.0;
        let ratio = max_bytes / avg_bytes;

        if ratio > 2.0 {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Warning,
                node_path: format!("{} (plan_node_id={})", 
                    context.node.operator_name,
                    context.node.plan_node_id.unwrap_or(-1)),
                plan_node_id: context.node.plan_node_id,
                message: format!(
                    "The Shuffle has data skew, max/avg ratio is {:.2}",
                    ratio
                ),
                reason: "The Shuffle data is unevenly distributed across the nodes, so some nodes receive more data. This is usually caused by hot values in the Shuffle key.".to_string(),
                suggestions: vec![
                    "Check whether the partition key choice is reasonable".to_string(),
                    "Consider a Skew Join optimization".to_string(),
                    "Check whether hot data exists".to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// Get all exchange rules
pub fn get_rules() -> Vec<Box<dyn DiagnosticRule>> {
    vec![
        Box::new(E001NetworkTransferLarge),
        Box::new(E002NetworkTimeHigh),
        Box::new(E003ShuffleSkew),
    ]
}
