//! Join operator diagnostic rules (J001-J010)
//!
//! Rules for HASH_JOIN, CROSS_JOIN, NEST_LOOP_JOIN operators.

use super::*;

/// J001: Join result explosion
/// Condition: PullRowNum > ProbeRows * 10
/// P0.2: Added absolute value protection (min 10k rows)
pub struct J001ResultExplosion;

impl DiagnosticRule for J001ResultExplosion {
    fn id(&self) -> &str {
        "J001"
    }
    fn name(&self) -> &str {
        "Join result explosion"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("JOIN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let output_rows = context.node.metrics.pull_row_num? as f64;
        let probe_rows = context.get_metric("ProbeRows")?;

        if probe_rows == 0.0 {
            return None;
        }

        let min_probe_rows = context.thresholds.get_min_rows_for_join();
        if probe_rows < min_probe_rows {
            return None;
        }

        let ratio = output_rows / probe_rows;

        if ratio > 10.0 {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Error,
                node_path: format!("{} (plan_node_id={})",
                    context.node.operator_name,
                    context.node.plan_node_id.unwrap_or(-1)),
                plan_node_id: context.node.plan_node_id,
                message: format!(
                    "The Join result is {:.1}x larger (output {:.0} rows / probe {:.0} rows)",
                    ratio, output_rows, probe_rows
                ),
                reason: "The Join output is significantly larger than its input, usually because a join condition is missing (making it a Cross Join) or because a wrong join condition produces 1:N matches.".to_string(),
                suggestions: vec![
                    "Check whether the JOIN condition is missing or incomplete".to_string(),
                    "Check whether a many-to-many relationship exists".to_string(),
                    "Consider adding more filter conditions".to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// J002: Build side larger than probe side
/// Condition: BuildRows > ProbeRows
pub struct J002BuildLargerThanProbe;

impl DiagnosticRule for J002BuildLargerThanProbe {
    fn id(&self) -> &str {
        "J002"
    }
    fn name(&self) -> &str {
        "Join build side too large"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("HASH")
            && node.operator_name.to_uppercase().contains("JOIN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let build_rows = context.get_metric("BuildRows")?;
        let probe_rows = context.get_metric("ProbeRows")?;

        if build_rows > probe_rows && build_rows > 100_000.0 {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Warning,
                node_path: format!("{} (plan_node_id={})", 
                    context.node.operator_name,
                    context.node.plan_node_id.unwrap_or(-1)),
                plan_node_id: context.node.plan_node_id,
                message: format!(
                    "The build side has more rows ({:.0}) than the probe side ({:.0}), the Join order may be suboptimal",
                    build_rows, probe_rows
                ),
                reason: "In a Hash Join the build side holds more data than the probe side, which makes the HashTable too large. The optimizer may have picked the wrong build side because the statistics are inaccurate.".to_string(),
                suggestions: vec![
                    "Run ANALYZE TABLE to refresh statistics".to_string(),
                    "Check whether the optimizer chose the right Join order".to_string(),
                    "Consider using a hint to force the Join order".to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// J003: HashTable memory too large
/// Condition: HashTableMemoryUsage > threshold (dynamic based on BE memory)
/// v2.0: Uses dynamic hash table memory threshold (5% of BE memory, clamped to 512MB-5GB)
pub struct J003HashTableTooLarge;

impl DiagnosticRule for J003HashTableTooLarge {
    fn id(&self) -> &str {
        "J003"
    }
    fn name(&self) -> &str {
        "HashTable memory too large"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("HASH")
            && node.operator_name.to_uppercase().contains("JOIN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let hash_memory = context
            .get_metric("HashTableMemoryUsage")
            .or_else(|| context.get_memory_usage().map(|v| v as f64))?;

        let memory_threshold = context.thresholds.get_hash_table_memory_threshold() as f64;

        if hash_memory > memory_threshold {
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
                    "HashTable memory usage {}, which may cause memory pressure (threshold: {})",
                    format_bytes(hash_memory as u64),
                    format_bytes(memory_threshold as u64)
                ),
                reason: "The HashTable of the Join takes too much memory, which may cause memory pressure or trigger a spill."
                    .to_string(),
                suggestions: vec![
                    "Check whether the build side holds too much data".to_string(),
                    "Consider using a Runtime Filter to reduce the data volume".to_string(),
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

/// J004: Runtime Filter not generated
/// Condition: RuntimeFilterNum == 0 && BuildRows > 10000
pub struct J004NoRuntimeFilter;

impl DiagnosticRule for J004NoRuntimeFilter {
    fn id(&self) -> &str {
        "J004"
    }
    fn name(&self) -> &str {
        "No Runtime Filter generated"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("JOIN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let rf_num = context.get_metric("RuntimeFilterNum").unwrap_or(0.0);
        let build_rows = context.get_metric("BuildRows").unwrap_or(0.0);

        if rf_num == 0.0 && build_rows > 10_000.0 {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Info,
                node_path: format!("{} (plan_node_id={})", 
                    context.node.operator_name,
                    context.node.plan_node_id.unwrap_or(-1)),
                plan_node_id: context.node.plan_node_id,
                message: format!(
                    "The Join generated no Runtime Filter, the build side has {:.0} rows",
                    build_rows
                ),
                reason: "The Runtime Filter was not generated or is not effective, so the Scan side cannot filter the data. The filter build may have failed or may have poor selectivity.".to_string(),
                suggestions: vec![
                    "Check whether enable_global_runtime_filter is enabled".to_string(),
                    "Check whether the Join conditions suit generating a RF".to_string(),
                    "Check whether the build side exceeds the row threshold".to_string(),
                ],
                parameter_suggestions: {
                    let mut suggestions = Vec::new();
                    if let Some(s) = context.suggest_parameter_smart("enable_global_runtime_filter") {
                        suggestions.push(s);
                    }
                    if let Some(s) = context.suggest_parameter_smart("runtime_join_filter_push_down_limit") {
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

/// J009: Non-equi join fallback
/// Condition: JoinType contains CROSS or NESTLOOP
pub struct J009NonEquiJoin;

impl DiagnosticRule for J009NonEquiJoin {
    fn id(&self) -> &str {
        "J009"
    }
    fn name(&self) -> &str {
        "Non-equi Join fallback"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        let name = node.operator_name.to_uppercase();
        name.contains("CROSS") || name.contains("NEST") || name.contains("LOOP")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let probe_rows = context.get_metric("ProbeRows").unwrap_or(0.0);
        let build_rows = context.get_metric("BuildRows").unwrap_or(0.0);

        if probe_rows > 1000.0 || build_rows > 1000.0 {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Warning,
                node_path: format!("{} (plan_node_id={})", 
                    context.node.operator_name,
                    context.node.plan_node_id.unwrap_or(-1)),
                plan_node_id: context.node.plan_node_id,
                message: format!(
                    "The {} operator is used, which is likely caused by a non-equi Join",
                    context.node.operator_name
                ),
                reason: "The Join conditions contain non-equi predicates, so a Hash Join cannot be used and the plan degrades to a Nested Loop Join, which performs poorly.".to_string(),
                suggestions: vec![
                    "Check whether the JOIN condition contains an equality predicate".to_string(),
                    "Try to turn the non-equi condition into an equality condition".to_string(),
                    "Consider restructuring the query logic".to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// J010: Probe cache unfriendly
/// Condition: ProbeRows >> BuildRows && HashTableMemory > L3 Cache
pub struct J010ProbeCacheUnfriendly;

impl DiagnosticRule for J010ProbeCacheUnfriendly {
    fn id(&self) -> &str {
        "J010"
    }
    fn name(&self) -> &str {
        "Probe cache unfriendly"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("HASH")
            && node.operator_name.to_uppercase().contains("JOIN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let hash_memory = context
            .get_metric("HashTableMemoryUsage")
            .or_else(|| context.get_memory_usage().map(|v| v as f64))?;
        let probe_rows = context.get_metric("ProbeRows")?;
        let build_rows = context.get_metric("BuildRows")?;

        const L3_CACHE: f64 = 50.0 * 1024.0 * 1024.0;

        if hash_memory > L3_CACHE && probe_rows > build_rows * 100.0 {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Info,
                node_path: format!("{} (plan_node_id={})", 
                    context.node.operator_name,
                    context.node.plan_node_id.unwrap_or(-1)),
                plan_node_id: context.node.plan_node_id,
                message: format!(
                    "The HashTable ({}) exceeds the L3 cache, the probe rows ({:.0}) are far more than the build rows ({:.0}), the cache access is likely unfriendly",
                    format_bytes(hash_memory as u64), probe_rows, build_rows
                ),
                reason: "The hash table probe has a low cache hit rate, probably because the HashTable is too large for the CPU cache or because the probe access pattern is unfriendly.".to_string(),
                suggestions: vec![
                    "Consider swapping the left and right tables of the Join".to_string(),
                    "Use a hint to force the Join order".to_string(),
                    "Check whether the statistics are accurate".to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// J005: Hash collision severe
pub struct J005HashCollision;

impl DiagnosticRule for J005HashCollision {
    fn id(&self) -> &str {
        "J005"
    }
    fn name(&self) -> &str {
        "Severe hash collisions"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("JOIN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let keys_per_bucket = context.get_metric("BuildKeysPerBucket%")?;
        if keys_per_bucket > 10.0 {
            let join_pred = context
                .get_join_predicates()
                .unwrap_or_else(|| "Unknown".to_string());
            let build_rows = context.get_metric("BuildRows").unwrap_or(0.0);

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
                    "The hash table has severe collisions, {:.0} keys per bucket on average",
                    keys_per_bucket
                ),
                reason: format!(
                    "The hash table of the Join condition 「{}」 has a lot of conflicts (the build side holds {:.0} rows), which slows the probe down. This is probably caused by an uneven distribution of the Join keys or by a poor hash function.",
                    join_pred, build_rows
                ),
                suggestions: vec![
                    format!(
                        "Check whether the Join key 「{}」 has many duplicate or NULL values",
                        join_pred
                    ),
                    "Consider adding more equi-join conditions to spread the hash distribution"
                        .to_string(),
                    "Check the data distribution of the build table and add a pre-filter if needed"
                        .to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// J006: Join shuffle skew
/// v2.0: Uses dynamic skew threshold based on cluster parallelism
pub struct J006ShuffleSkew;

impl DiagnosticRule for J006ShuffleSkew {
    fn id(&self) -> &str {
        "J006"
    }
    fn name(&self) -> &str {
        "Join shuffle skew"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("JOIN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let max_probe = context.get_metric("__MAX_OF_ProbeRows")?;
        let min_probe = context.get_metric("__MIN_OF_ProbeRows").unwrap_or(0.0);
        if min_probe == 0.0 {
            return None;
        }
        let ratio = max_probe / ((max_probe + min_probe) / 2.0);

        let skew_threshold = context.thresholds.get_skew_threshold() + 1.0;

        if ratio > skew_threshold {
            let join_pred = context
                .get_join_predicates()
                .unwrap_or_else(|| "Unknown".to_string());

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
                    "The Join data distribution is skewed, max/avg ratio is {:.2} (threshold: {:.1})",
                    ratio, skew_threshold
                ),
                reason: format!(
                    "The Shuffle Join distributes the data unevenly on 「{}」, the largest instance handles {:.0} rows while the smallest handles only {:.0} rows. This is usually caused by hot values in the Join key.",
                    join_pred, max_probe, min_probe
                ),
                suggestions: vec![
                    format!(
                        "Check whether the Join key 「{}」 holds hot values (e.g. NULL or very frequent values)",
                        join_pred
                    ),
                    "Consider adding more Join conditions to spread the data".to_string(),
                    "Handle the hot keys separately or add a salt key to spread them".to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// J007: Partition join probe overhead high
pub struct J007PartitionProbeOverhead;

impl DiagnosticRule for J007PartitionProbeOverhead {
    fn id(&self) -> &str {
        "J007"
    }
    fn name(&self) -> &str {
        "High partition join probe overhead"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("JOIN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let partition_nums = context.get_metric("PartitionNums")?;
        let probe_overhead = context.get_metric("PartitionProbeOverhead").unwrap_or(0.0);
        let search_time = context.get_metric("SearchHashTableTime").unwrap_or(1.0);
        if partition_nums > 1.0 && search_time > 0.0 && probe_overhead / search_time > 0.5 {
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
                    "The partition probe overhead is {:.1}% of the probe time, with {:.0} partitions",
                    probe_overhead / search_time * 100.0,
                    partition_nums
                ),
                reason: "The partition probe overhead is too high, probably because there are too many partitions or the partitioning strategy is poor.".to_string(),
                suggestions: vec![
                    "Check whether the partition count is reasonable".to_string(),
                    "Consider raising the memory limit to avoid over-partitioning".to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// J008: Runtime filter memory high
pub struct J008RFMemoryHigh;

impl DiagnosticRule for J008RFMemoryHigh {
    fn id(&self) -> &str {
        "J008"
    }
    fn name(&self) -> &str {
        "Runtime Filter memory usage high"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("JOIN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let rf_bytes = context.get_metric("PartialRuntimeMembershipFilterBytes")?;
        const HUNDRED_MB: f64 = 100.0 * 1024.0 * 1024.0;
        if rf_bytes > HUNDRED_MB {
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
                message: format!("Runtime Filter memory usage {}", format_bytes(rf_bytes as u64)),
                reason: "The Runtime Filter takes too much memory, probably because there are too many filters or a single filter is too large."
                    .to_string(),
                suggestions: vec![
                    "Lower the runtime_filter_max_size setting".to_string(),
                    "Check whether the Join key cardinality is too high".to_string(),
                ],
                parameter_suggestions: vec![ParameterSuggestion::new(
                    "runtime_filter_max_size",
                    ParameterType::Session,
                    None,
                    "67108864",
                    "SET runtime_filter_max_size = 67108864; -- 64MB",
                )],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// J011: Broadcast Join not recommended for large build table
/// Condition: DistributionMode = BROADCAST and BuildRows > 1M or HashTableMemoryUsage > 100MB
pub struct J011BroadcastNotRecommended;

impl DiagnosticRule for J011BroadcastNotRecommended {
    fn id(&self) -> &str {
        "J011"
    }
    fn name(&self) -> &str {
        "Join should not use Broadcast"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("JOIN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let dist_mode = context.node.unique_metrics.get("DistributionMode")?;
        if !dist_mode.to_uppercase().contains("BROADCAST") {
            return None;
        }

        let build_rows = context.get_metric("BuildRows").unwrap_or(0.0);
        let hash_table_memory = context.get_metric("HashTableMemoryUsage").unwrap_or(0.0);

        let rows_threshold = 1_000_000.0;
        let memory_threshold = 100.0 * 1024.0 * 1024.0;

        if build_rows > rows_threshold || hash_table_memory > memory_threshold {
            let join_pred = context
                .get_join_predicates()
                .unwrap_or_else(|| "Unknown".to_string());
            let probe_rows = context.get_metric("ProbeRows").unwrap_or(0.0);

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
                    "The build side of the Broadcast Join is too large ({:.0} rows, {})",
                    build_rows,
                    format_bytes(hash_table_memory as u64)
                ),
                reason: format!(
                    "When a small table is joined with a large table, a Broadcast Join broadcasts the small table to every node.\
                    But when the build side is too large, the network and compute cost grows.\
                    The build side of the Join condition 「{}」 has {:.0} rows and the probe side has {:.0} rows,\
                    which is likely caused by inaccurate statistics that made the optimizer misestimate the table size.",
                    join_pred, build_rows, probe_rows
                ),
                suggestions: vec![
                    format!(
                        "Add the [shuffle] hint after the JOIN keyword: SELECT ... FROM a JOIN [shuffle] b ON {}",
                        join_pred
                    ),
                    "Run ANALYZE TABLE <build_table> to refresh statistics".to_string(),
                    "Check the Join order and make sure the small table is on the build side"
                        .to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// Get all join rules
pub fn get_rules() -> Vec<Box<dyn DiagnosticRule>> {
    vec![
        Box::new(J001ResultExplosion),
        Box::new(J002BuildLargerThanProbe),
        Box::new(J003HashTableTooLarge),
        Box::new(J004NoRuntimeFilter),
        Box::new(J005HashCollision),
        Box::new(J006ShuffleSkew),
        Box::new(J007PartitionProbeOverhead),
        Box::new(J008RFMemoryHigh),
        Box::new(J009NonEquiJoin),
        Box::new(J010ProbeCacheUnfriendly),
        Box::new(J011BroadcastNotRecommended),
    ]
}
