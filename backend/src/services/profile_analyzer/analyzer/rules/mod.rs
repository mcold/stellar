//! Diagnostic rules module
//!
//! Implements the rule engine for Query Profile diagnostics.
//! Rules are organized by operator type following the design document.

pub mod aggregate;
pub mod common;
pub mod exchange;
pub mod fragment;
pub mod join;
pub mod planner;
pub mod project;
pub mod query;
pub mod scan;
pub mod sink;
pub mod sort;

use super::thresholds::DynamicThresholds;
use crate::services::profile_analyzer::models::*;
use once_cell::sync::Lazy;
use regex::Regex;

/// Regex to clean slot IDs from column names (e.g., "46: dayno" -> "dayno")
static SLOT_ID_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\d+:\s*").unwrap());

/// Clean slot IDs from StarRocks profile column references
/// e.g., "46: dayno, 47: case, 18: open_traceid" -> "dayno, case, open_traceid"
pub fn clean_slot_ids(s: &str) -> String {
    SLOT_ID_REGEX.replace_all(s, "").to_string()
}

// ============================================================================
// Rule Trait and Types
// ============================================================================

/// Severity level for diagnostic rules
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuleSeverity {
    Info = 0,
    Warning = 1,
    Error = 2,
}

impl From<RuleSeverity> for HotSeverity {
    fn from(severity: RuleSeverity) -> Self {
        match severity {
            RuleSeverity::Info => HotSeverity::Mild,
            RuleSeverity::Warning => HotSeverity::Moderate,
            RuleSeverity::Error => HotSeverity::Severe,
        }
    }
}

/// Parameter suggestion for tuning
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ParameterSuggestion {
    /// Parameter name (e.g., "enable_scan_datacache")
    pub name: String,
    /// Parameter type (Session or BE)
    pub param_type: ParameterType,
    /// Current value if set, None if using default
    pub current: Option<String>,
    /// Recommended value
    pub recommended: String,
    /// SQL command to set the parameter
    pub command: String,
    /// Human-readable description of what this parameter does
    pub description: String,
    /// Expected impact of changing this parameter
    pub impact: String,
}

/// Parameter type classification
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub enum ParameterType {
    Session,
    BE,
}

/// Get parameter metadata (description and impact) for common StarRocks parameters
pub fn get_parameter_metadata(name: &str) -> ParameterMetadata {
    match name {
        "enable_scan_datacache" => ParameterMetadata {
            description: "Enables the DataCache read cache, so the data can be read from the local cache instead of the remote storage".to_string(),
            impact: "Improves the query performance in a disaggregated storage architecture and reduces the network IO".to_string(),
        },
        "enable_populate_datacache" => ParameterMetadata {
            description: "Enables the DataCache write population, caching the remotely read data locally".to_string(),
            impact: "The later queries hit the local cache, but the first query pays an extra write cost".to_string(),
        },
        "datacache_evict_probability" => ParameterMetadata {
            description: "DataCache eviction probability (0-100), controls how likely the cached data is evicted".to_string(),
            impact: "Lowering it reduces the cache churn, but may leave too little cache space".to_string(),
        },

        "enable_query_cache" => ParameterMetadata {
            description: "Enables the query result cache, so a repeated query can return the cached result directly".to_string(),
            impact: "Significantly speeds up repeated queries, but takes extra memory".to_string(),
        },
        "enable_adaptive_sink_dop" => ParameterMetadata {
            description: "Enables the adaptive sink parallelism, adjusting the write parallelism to the data volume".to_string(),
            impact: "Improves the write performance and reduces the number of small files".to_string(),
        },
        "enable_runtime_adaptive_dop" => ParameterMetadata {
            description: "Enables the runtime adaptive parallelism, adjusting the execution parallelism to the actual data volume".to_string(),
            impact: "Improves the resource utilization and keeps small queries from taking too many resources".to_string(),
        },
        "enable_spill" => ParameterMetadata {
            description: "Enables spilling the intermediate results to disk when the memory is not enough".to_string(),
            impact: "Handles queries over very large data volumes, but slows the query down".to_string(),
        },

        "enable_connector_adaptive_io_tasks" => ParameterMetadata {
            description: "Enables the adaptive IO task count of the connector, adjusting the IO parallelism to the data volume".to_string(),
            impact: "Improves the external table scan performance and balances the IO and CPU resources".to_string(),
        },
        "io_tasks_per_scan_operator" => ParameterMetadata {
            description: "Number of IO tasks per scan operator, controls the parallelism of the local table scan".to_string(),
            impact: "Raising it improves the scan throughput, but increases the IO pressure".to_string(),
        },
        "connector_io_tasks_per_scan_operator" => ParameterMetadata {
            description: "Number of IO tasks per connector scan operator, controls the parallelism of the external table scan".to_string(),
            impact: "Raising it improves the external table scan throughput, but increases the pressure on the remote storage".to_string(),
        },

        "hash_join_push_down_right_table" => ParameterMetadata {
            description: "Enables the right table pushdown of a Hash Join, broadcasting the small table to every node".to_string(),
            impact: "Reduces the Shuffled data and improves the Join performance".to_string(),
        },
        "enable_local_shuffle_agg" => ParameterMetadata {
            description: "Enables the local Shuffle aggregation, pre-aggregating the data locally first".to_string(),
            impact: "Reduces the amount of data transferred over the network and improves the aggregation performance".to_string(),
        },

        "runtime_filter_on_exchange_node" => ParameterMetadata {
            description: "Enables the Runtime Filter on the Exchange node, passing the filter conditions across the nodes".to_string(),
            impact: "Filters the data earlier to reduce the Shuffle, but adds the filter build cost".to_string(),
        },
        "global_runtime_filter_build_max_size" => ParameterMetadata {
            description: "Maximum build size of the global Runtime Filter (in bytes)".to_string(),
            impact: "Raising it supports bigger filters, but takes more memory".to_string(),
        },

        "parallel_fragment_exec_instance_num" => ParameterMetadata {
            description: "Number of parallel execution instances per Fragment".to_string(),
            impact: "Raising it improves the parallelism, but takes more resources".to_string(),
        },
        "pipeline_dop" => ParameterMetadata {
            description: "Pipeline execution parallelism, 0 means automatic".to_string(),
            impact: "A manual value controls the resource usage, the automatic mode adjusts it to the number of CPU cores".to_string(),
        },

        "query_mem_limit" => ParameterMetadata {
            description: "Memory limit of a single query (in bytes)".to_string(),
            impact: "Raising it handles larger data volumes, but may affect the other queries".to_string(),
        },
        "query_timeout" => ParameterMetadata {
            description: "Query timeout (in seconds)".to_string(),
            impact: "Raising it allows long running queries, but may hold the resources for too long".to_string(),
        },

        "streaming_preaggregation_mode" => ParameterMetadata {
            description: "Streaming pre-aggregation mode (auto/force_streaming/force_preaggregation)".to_string(),
            impact: "The auto mode picks the best strategy, the force modes enforce the given strategy".to_string(),
        },
        "enable_sort_aggregate" => ParameterMetadata {
            description: "Enables the sort aggregation, which fits a high cardinality GROUP BY".to_string(),
            impact: "Reduces the memory usage, but adds the sorting cost".to_string(),
        },

        "pipeline_profile_level" => ParameterMetadata {
            description: "Pipeline Profile detail level (0-2)".to_string(),
            impact: "A higher level gives more detail, but also costs more to collect".to_string(),
        },

        "storage_page_cache_limit" => ParameterMetadata {
            description: "Size limit of the BE storage page cache".to_string(),
            impact: "Raising it improves the read performance of hot data, but takes more memory".to_string(),
        },

        _ => ParameterMetadata {
            description: format!("StarRocks parameter {}", name),
            impact: "See the StarRocks official documentation for the details".to_string(),
        },
    }
}

/// Metadata for a parameter
#[derive(Debug, Clone)]
pub struct ParameterMetadata {
    pub description: String,
    pub impact: String,
}

impl ParameterSuggestion {
    /// Create a new parameter suggestion with automatic metadata lookup
    pub fn new(
        name: &str,
        param_type: ParameterType,
        current: Option<String>,
        recommended: &str,
        command: &str,
    ) -> Self {
        let metadata = get_parameter_metadata(name);
        Self {
            name: name.to_string(),
            param_type,
            current,
            recommended: recommended.to_string(),
            command: command.to_string(),
            description: metadata.description,
            impact: metadata.impact,
        }
    }
}

/// Threshold metadata for diagnostic traceability
/// Captures what threshold was used and its source (baseline vs default)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ThresholdMetadata {
    /// Threshold value used for this diagnostic (e.g., 10000.0 for 10s)
    pub threshold_value: f64,
    /// Threshold source: "baseline" | "default" | "config"
    pub threshold_source: String,
    /// Baseline P95 value if baseline was used (ms)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_p95_ms: Option<f64>,
    /// Baseline sample count if baseline was used
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_sample_count: Option<usize>,
    /// Cluster ID where baseline was fetched from
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster_id: Option<i64>,
}

impl ThresholdMetadata {
    /// Create metadata for default threshold
    pub fn from_default(threshold_value: f64) -> Self {
        Self {
            threshold_value,
            threshold_source: "default".to_string(),
            baseline_p95_ms: None,
            baseline_sample_count: None,
            cluster_id: None,
        }
    }

    /// Create metadata from baseline
    pub fn from_baseline(
        threshold_value: f64,
        baseline: &super::baseline::PerformanceBaseline,
    ) -> Self {
        Self {
            threshold_value,
            threshold_source: "baseline".to_string(),
            baseline_p95_ms: Some(baseline.stats.p95_ms),
            baseline_sample_count: Some(baseline.sample_size),
            cluster_id: None,
        }
    }
}

/// A diagnostic result from rule evaluation
///
/// Structure follows Aliyun EMR StarRocks diagnostic standard:
/// - message: 诊断结果概要说明 (Summary of the issue)
/// - reason: 详细诊断原因说明 (Detailed explanation of why this happens)
/// - suggestions: 建议措施 (Recommended actions)
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub rule_id: String,
    pub rule_name: String,
    pub severity: RuleSeverity,
    pub node_path: String,
    /// Plan node ID for associating diagnostic with execution tree node
    pub plan_node_id: Option<i32>,
    /// Summary of the diagnostic issue (诊断结果概要)
    pub message: String,
    /// Detailed explanation of why this issue occurs (详细诊断原因)
    pub reason: String,
    /// Recommended actions to fix the issue (建议措施)
    pub suggestions: Vec<String>,
    pub parameter_suggestions: Vec<ParameterSuggestion>,
    /// Threshold metadata for traceability (what threshold triggered this diagnostic)
    pub threshold_metadata: Option<ThresholdMetadata>,
}

impl Diagnostic {
    /// Create a new diagnostic with threshold metadata
    pub fn with_threshold(mut self, metadata: ThresholdMetadata) -> Self {
        self.threshold_metadata = Some(metadata);
        self
    }
}

impl Diagnostic {
    /// Convert to HotSpot for backward compatibility
    pub fn to_hotspot(&self) -> HotSpot {
        let mut all_suggestions = self.suggestions.clone();

        for param in &self.parameter_suggestions {
            all_suggestions.push(format!(
                "Adjust the parameter: {} → {} (command: {})",
                param.name, param.recommended, param.command
            ));
        }

        HotSpot {
            node_path: self.node_path.clone(),
            severity: self.severity.into(),
            issue_type: self.rule_id.clone(),
            description: self.message.clone(),
            suggestions: all_suggestions,
        }
    }
}

/// Context for rule evaluation
pub struct RuleContext<'a> {
    pub node: &'a ExecutionTreeNode,
    /// Non-default session variables from profile summary
    pub session_variables: &'a std::collections::HashMap<String, SessionVariableInfo>,
    /// Cluster information for smart recommendations
    pub cluster_info: Option<ClusterInfo>,
    /// Live cluster variables (actual current values from cluster)
    /// Takes precedence over session_variables for parameter recommendations
    pub cluster_variables: Option<&'a std::collections::HashMap<String, String>>,
    /// Default database from profile summary
    pub default_db: Option<&'a str>,
    /// Dynamic thresholds based on cluster info and query type
    pub thresholds: DynamicThresholds,
}

impl<'a> RuleContext<'a> {
    /// Get a metric value from unique_metrics as f64
    pub fn get_metric(&self, name: &str) -> Option<f64> {
        self.node
            .unique_metrics
            .get(name)
            .and_then(|v| parse_metric_value(v))
    }

    /// Get a metric value that represents bytes (e.g., "13.227 TB")
    pub fn get_metric_bytes(&self, name: &str) -> Option<f64> {
        self.node
            .unique_metrics
            .get(name)
            .and_then(|v| parse_bytes_value(v))
    }

    /// Get a metric value that represents duration (e.g., "2m4s", "1.5s", "100ms")
    pub fn get_metric_duration(&self, name: &str) -> Option<f64> {
        self.node
            .unique_metrics
            .get(name)
            .and_then(|v| parse_duration_to_ms(v))
    }

    /// Get operator total time in ms
    pub fn get_operator_time_ms(&self) -> Option<f64> {
        self.node
            .metrics
            .operator_total_time
            .map(|ns| ns as f64 / 1_000_000.0)
    }

    /// Get time percentage
    pub fn get_time_percentage(&self) -> Option<f64> {
        self.node.time_percentage
    }

    /// Get memory usage in bytes
    pub fn get_memory_usage(&self) -> Option<u64> {
        self.node.metrics.memory_usage
    }

    /// Get table name from unique_metrics (e.g., "Table" metric)
    pub fn get_table_name(&self) -> String {
        self.node
            .unique_metrics
            .get("Table")
            .map(|s| s.as_str())
            .unwrap_or("unknown")
            .to_string()
    }

    /// Get full table name with database prefix
    pub fn get_full_table_name(&self) -> String {
        let table = self.get_table_name();
        if table.contains('.') {
            return table;
        }
        match self.default_db {
            Some(db) if !db.is_empty() => format!("{}.{}", db, table),
            _ => table,
        }
    }

    /// Get Join EQ predicate info if available
    pub fn get_join_predicates(&self) -> Option<String> {
        self.node
            .unique_metrics
            .get("EQJoinConjuncts")
            .or(self.node.unique_metrics.get("JoinPredicates"))
            .or(self.node.unique_metrics.get("JoinConjuncts"))
            .cloned()
    }

    /// Get GROUP BY key info if available (with slot IDs cleaned)
    pub fn get_group_by_keys(&self) -> Option<String> {
        self.node
            .unique_metrics
            .get("GroupingKeys")
            .or(self.node.unique_metrics.get("GroupByKeys"))
            .map(|s| clean_slot_ids(s))
    }

    /// Check if this is an internal table (OLAP_SCAN)
    pub fn is_internal_table(&self) -> bool {
        let op = self.node.operator_name.to_uppercase();
        op.contains("OLAP_SCAN") || op.contains("OLAP_TABLE")
    }

    /// Check if this is an external table (covers all external data sources)
    /// Includes: HDFS, Hive, Iceberg, Hudi, Delta, Paimon, JDBC, MySQL, ES, File, etc.
    pub fn is_external_table(&self) -> bool {
        let op = self.node.operator_name.to_uppercase();

        if op.contains("CONNECTOR")
            || op.contains("HDFS")
            || op.contains("HIVE")
            || op.contains("ICEBERG")
            || op.contains("HUDI")
            || op.contains("DELTA")
            || op.contains("PAIMON")
            || op.contains("FILE_SCAN")
        {
            return true;
        }

        if op.contains("JDBC")
            || op.contains("MYSQL")
            || op.contains("ES_SCAN")
            || op.contains("ELASTICSEARCH")
        {
            return true;
        }

        if let Some(ds) = self.node.unique_metrics.get("DataSourceType") {
            let ds_up = ds.to_uppercase();
            return ds_up.contains("HIVE")
                || ds_up.contains("ICEBERG")
                || ds_up.contains("HUDI")
                || ds_up.contains("DELTA")
                || ds_up.contains("JDBC")
                || ds_up.contains("MYSQL")
                || ds_up.contains("ES")
                || ds_up.contains("PAIMON");
        }
        false
    }

    /// Get detailed external table type for more specific suggestions
    pub fn get_external_table_type(&self) -> Option<&'static str> {
        let op = self.node.operator_name.to_uppercase();
        if op.contains("HIVE") {
            return Some("Hive");
        }
        if op.contains("ICEBERG") {
            return Some("Iceberg");
        }
        if op.contains("HUDI") {
            return Some("Hudi");
        }
        if op.contains("DELTA") {
            return Some("Delta Lake");
        }
        if op.contains("PAIMON") {
            return Some("Paimon");
        }
        if op.contains("HDFS") || op.contains("FILE_SCAN") {
            return Some("HDFS/File");
        }
        if op.contains("JDBC") {
            return Some("JDBC");
        }
        if op.contains("MYSQL") {
            return Some("MySQL");
        }
        if op.contains("ES") || op.contains("ELASTICSEARCH") {
            return Some("Elasticsearch");
        }
        if op.contains("CONNECTOR") {
            return Some("Connector");
        }
        None
    }

    /// Check if a session variable is already set to the expected value
    /// Returns true if the variable is set and matches the expected value
    pub fn is_variable_set_to(&self, name: &str, expected: &str) -> bool {
        self.session_variables
            .get(name)
            .map(|info| info.actual_value_is(expected))
            .unwrap_or(false)
    }

    /// Get current value of a session variable as string
    /// Priority: cluster_variables > session_variables (non-default) > None
    pub fn get_variable_value(&self, name: &str) -> Option<String> {
        if let Some(cluster_vars) = self.cluster_variables
            && let Some(value) = cluster_vars.get(name)
        {
            return Some(value.clone());
        }

        self.session_variables
            .get(name)
            .map(|info| info.actual_value_str())
    }

    /// Create a parameter suggestion only if the parameter is not already set to the recommended value
    /// Returns None if the parameter is already set to the recommended value (no suggestion needed)
    ///
    /// Note: For parameters not in NonDefaultSessionVariables, we check against known defaults.
    /// If a parameter uses its default value and that default matches the recommendation, no suggestion is made.
    pub fn suggest_parameter(
        &self,
        name: &str,
        recommended: &str,
        command: &str,
    ) -> Option<ParameterSuggestion> {
        if self.is_variable_set_to(name, recommended) {
            return None;
        }

        if !self.session_variables.contains_key(name)
            && let Some(default) = get_parameter_default(name)
            && default.eq_ignore_ascii_case(recommended)
        {
            return None;
        }

        let current = self.get_variable_value(name);

        let metadata = get_parameter_metadata(name);

        Some(ParameterSuggestion {
            name: name.to_string(),
            param_type: ParameterType::Session,
            current,
            recommended: recommended.to_string(),
            command: command.to_string(),
            description: metadata.description,
            impact: metadata.impact,
        })
    }

    /// Smart parameter suggestion that considers cluster info and current values
    /// Returns None if:
    /// - Current value already meets or exceeds recommendation
    /// - Query is too small to benefit from the change
    /// - Parameter is already set to recommended value
    pub fn suggest_parameter_smart(&self, name: &str) -> Option<ParameterSuggestion> {
        let cluster_info = self.cluster_info.as_ref();

        let current_str = self.get_variable_value(name);
        let default_str = get_parameter_default(name).map(|s| s.to_string());
        let effective_value = current_str.as_ref().or(default_str.as_ref());

        let current_i64 = effective_value.and_then(|v| v.parse::<i64>().ok());
        let current_bool = effective_value.map(|v| v.eq_ignore_ascii_case("true"));

        let (recommended, reason, param_type) = match name {
            "parallel_fragment_exec_instance_num" => {
                let be_count = cluster_info.map(|c| c.backend_num).unwrap_or(1).max(1);
                let recommended = be_count.min(16) as i64;
                let current = current_i64.unwrap_or(1);

                if current >= recommended {
                    return None;
                }

                if cluster_info
                    .map(|c| c.total_scan_bytes < 100_000_000)
                    .unwrap_or(true)
                {
                    return None;
                }

                (
                    recommended.to_string(),
                    format!("Recommended for a cluster with {} BE nodes", be_count),
                    ParameterType::Session,
                )
            },

            "pipeline_dop" => {
                let current = current_i64.unwrap_or(0);
                if current == 0 {
                    return None;
                }
                (
                    "0".to_string(),
                    "The automatic mode is recommended, the system adjusts it to the number of CPU cores".to_string(),
                    ParameterType::Session,
                )
            },

            "io_tasks_per_scan_operator" => {
                let current = current_i64.unwrap_or(4);
                let total_bytes = cluster_info.map(|c| c.total_scan_bytes).unwrap_or(0);
                let is_large_scan = total_bytes > 1_000_000_000;
                let recommended = if is_large_scan { 8 } else { 4 };

                if current >= recommended || !is_large_scan {
                    return None;
                }

                (
                    recommended.to_string(),
                    "Large data volume, increasing the IO parallelism is recommended".to_string(),
                    ParameterType::Session,
                )
            },

            "query_mem_limit" => {
                let current = current_i64.unwrap_or(0);
                let total_bytes = cluster_info.map(|c| c.total_scan_bytes).unwrap_or(0);

                let recommended = if total_bytes > 0 {
                    (total_bytes * 2).clamp(4 * 1024 * 1024 * 1024, 32 * 1024 * 1024 * 1024) as i64
                } else {
                    8 * 1024 * 1024 * 1024
                };

                if current >= recommended {
                    return None;
                }

                let recommended_gb = recommended / (1024 * 1024 * 1024);
                (
                    recommended.to_string(),
                    format!(
                        "A {}GB memory limit is recommended for this data volume",
                        recommended_gb
                    ),
                    ParameterType::Session,
                )
            },

            "enable_spill" => {
                let current = current_bool.unwrap_or(false);
                if current {
                    return None;
                }
                (
                    "true".to_string(),
                    "Enabling it avoids an OOM on large queries".to_string(),
                    ParameterType::Session,
                )
            },

            "query_timeout" => {
                let current = current_i64.unwrap_or(300);

                if current >= 600 {
                    return None;
                }
                (
                    "600".to_string(),
                    "Raising the timeout supports the complex queries".to_string(),
                    ParameterType::Session,
                )
            },

            "enable_query_cache" => {
                let current = current_bool.unwrap_or(false);
                if current {
                    return None;
                }
                (
                    "true".to_string(),
                    "The query cache speeds up the repeated queries".to_string(),
                    ParameterType::Session,
                )
            },

            "enable_global_runtime_filter" => {
                let current = current_bool.unwrap_or(true);
                if current {
                    return None;
                }
                (
                    "true".to_string(),
                    "The global Runtime Filter improves the Join performance".to_string(),
                    ParameterType::Session,
                )
            },

            "runtime_join_filter_push_down_limit" => {
                let current = current_i64.unwrap_or(1024000);
                if current >= 10_000_000 {
                    return None;
                }
                (
                    "10000000".to_string(),
                    "Raising the RF pushdown threshold supports a bigger build side".to_string(),
                    ParameterType::Session,
                )
            },

            "enable_scan_datacache" => {
                let current = current_bool.unwrap_or(true);
                if current {
                    return None;
                }
                (
                    "true".to_string(),
                    "The DataCache improves the performance in a disaggregated storage architecture".to_string(),
                    ParameterType::Session,
                )
            },

            "enable_populate_datacache" => {
                let current = current_bool.unwrap_or(true);
                if current {
                    return None;
                }
                (
                    "true".to_string(),
                    "Enabling the cache population warms the cache up".to_string(),
                    ParameterType::Session,
                )
            },

            "pipeline_profile_level" => {
                let current = current_i64.unwrap_or(1);
                if current <= 1 {
                    return None;
                }
                (
                    "1".to_string(),
                    "Lowering the Profile level reduces the collection cost".to_string(),
                    ParameterType::Session,
                )
            },

            "storage_page_cache_limit" => (
                "30%".to_string(),
                "Raising the page cache improves the read performance of hot data".to_string(),
                ParameterType::BE,
            ),

            _ => return None,
        };

        let current = self
            .get_variable_value(name)
            .or_else(|| get_parameter_default(name).map(|s| s.to_string()));

        let metadata = get_parameter_metadata(name);
        let command = match param_type {
            ParameterType::Session => format!("SET {} = {};", name, recommended),
            ParameterType::BE => format!("-- BE config: {} = {}", name, recommended),
        };

        Some(ParameterSuggestion {
            name: name.to_string(),
            param_type,
            current,
            recommended,
            command,
            description: metadata.description,
            impact: format!("{} ({})", metadata.impact, reason),
        })
    }
}

/// Known default values for common StarRocks session parameters
/// This helps avoid suggesting parameters that are already at their recommended default values
fn get_parameter_default(name: &str) -> Option<&'static str> {
    match name {
        "enable_scan_datacache" => Some("true"),
        "enable_populate_datacache" => Some("true"),
        "datacache_evict_probability" => Some("100"),

        "enable_query_cache" => Some("false"),
        "enable_adaptive_sink_dop" => Some("false"),
        "enable_runtime_adaptive_dop" => Some("false"),
        "enable_spill" => Some("false"),

        "enable_connector_adaptive_io_tasks" => Some("true"),
        "io_tasks_per_scan_operator" => Some("4"),
        "connector_io_tasks_per_scan_operator" => Some("16"),

        "hash_join_push_down_right_table" => Some("true"),
        "enable_local_shuffle_agg" => Some("true"),

        "runtime_filter_on_exchange_node" => Some("false"),
        "global_runtime_filter_build_max_size" => Some("67108864"),

        "parallel_fragment_exec_instance_num" => Some("1"),
        "pipeline_dop" => Some("0"),

        _ => None,
    }
}

/// Trait for diagnostic rules
pub trait DiagnosticRule: Send + Sync {
    /// Rule ID (e.g., "S001", "J001")
    fn id(&self) -> &str;

    /// Rule name
    fn name(&self) -> &str;

    /// Check if rule applies to this node
    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool;

    /// Evaluate the rule and return diagnostic if triggered
    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic>;
}

// ============================================================================
// Utility Functions
// ============================================================================

/// Parse metric value from string (handles various formats)
pub fn parse_metric_value(value: &str) -> Option<f64> {
    let s = value.trim();

    if let Some(start) = s.find('(')
        && let Some(end) = s.find(')')
    {
        let inner = &s[start + 1..end].trim();
        if let Ok(v) = inner.parse::<f64>() {
            return Some(v);
        }
    }

    if s.ends_with('%') {
        return s.trim_end_matches('%').parse().ok();
    }

    if let Some(bytes) = parse_bytes(s) {
        return Some(bytes as f64);
    }

    if let Some(ms) = parse_duration_ms(s) {
        return Some(ms);
    }

    if let Some(multiplier) = get_suffix_multiplier(s) {
        let numeric_part: String = s
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
            .collect();
        if let Ok(v) = numeric_part.parse::<f64>() {
            return Some(v * multiplier);
        }
    }

    let numeric_part: String = s
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();

    numeric_part.parse().ok()
}

/// Get multiplier for K/M/B suffixes in metric values
fn get_suffix_multiplier(s: &str) -> Option<f64> {
    let s = s.trim().to_uppercase();
    if s.ends_with('K') {
        return Some(1000.0);
    }
    if s.ends_with('M') {
        return Some(1_000_000.0);
    }
    if s.ends_with('B') || s.ends_with('G') {
        return Some(1_000_000_000.0);
    }
    if s.ends_with('T') {
        return Some(1_000_000_000_000.0);
    }
    None
}

/// Parse bytes string to u64
pub fn parse_bytes(s: &str) -> Option<u64> {
    let s = s.trim();
    let parts: Vec<&str> = s.split_whitespace().collect();

    if parts.len() != 2 {
        return None;
    }

    let value: f64 = parts[0].parse().ok()?;
    let unit = parts[1].to_uppercase();

    let multiplier = match unit.as_str() {
        "B" => 1u64,
        "KB" | "K" => 1024,
        "MB" | "M" => 1024 * 1024,
        "GB" | "G" => 1024 * 1024 * 1024,
        "TB" | "T" => 1024 * 1024 * 1024 * 1024,
        _ => return None,
    };

    Some((value * multiplier as f64) as u64)
}

/// Parse duration string to milliseconds
pub fn parse_duration_ms(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let mut total_ms = 0.0;
    let mut num_buf = String::new();
    let mut found_unit = false;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() || c == '.' {
            num_buf.push(c);
            i += 1;
        } else {
            let value: f64 = num_buf.parse().unwrap_or(0.0);
            num_buf.clear();

            if c == 'h' {
                total_ms += value * 3600.0 * 1000.0;
                found_unit = true;
                i += 1;
            } else if c == 'm' {
                if i + 1 < chars.len() && chars[i + 1] == 's' {
                    total_ms += value;
                    i += 2;
                } else {
                    total_ms += value * 60.0 * 1000.0;
                    i += 1;
                }
                found_unit = true;
            } else if c == 's' {
                total_ms += value * 1000.0;
                found_unit = true;
                i += 1;
            } else if c == 'u' && i + 1 < chars.len() && chars[i + 1] == 's' {
                total_ms += value / 1000.0;
                found_unit = true;
                i += 2;
            } else if c == 'n' && i + 1 < chars.len() && chars[i + 1] == 's' {
                total_ms += value / 1_000_000.0;
                found_unit = true;
                i += 2;
            } else {
                i += 1;
            }
        }
    }

    if found_unit { Some(total_ms) } else { None }
}

/// Format bytes to human-readable string
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    format!("{:.2} {}", size, UNITS[unit_index])
}

/// Format duration in ms to human-readable string
pub fn format_duration_ms(ms: f64) -> String {
    if ms < 1.0 {
        format!("{:.2}μs", ms * 1000.0)
    } else if ms < 1000.0 {
        format!("{:.2}ms", ms)
    } else if ms < 60000.0 {
        format!("{:.2}s", ms / 1000.0)
    } else if ms < 3600000.0 {
        format!("{:.1}m", ms / 60000.0)
    } else {
        format!("{:.1}h", ms / 3600000.0)
    }
}

/// Parse bytes value string (e.g., "13.227 TB", "347.476 GB") to f64 bytes
pub fn parse_bytes_value(s: &str) -> Option<f64> {
    let s = s.trim();
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }

    let value: f64 = parts[0].parse().ok()?;
    let unit = parts[1].to_uppercase();

    let multiplier: f64 = match unit.as_str() {
        "B" => 1.0,
        "KB" | "K" => 1024.0,
        "MB" | "M" => 1024.0 * 1024.0,
        "GB" | "G" => 1024.0 * 1024.0 * 1024.0,
        "TB" | "T" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        "PB" | "P" => 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };

    Some(value * multiplier)
}

/// Parse duration string to milliseconds (handles "2m4s", "1.5s", "100ms", etc.)
pub fn parse_duration_to_ms(s: &str) -> Option<f64> {
    parse_duration_ms(s)
}

// ============================================================================
// Rule Registry
// ============================================================================

/// Get all registered rules
pub fn get_all_rules() -> Vec<Box<dyn DiagnosticRule>> {
    let mut rules: Vec<Box<dyn DiagnosticRule>> = Vec::new();

    rules.extend(common::get_rules());

    rules.extend(scan::get_rules());

    rules.extend(join::get_rules());

    rules.extend(aggregate::get_rules());

    rules.extend(sort::get_rules());

    rules.extend(exchange::get_rules());

    rules.extend(fragment::get_rules());

    rules.extend(project::get_rules());

    rules.extend(sink::get_rules());

    rules
}

/// Get query-level rules
pub fn get_query_rules() -> Vec<Box<dyn query::QueryRule>> {
    query::get_rules()
}
