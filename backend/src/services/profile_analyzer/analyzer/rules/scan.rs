//! Scan operator diagnostic rules (S001-S014)
//!
//! Rules for OLAP_SCAN and CONNECTOR_SCAN operators.
//!
//! ## Rule List:
//! - S001: Data skew detection
//! - S002: IO skew detection
//! - S003: Poor filter effectiveness
//! - S004: Predicate not pushed down
//! - S005: IO thread pool saturation
//! - S006: Rowset fragmentation
//! - S007: Cold storage access (IO bound)
//! - S008: ZoneMap index not effective
//! - S009: Low cache hit rate (PageCache + DataCache for disaggregated storage)
//! - S010: Runtime Filter not effective
//! - S011: Accumulated soft deletes
//! - S012: Bitmap index not effective
//! - S013: Bloom filter index not effective
//! - S014: Colocate Join opportunity missed

use super::*;

/// S001: Data skew detection
/// Condition: max(RowsRead)/avg(RowsRead) > threshold (dynamic based on cluster size)
/// Distinguishes between internal tables (bucket key) and external tables (partition skew)
pub struct S001DataSkew;

impl DiagnosticRule for S001DataSkew {
    fn id(&self) -> &str {
        "S001"
    }
    fn name(&self) -> &str {
        "Scan data skew"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("SCAN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let min_rows = context.get_metric("__MIN_OF_RowsRead").unwrap_or(0.0);
        let max_rows = context
            .get_metric("__MAX_OF_RowsRead")
            .or_else(|| context.get_metric("RowsRead"))?;

        if min_rows == 0.0 && max_rows > 0.0 {
            return None;
        }

        let min_rows_threshold = context.thresholds.get_min_rows_for_skew();
        if max_rows < min_rows_threshold {
            return None;
        }

        let avg_rows = (max_rows + min_rows) / 2.0;
        let ratio = max_rows / avg_rows;
        let skew_threshold = context.thresholds.get_skew_threshold();

        if ratio > skew_threshold {
            let table = context.get_full_table_name();
            let is_internal = context.is_internal_table();

            let (reason, suggestions) = if is_internal {
                (
                    format!(
                        "Internal table 「{}」 has an uneven data distribution across nodes (max {:.0} rows, min {:.0} rows). Usually caused by a poorly chosen bucket key.",
                        table, max_rows, min_rows
                    ),
                    vec![
                        format!(
                            "Check whether the bucket key of table 「{}」 is a high-cardinality column",
                            table
                        ),
                        format!(
                            "Inspect the data distribution: SELECT COUNT(*) FROM {} GROUP BY <bucket_key> ORDER BY 1 DESC",
                            table
                        ),
                        format!(
                            "Rebuild the buckets if needed: ALTER TABLE {} DISTRIBUTED BY HASH(<high_cardinality_column>) BUCKETS N",
                            table
                        ),
                    ],
                )
            } else {
                (
                    format!(
                        "External table 「{}」 has an uneven data distribution across nodes (max {:.0} rows, min {:.0} rows). This may be caused by uneven Hive partition sizes or uneven file placement.",
                        table, max_rows, min_rows
                    ),
                    vec![
                        format!(
                            "Check whether the partitions of external table 「{}」 are balanced in size",
                            table
                        ),
                        "Check for hot partitions or oversized files".to_string(),
                        "Consider repartitioning or merging small files on the Hive side"
                            .to_string(),
                    ],
                )
            };

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
                    "Scan has data skew, max/avg ratio is {:.2} (threshold: {:.1})",
                    ratio, skew_threshold
                ),
                reason,
                suggestions,
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// S003: Poor filter effectiveness
/// Condition: RowsRead/RawRowsRead > 0.8 (less than 20% filtered)
/// Distinguishes between internal and external tables for suggestions
pub struct S003PoorFilter;

impl DiagnosticRule for S003PoorFilter {
    fn id(&self) -> &str {
        "S003"
    }
    fn name(&self) -> &str {
        "Poor filter effectiveness"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("SCAN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let rows_read = context.get_metric("RowsRead")?;
        let raw_rows_read = context.get_metric("RawRowsRead")?;
        if raw_rows_read == 0.0 {
            return None;
        }

        let ratio = rows_read / raw_rows_read;
        let min_rows_threshold = context.thresholds.get_min_rows_for_filter();

        if ratio > 0.8 && raw_rows_read > min_rows_threshold {
            let table = context.get_full_table_name();
            let is_internal = context.is_internal_table();

            let (reason, suggestions) = if is_internal {
                (
                    format!(
                        "Internal table 「{}」 scanned {:.0} rows but filtered out only {:.1}%. ZoneMap, Bloom filter indexes or predicate pushdown can filter earlier.",
                        table,
                        raw_rows_read,
                        (1.0 - ratio) * 100.0
                    ),
                    vec![
                        format!("Add a ZoneMap or Bloom filter index for the filtered columns of table 「{}」", table),
                        "Check whether the WHERE conditions support pushdown (avoid wrapping functions or type casts)".to_string(),
                        format!("Check whether the partitions of table 「{}」 can be pruned", table),
                        "Use EXPLAIN to inspect the predicate pushdown".to_string(),
                    ],
                )
            } else {
                (
                    format!(
                        "External table 「{}」 scanned {:.0} rows but filtered out only {:.1}%. Filtering external tables relies on Hive partition pruning and on the statistics of the file format.",
                        table,
                        raw_rows_read,
                        (1.0 - ratio) * 100.0
                    ),
                    vec![
                        format!(
                            "Check whether the Hive partitions of external table 「{}」 can be pruned (the WHERE clause should include the partition column)",
                            table
                        ),
                        "ORC/Parquet files use min/max statistics for filtering, make sure the files carry statistics"
                            .to_string(),
                        "Check whether the WHERE conditions support pushdown to the external storage".to_string(),
                    ],
                )
            };

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
                    "Poor filter effectiveness, only {:.1}% of the data was filtered (read {:.0} rows / raw {:.0} rows)",
                    (1.0 - ratio) * 100.0,
                    rows_read,
                    raw_rows_read
                ),
                reason,
                suggestions,
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// S007: Cold storage access (IO bound)
/// Condition: IOTime/ScanTime > 0.8 && BytesRead > 1GB
pub struct S007ColdStorage;

impl DiagnosticRule for S007ColdStorage {
    fn id(&self) -> &str {
        "S007"
    }
    fn name(&self) -> &str {
        "Cold storage access"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("SCAN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let io_time = context
            .get_metric("IOTime")
            .or_else(|| context.get_metric("IOTaskExecTime"))?;
        let scan_time = context
            .get_metric("ScanTime")
            .or_else(|| context.get_operator_time_ms())?;

        if scan_time == 0.0 {
            return None;
        }

        let bytes_read = context.get_metric("BytesRead").unwrap_or(0.0);
        let ratio = io_time / scan_time;

        const ONE_GB: f64 = 1024.0 * 1024.0 * 1024.0;

        if ratio > 0.8 && bytes_read > ONE_GB {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Warning,
                node_path: format!("{} (plan_node_id={})", 
                    context.node.operator_name,
                    context.node.plan_node_id.unwrap_or(-1)),
                plan_node_id: context.node.plan_node_id,
                message: format!(
                    "IO time share {:.1}%, {} read, there may be a storage bottleneck",
                    ratio * 100.0, format_bytes(bytes_read as u64)
                ),
                reason: "The data is stored on cold storage (e.g. object storage), where IO latency is high. Cold storage usually offers lower IOPS and throughput than a local SSD.".to_string(),
                suggestions: vec![
                    "Check the storage performance, consider using SSD".to_string(),
                    "Increase the PageCache size".to_string(),
                    "Check the network bandwidth (for remote storage)".to_string(),
                ],
                parameter_suggestions: {
                    let mut suggestions = Vec::new();
                    if let Some(s) = context.suggest_parameter_smart("storage_page_cache_limit") {
                        suggestions.push(s);
                    }
                    if let Some(s) = context.suggest_parameter_smart("io_tasks_per_scan_operator") {
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

/// S009: Low cache hit rate (PageCache or DataCache)
/// Condition:
/// - PageCache: CachedPagesNum/ReadPagesNum < threshold
/// - DataCache (disaggregated): CompressedBytesReadLocalDisk/(Local+Remote) < threshold
///   v2.0: Uses dynamic cache hit threshold based on storage type
pub struct S009LowCacheHit;

impl DiagnosticRule for S009LowCacheHit {
    fn id(&self) -> &str {
        "S009"
    }
    fn name(&self) -> &str {
        "Low cache hit rate"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("SCAN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let cache_threshold = context.thresholds.get_cache_hit_threshold();
        let error_threshold = cache_threshold * 0.6;

        let bytes_local = context
            .get_metric("CompressedBytesReadLocalDisk")
            .unwrap_or(0.0);
        let bytes_remote = context
            .get_metric("CompressedBytesReadRemote")
            .unwrap_or(0.0);
        let total_bytes = bytes_local + bytes_remote;

        const MIN_BYTES: f64 = 10.0 * 1024.0 * 1024.0;

        if bytes_remote > 0.0 && total_bytes > MIN_BYTES {
            let hit_rate = bytes_local / total_bytes;

            if hit_rate < cache_threshold {
                let miss_rate = (1.0 - hit_rate) * 100.0;
                return Some(Diagnostic {
                    rule_id: self.id().to_string(),
                    rule_name: self.name().to_string(),
                    severity: if hit_rate < error_threshold { RuleSeverity::Error } else { RuleSeverity::Warning },
                    node_path: format!("{} (plan_node_id={})", 
                        context.node.operator_name,
                        context.node.plan_node_id.unwrap_or(-1)),
                    plan_node_id: context.node.plan_node_id,
                    message: format!(
                        "DataCache hit rate {:.1}%, {:.1}% of the data was read from remote storage (local: {}, remote: {})",
                        hit_rate * 100.0,
                        miss_rate,
                        format_bytes(bytes_local as u64),
                        format_bytes(bytes_remote as u64)
                    ),
                    reason: "In a disaggregated storage architecture, DataCache is the key to query performance. When a large amount of data must be read from remote storage (e.g. S3/OSS), network latency noticeably hurts query performance.".to_string(),
                    suggestions: vec![
                        "Increase the DataCache capacity (datacache_disk_size)".to_string(),
                        "Check whether the DataCache has enough disk space".to_string(),
                        "Warm up the cache for hot data (CACHE SELECT)".to_string(),
                        "Check whether other queries compete for the cache".to_string(),
                    ],

                    parameter_suggestions: [
                        context.suggest_parameter(
                            "enable_scan_datacache",
                            "true",
                            "SET enable_scan_datacache = true;"
                        ),
                        context.suggest_parameter(
                            "enable_populate_datacache",
                            "true",
                            "SET enable_populate_datacache = true;"
                        ),
                    ].into_iter().flatten().collect(),
                    threshold_metadata: None,
                });
            }
        }

        let io_local = context.get_metric("IOCountLocalDisk").unwrap_or(0.0);
        let io_remote = context.get_metric("IOCountRemote").unwrap_or(0.0);
        let total_io = io_local + io_remote;

        if io_remote > 0.0 && total_io > 100.0 {
            let hit_rate = io_local / total_io;

            if hit_rate < cache_threshold {
                return Some(Diagnostic {
                    rule_id: self.id().to_string(),
                    rule_name: self.name().to_string(),
                    severity: if hit_rate < error_threshold { RuleSeverity::Error } else { RuleSeverity::Warning },
                    node_path: format!("{} (plan_node_id={})", 
                        context.node.operator_name,
                        context.node.plan_node_id.unwrap_or(-1)),
                    plan_node_id: context.node.plan_node_id,
                    message: format!(
                        "DataCache IO hit rate {:.1}%, {:.1}% of the IO accesses remote storage (local: {:.0}, remote: {:.0})",
                        hit_rate * 100.0,
                        (1.0 - hit_rate) * 100.0,
                        io_local,
                        io_remote
                    ),
                    reason: "In a disaggregated storage architecture, DataCache is the key to query performance. When a large number of IO requests have to reach remote storage, network latency noticeably hurts query performance.".to_string(),
                    suggestions: vec![
                        "Increase the DataCache capacity (datacache_disk_size)".to_string(),
                        "Warm up the cache for hot data (CACHE SELECT)".to_string(),
                    ],

                    parameter_suggestions: context.suggest_parameter(
                        "enable_scan_datacache",
                        "true",
                        "SET enable_scan_datacache = true;"
                    ).into_iter().collect(),
                    threshold_metadata: None,
                });
            }
        }

        let cached_pages = context.get_metric("CachedPagesNum");
        let read_pages = context.get_metric("ReadPagesNum");

        if let (Some(cached), Some(total)) = (cached_pages, read_pages)
            && total > 1000.0
        {
            let hit_rate = cached / total;

            if hit_rate < 0.3 {
                return Some(Diagnostic {
                        rule_id: self.id().to_string(),
                        rule_name: self.name().to_string(),
                        severity: RuleSeverity::Info,
                        node_path: format!("{} (plan_node_id={})", 
                            context.node.operator_name,
                            context.node.plan_node_id.unwrap_or(-1)),
                        plan_node_id: context.node.plan_node_id,
                        message: format!(
                            "PageCache hit rate is only {:.1}% ({:.0}/{:.0} pages)",
                            hit_rate * 100.0, cached, total
                        ),
                        reason: "The PageCache hit rate is low, so a lot of data has to be read from disk. The cache may be too small, or the access pattern may not suit caching.".to_string(),
                        suggestions: vec![
                            "Increase the PageCache capacity (storage_page_cache_limit)".to_string(),
                            "Check whether other queries compete for the cache".to_string(),
                        ],
                    parameter_suggestions: vec![],
                threshold_metadata: None,
                });
            }
        }

        None
    }
}

/// S010: Runtime Filter not effective on Scan (Internal tables only)
/// Runtime Filter is primarily for internal tables with sorted keys
/// Condition: RuntimeFilterRows == 0 && RawRowsRead > 100k
pub struct S010RFNotEffective;

impl DiagnosticRule for S010RFNotEffective {
    fn id(&self) -> &str {
        "S010"
    }
    fn name(&self) -> &str {
        "Scan Runtime Filter not effective"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("OLAP_SCAN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let rf_rows = context.get_metric("RuntimeFilterRows").unwrap_or(0.0);
        let raw_rows = context.get_metric("RawRowsRead").unwrap_or(0.0);

        if rf_rows == 0.0 && raw_rows > 100_000.0 {
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
                message: format!(
                    "The Runtime Filter filtered no rows, {:.0} rows were scanned",
                    raw_rows
                ),
                reason: {
                    let table = context.get_full_table_name();
                    format!(
                        "Table 「{}」 scanned {:.0} rows but the Runtime Filter filtered nothing. The RF build may have failed, timed out, or have poor selectivity.",
                        table, raw_rows
                    )
                },
                suggestions: {
                    let table = context.get_full_table_name();
                    vec![
                        format!(
                            "Check whether the Join side produced a Runtime Filter for 「{}」",
                            table
                        ),
                        "Make sure enable_global_runtime_filter = true".to_string(),
                        "Check whether the RF was skipped because the build side is too large"
                            .to_string(),
                    ]
                },
                parameter_suggestions: {
                    let mut suggestions = Vec::new();
                    if let Some(s) = context.suggest_parameter_smart("enable_global_runtime_filter")
                    {
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

/// S011: Accumulated soft deletes
/// Condition: DelVecFilterRows/RawRowsRead > 0.3
pub struct S011SoftDeletes;

impl DiagnosticRule for S011SoftDeletes {
    fn id(&self) -> &str {
        "S011"
    }
    fn name(&self) -> &str {
        "Too many accumulated soft deletes"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("OLAP_SCAN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let del_vec_rows = context.get_metric("DelVecFilterRows")?;
        let raw_rows = context.get_metric("RawRowsRead")?;

        if raw_rows == 0.0 {
            return None;
        }

        let ratio = del_vec_rows / raw_rows;

        if ratio > 0.3 {
            let table_name = context
                .node
                .unique_metrics
                .get("Table")
                .map(|s| s.as_str())
                .unwrap_or("unknown_table");

            let full_table_name = if table_name.contains('.') {
                table_name.to_string()
            } else if let Some(db) = context.default_db {
                if db.is_empty() {
                    table_name.to_string()
                } else {
                    format!("{}.{}", db, table_name)
                }
            } else {
                table_name.to_string()
            };

            let compaction_cmd = format!("ALTER TABLE {} COMPACT;", full_table_name);

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
                    "Soft-deleted rows are {:.1}% of table {}, a Compaction is recommended",
                    full_table_name,
                    ratio * 100.0
                ),
                reason: format!(
                    "Table {} contains a large number of soft-deleted records ({:.0} rows), which have to be filtered out during the scan and hurt query performance. Run a Compaction to clear the delete markers.",
                    full_table_name, del_vec_rows
                ),
                suggestions: vec![
                    format!("Run Compaction: {}", compaction_cmd),
                    "Check the Compaction status: SHOW PROC '/compactions';".to_string(),
                    format!(
                        "Inspect the tablet status of the table: SHOW TABLET FROM {};",
                        full_table_name
                    ),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// S002: IO skew detection
/// Condition: max(IOTime)/avg > threshold (dynamic based on cluster size)
/// P0.2: Added sample protection (min 4 samples) and absolute value protection (min 500ms)
/// v2.0: Uses dynamic skew threshold based on cluster parallelism
pub struct S002IOSkew;

impl DiagnosticRule for S002IOSkew {
    fn id(&self) -> &str {
        "S002"
    }
    fn name(&self) -> &str {
        "Scan IO skew"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("SCAN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let max_io = context.get_metric("__MAX_OF_IOTime")?;
        let min_io = context.get_metric("__MIN_OF_IOTime").unwrap_or(0.0);

        if min_io == 0.0 {
            return None;
        }

        use crate::services::profile_analyzer::analyzer::thresholds::defaults::MIN_IO_TIME_NS;

        if max_io < MIN_IO_TIME_NS {
            return None;
        }

        let ratio = max_io / ((max_io + min_io) / 2.0);

        let skew_threshold = context.thresholds.get_skew_threshold();

        if ratio > skew_threshold {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Warning,
                node_path: format!("{} (plan_node_id={})", context.node.operator_name, context.node.plan_node_id.unwrap_or(-1)),
                plan_node_id: context.node.plan_node_id,
                message: format!("Scan IO time is skewed, max/avg ratio is {:.2} (threshold: {:.1})", ratio, skew_threshold),
                reason: "Some instances of the Scan operator take far more time to read the data than the others. This is usually caused by uneven node IO utilization or uneven data placement across nodes.".to_string(),
                suggestions: vec!["Check whether the node IO utilization is uneven".to_string(), "Check whether the storage devices have performance problems".to_string()],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// S004: Predicate not pushed down
/// Distinguishes internal vs external table suggestions
pub struct S004PredicateNotPushed;

impl DiagnosticRule for S004PredicateNotPushed {
    fn id(&self) -> &str {
        "S004"
    }
    fn name(&self) -> &str {
        "Predicate not pushed down"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("SCAN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let pushdown = context.get_metric("PushdownPredicates").unwrap_or(0.0);
        let pred_filter = context.get_metric("PredFilterRows").unwrap_or(0.0);
        let raw_rows = context.get_metric("RawRowsRead").unwrap_or(0.0);

        if pushdown == 0.0 && raw_rows > 10000.0 && pred_filter / raw_rows > 0.1 {
            let is_internal = context.is_internal_table();
            let suggestions = if is_internal {
                vec![
                    "Rewrite the predicates as simple comparisons (avoid wrapping functions)"
                        .to_string(),
                    "Add a ZoneMap/Bloom index for the filtered columns".to_string(),
                    "Check whether the column types match (avoid implicit casts)".to_string(),
                ]
            } else {
                vec![
                    "Rewrite the predicates as simple comparisons (avoid wrapping functions)"
                        .to_string(),
                    "Make sure the filtered column is a Hive partition column".to_string(),
                    "Check whether the ORC/Parquet files carry statistics".to_string(),
                ]
            };

            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Warning,
                node_path: format!("{} (plan_node_id={})", context.node.operator_name, context.node.plan_node_id.unwrap_or(-1)),
                plan_node_id: context.node.plan_node_id,
                message: format!("The predicate was not pushed down to the storage layer, {:.0} rows ({:.1}%) are filtered at the expression layer", pred_filter, pred_filter / raw_rows * 100.0),
                reason: "The query conditions were not pushed down to the storage layer, so a lot of data has to be filtered in the compute layer. This is usually because the conditions wrap functions, have mismatched types, or use expressions that cannot be pushed down.".to_string(),
                suggestions,
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// S005: IO thread pool saturation
pub struct S005IOThreadPoolSaturation;

impl DiagnosticRule for S005IOThreadPoolSaturation {
    fn id(&self) -> &str {
        "S005"
    }
    fn name(&self) -> &str {
        "IO thread pool saturated"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("SCAN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let wait_time = context.get_metric("IOTaskWaitTime").unwrap_or(0.0);
        let peak_tasks = context.get_metric("PeakIOTasks").unwrap_or(100.0);
        if wait_time > 1_000_000_000.0 && peak_tasks < 10.0 {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Warning,
                node_path: format!("{} (plan_node_id={})", context.node.operator_name, context.node.plan_node_id.unwrap_or(-1)),
                plan_node_id: context.node.plan_node_id,
                message: format!("The IO thread pool may be saturated, wait time {:.1}s", wait_time / 1_000_000_000.0),
                reason: "The IO thread pool is over-utilized, so IO tasks wait too long. This is usually caused by too many concurrent queries or by insufficient storage performance.".to_string(),
                suggestions: vec!["Increase the max_io_threads configuration on the BE".to_string()],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// S006: Rowset fragmentation
pub struct S006RowsetFragmentation;

impl DiagnosticRule for S006RowsetFragmentation {
    fn id(&self) -> &str {
        "S006"
    }
    fn name(&self) -> &str {
        "Rowset fragmentation"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("OLAP_SCAN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let rowsets = context.get_metric("RowsetsReadCount").unwrap_or(0.0);
        let init_time = context.get_metric("SegmentInitTime").unwrap_or(0.0);
        if rowsets > 100.0 && init_time > 500_000_000.0 {
            let table = context.get_full_table_name();
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
                    "Too many Rowsets ({:.0}), segment initialization took {:.1}ms",
                    rowsets,
                    init_time / 1_000_000.0
                ),
                reason: format!(
                    "Table 「{}」 has too many Rowsets ({:.0}), which makes the segment initialization take {:.1}ms. This is usually caused by frequent small-batch loads or by Compaction not keeping up.",
                    table,
                    rowsets,
                    init_time / 1_000_000.0
                ),
                suggestions: vec![
                    format!("Trigger a manual Compaction: ALTER TABLE {} COMPACT", table),
                    format!("Check the Compaction status: SHOW TABLET FROM {}", table),
                    "Batch the small load jobs together to reduce the load frequency".to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// S008: ZoneMap index not effective (Internal tables only)
/// ZoneMap is a StarRocks internal table feature, not applicable to external tables
pub struct S008ZoneMapNotEffective;

impl DiagnosticRule for S008ZoneMapNotEffective {
    fn id(&self) -> &str {
        "S008"
    }
    fn name(&self) -> &str {
        "ZoneMap index not effective"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("OLAP_SCAN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let zonemap_rows = context.get_metric("ZoneMapIndexFilterRows").unwrap_or(0.0);
        let raw_rows = context.get_metric("RawRowsRead").unwrap_or(0.0);
        if zonemap_rows == 0.0 && raw_rows > 100000.0 {
            let table = context.get_full_table_name();
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
                message: "The ZoneMap index filtered no data".to_string(),
                reason: format!(
                    "Table 「{}」 scanned {:.0} rows but the ZoneMap filtered no data. The ZoneMap filters on the min/max values of the sort key, so the WHERE clause must include a prefix column of the sort key.",
                    table, raw_rows
                ),
                suggestions: vec![
                    format!("Inspect the sort key of table 「{}」: SHOW CREATE TABLE {}", table, table),
                    "Make sure the WHERE clause includes a prefix column of the sort key (e.g. WHERE dt = '2024-01-01')".to_string(),
                    "Avoid functions on the sort key (e.g. WHERE DATE(dt) = ...)".to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// S012: Bitmap index not effective (Internal tables only)
/// Condition: BitmapIndexFilterRows = 0 with low cardinality column filter
///
/// Reason: Bitmap索引适用于基数较低且大量重复的字段（如性别、状态）。
/// 如果查询条件包含这类字段但未命中Bitmap索引，可能是：
/// 1. 未创建Bitmap索引
/// 2. 查询条件不支持Bitmap索引（如范围查询）
/// 3. 优化器选择了其他索引
pub struct S012BitmapIndexNotEffective;

impl DiagnosticRule for S012BitmapIndexNotEffective {
    fn id(&self) -> &str {
        "S012"
    }
    fn name(&self) -> &str {
        "Bitmap index not effective"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("OLAP_SCAN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let bitmap_rows = context.get_metric("BitmapIndexFilterRows").unwrap_or(0.0);
        let raw_rows = context.get_metric("RawRowsRead").unwrap_or(0.0);

        if bitmap_rows == 0.0 && raw_rows > 100_000.0 {
            let expr_filter = context.get_metric("ExprFilterRows").unwrap_or(0.0);
            if expr_filter > raw_rows * 0.1 {
                return Some(Diagnostic {
                    rule_id: self.id().to_string(),
                    rule_name: self.name().to_string(),
                    severity: RuleSeverity::Info,
                    node_path: format!("{} (plan_node_id={})", 
                        context.node.operator_name,
                        context.node.plan_node_id.unwrap_or(-1)),
                    plan_node_id: context.node.plan_node_id,
                    message: format!(
                        "The Bitmap index filtered no data, the expression filtered {:.0} rows",
                        expr_filter
                    ),
                    reason: "Bitmap indexes fit low-cardinality, highly repetitive columns (e.g. gender, status). If the query conditions contain such a column but do not hit the index, the index may be missing or the conditions may be unsupported.".to_string(),
                suggestions: vec![
                        "Create a Bitmap index for low-cardinality columns (e.g. status, type)".to_string(),
                        "Make sure the query conditions use equality matching (=, IN)".to_string(),
                        "Check the BitmapIndexFilterRows metric in the profile".to_string(),
                    ],
                    parameter_suggestions: vec![],
                threshold_metadata: None,
                });
            }
        }
        None
    }
}

/// S013: Bloom filter index not effective
/// Condition: BloomFilterFilterRows = 0 with high cardinality column filter
///
/// Reason: Bloom Filter索引适用于高基数列（如ID列）的等值查询。
/// 如果查询条件包含这类字段但未命中Bloom Filter索引，可能是：
/// 1. 未创建Bloom Filter索引
/// 2. 查询条件不是等值匹配（Bloom Filter仅支持 = 和 IN）
/// 3. 列类型不支持（TINYINT, FLOAT, DOUBLE, DECIMAL不支持）
pub struct S013BloomFilterNotEffective;

impl DiagnosticRule for S013BloomFilterNotEffective {
    fn id(&self) -> &str {
        "S013"
    }
    fn name(&self) -> &str {
        "Bloom filter index not effective"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("OLAP_SCAN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let bloom_rows = context.get_metric("BloomFilterFilterRows").unwrap_or(0.0);
        let raw_rows = context.get_metric("RawRowsRead").unwrap_or(0.0);

        if bloom_rows == 0.0 && raw_rows > 100_000.0 {
            let expr_filter = context.get_metric("ExprFilterRows").unwrap_or(0.0);
            if expr_filter > raw_rows * 0.5 {
                return Some(Diagnostic {
                    rule_id: self.id().to_string(),
                    rule_name: self.name().to_string(),
                    severity: RuleSeverity::Info,
                    node_path: format!("{} (plan_node_id={})", 
                        context.node.operator_name,
                        context.node.plan_node_id.unwrap_or(-1)),
                    plan_node_id: context.node.plan_node_id,
                    message: format!(
                        "The Bloom filter index filtered no data, the expression filtered {:.0} rows",
                        expr_filter
                    ),
                    reason: "Bloom filter indexes fit equality lookups on high-cardinality columns (e.g. ID columns). They only support = and IN conditions, and the TINYINT/FLOAT/DOUBLE/DECIMAL types are not supported.".to_string(),
                suggestions: vec![
                        "Create a Bloom filter index for high-cardinality columns (e.g. ID columns)".to_string(),
                        "Make sure the query conditions use equality matching (=, IN)".to_string(),
                        "Note: the TINYINT/FLOAT/DOUBLE/DECIMAL types do not support Bloom filters".to_string(),
                        "Check the BloomFilterFilterRows metric in the profile".to_string(),
                    ],
                    parameter_suggestions: vec![],
                threshold_metadata: None,
                });
            }
        }
        None
    }
}

/// S014: Colocate Join opportunity missed
/// Condition: Shuffle Join on tables that could be colocated
///
/// Reason: Colocate Join可以避免数据网络传输，显著提升Join性能。
/// 当两个表的分桶键相同且分桶数相同时，可以使用Colocate Join。
pub struct S014ColocateJoinOpportunity;

impl DiagnosticRule for S014ColocateJoinOpportunity {
    fn id(&self) -> &str {
        "S014"
    }
    fn name(&self) -> &str {
        "Could be optimized into a Colocate Join"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        let name = node.operator_name.to_uppercase();
        name.contains("HASH") && name.contains("JOIN") && name.contains("SHUFFLE")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let bytes_sent = context
            .get_metric("BytesSent")
            .or_else(|| context.get_metric("NetworkBytesSent"))
            .unwrap_or(0.0);

        const HUNDRED_MB: f64 = 100.0 * 1024.0 * 1024.0;

        if bytes_sent > HUNDRED_MB {
            Some(Diagnostic {
                rule_id: self.id().to_string(),
                rule_name: self.name().to_string(),
                severity: RuleSeverity::Info,
                node_path: format!("{} (plan_node_id={})", 
                    context.node.operator_name,
                    context.node.plan_node_id.unwrap_or(-1)),
                plan_node_id: context.node.plan_node_id,
                message: format!(
                    "Shuffle Join transferred {} over the network, consider a Colocate Join",
                    format_bytes(bytes_sent as u64)
                ),
                reason: "A Colocate Join avoids transferring data over the network and greatly improves Join performance. When two tables share the same bucket key and the same bucket count, a Colocate Join can be used.".to_string(),
                suggestions: vec![
                    "Put the frequently joined tables into the same Colocation Group".to_string(),
                    "Make sure both tables use the same bucket key and bucket count".to_string(),
                    "Use SHOW COLOCATION GROUP to list the existing groups".to_string(),
                    "A Colocate Join avoids transferring data over the network and greatly improves performance".to_string(),
                ],
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// S016: Small file detection for external tables (v2.0 updated)
/// Condition: FileCount > threshold AND AvgFileSize < threshold
/// v2.0: Uses ExternalScanType enum for type detection and type-specific suggestions
pub struct S016SmallFiles;

impl DiagnosticRule for S016SmallFiles {
    fn id(&self) -> &str {
        "S016"
    }
    fn name(&self) -> &str {
        "Too many small files in external table"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        use crate::services::profile_analyzer::analyzer::thresholds::ExternalScanType;
        ExternalScanType::from_operator_name(&node.operator_name)
            .map(|t| t.supports_small_file_detection())
            .unwrap_or(false)
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        use crate::services::profile_analyzer::analyzer::thresholds::{
            ExternalScanType, generate_small_file_suggestions,
        };

        let scan_type = ExternalScanType::from_operator_name(&context.node.operator_name)?;

        if !scan_type.supports_small_file_detection() {
            return None;
        }

        let metric_name = scan_type.file_count_metric();
        let file_count = context
            .get_metric(metric_name)
            .or_else(|| context.get_metric("ScanFileCount"))
            .or_else(|| context.get_metric("FileCount"))
            .or_else(|| context.get_metric("TotalFilesNum"))
            .or_else(|| context.get_metric("MorselsCount"))?;

        let total_bytes = context
            .get_metric("BytesRead")
            .or_else(|| context.get_metric("CompressedBytesRead"))?;

        if file_count == 0.0 || total_bytes == 0.0 {
            return None;
        }

        let storage_type = scan_type.storage_type();

        let min_file_count = context.thresholds.get_min_file_count(storage_type) as f64;
        let small_file_threshold = context.thresholds.get_small_file_threshold(storage_type) as f64;

        let avg_file_size = total_bytes / file_count;

        if file_count > min_file_count && avg_file_size < small_file_threshold {
            let table_name = context
                .node
                .unique_metrics
                .get("Table")
                .map(|s| s.as_str())
                .unwrap_or("external_table");

            let suggestions = generate_small_file_suggestions(&scan_type, table_name);

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
                    "Scanned {:.0} files, average size only {} (recommended > {})",
                    file_count,
                    format_bytes(avg_file_size as u64),
                    format_bytes(small_file_threshold as u64)
                ),
                reason: format!(
                    "External table {} of type {} has a large number of small files, which causes a high metadata overhead and low IO efficiency.",
                    table_name,
                    scan_type.display_name()
                ),
                suggestions,
                parameter_suggestions: vec![],
                threshold_metadata: None,
            })
        } else {
            None
        }
    }
}

/// S017: File format fragmentation detection (ORC Stripes / Parquet RowGroups)
/// Detects when files have too many small stripes/rowgroups, causing excessive IO
/// Distinguishes between internal tables (Compaction) and external tables (Hive merge)
pub struct S017FileFragmentation;

impl DiagnosticRule for S017FileFragmentation {
    fn id(&self) -> &str {
        "S017"
    }
    fn name(&self) -> &str {
        "File format fragmentation"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        node.operator_name.to_uppercase().contains("SCAN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        if let Some(diag) = self.check_orc_fragmentation(context) {
            return Some(diag);
        }
        if let Some(diag) = self.check_parquet_fragmentation(context) {
            return Some(diag);
        }
        None
    }
}

impl S017FileFragmentation {
    /// Get suggestions based on table type (internal vs external)
    /// For external tables, provide a single consolidated suggestion to avoid repetition
    fn get_suggestions(is_external: bool, _format: &str, table: &str) -> Vec<String> {
        if is_external {
            vec![format!(
                "External table small file merge plan: ①Hive simple merge: ALTER TABLE {} PARTITION(...) CONCATENATE; \
                 ②Recommended rewrite: INSERT OVERWRITE TABLE {} PARTITION(...) SELECT * FROM {}; \
                 ③Large volumes with Spark: df.repartition(N).saveAsTable('{}'); \
                 ④StarRocks temporary tuning: SET connector_io_tasks_per_scan_operator=64",
                table, table, table, table
            )]
        } else {
            vec![format!("Run Compaction to merge fragments: ALTER TABLE {} COMPACT", table)]
        }
    }

    fn check_orc_fragmentation(&self, context: &RuleContext) -> Option<Diagnostic> {
        let stripe_count = context.get_metric("TotalStripeNumber")?;
        let stripe_size = context.get_metric_bytes("TotalStripeSize").unwrap_or(0.0);
        let tiny_stripe_size = context
            .get_metric_bytes("TotalTinyStripeSize")
            .unwrap_or(0.0);

        if stripe_count <= 10000.0 {
            return None;
        }

        let avg_stripe_size = if stripe_count > 0.0 { stripe_size / stripe_count } else { 0.0 };
        let tiny_ratio =
            if stripe_size > 0.0 { tiny_stripe_size / stripe_size * 100.0 } else { 0.0 };

        if avg_stripe_size >= 64.0 * 1024.0 * 1024.0 && tiny_ratio < 2.0 {
            return None;
        }

        let table = context
            .node
            .unique_metrics
            .get("Table")
            .map(|s| s.as_str())
            .unwrap_or("unknown");
        let is_external = context.is_external_table();
        let severity = if stripe_count > 500000.0 || tiny_ratio > 10.0 {
            RuleSeverity::Error
        } else {
            RuleSeverity::Warning
        };
        let table_type = if is_external { "External table" } else { "Internal table" };

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
                "The ORC files hold {:.0} stripes (average {}), tiny stripes account for {:.1}%",
                stripe_count,
                format_bytes(avg_stripe_size as u64),
                tiny_ratio
            ),
            reason: format!(
                "The ORC files of {}「{}」 are heavily fragmented by stripes ({} in total), which causes a lot of IO requests and file open overhead.",
                table_type, table, stripe_count
            ),
            suggestions: Self::get_suggestions(is_external, "ORC", table),
            parameter_suggestions: vec![],
            threshold_metadata: None,
        })
    }

    fn check_parquet_fragmentation(&self, context: &RuleContext) -> Option<Diagnostic> {
        let total_rowgroups = context.get_metric("TotalRowGroups")?;
        if total_rowgroups <= 10000.0 {
            return None;
        }

        let table = context
            .node
            .unique_metrics
            .get("Table")
            .map(|s| s.as_str())
            .unwrap_or("unknown");
        let is_external = context.is_external_table();
        let severity =
            if total_rowgroups > 100000.0 { RuleSeverity::Error } else { RuleSeverity::Warning };
        let table_type = if is_external { "External table" } else { "Internal table" };

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
                "The Parquet files hold {:.0} row groups, the files are heavily fragmented",
                total_rowgroups
            ),
            reason: format!(
                "The Parquet files of {}「{}」 have too many row groups ({} in total), which causes a high metadata overhead and low IO efficiency.",
                table_type, table, total_rowgroups
            ),
            suggestions: Self::get_suggestions(is_external, "Parquet", table),
            parameter_suggestions: vec![],
            threshold_metadata: None,
        })
    }
}

/// S018: IO wait time detection for HDFS/external scans
/// Condition: IOTaskWaitTime > threshold (significant IO queue waiting)
pub struct S018IOWaitTime;

impl DiagnosticRule for S018IOWaitTime {
    fn id(&self) -> &str {
        "S018"
    }
    fn name(&self) -> &str {
        "IO wait time too long"
    }

    fn applicable_to(&self, node: &ExecutionTreeNode) -> bool {
        let op = node.operator_name.to_uppercase();
        op.contains("SCAN")
    }

    fn evaluate(&self, context: &RuleContext) -> Option<Diagnostic> {
        let io_wait = context
            .get_metric_duration("IOTaskWaitTime")
            .or_else(|| context.get_metric_duration("__MAX_OF_IOTaskWaitTime"))?;

        let io_exec = context
            .get_metric_duration("IOTaskExecTime")
            .or_else(|| context.get_metric_duration("__MAX_OF_IOTaskExecTime"))
            .unwrap_or(0.0);

        let wait_threshold_ms = 10_000.0;
        if io_wait < wait_threshold_ms {
            return None;
        }

        let total_io = io_wait + io_exec;
        let wait_ratio = if total_io > 0.0 { io_wait / total_io * 100.0 } else { 0.0 };

        let severity = if io_wait > 60_000.0 { RuleSeverity::Error } else { RuleSeverity::Warning };

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
                "IO wait time {} ({:.1}% of the total IO time), there is an IO queueing bottleneck",
                format_duration_ms(io_wait),
                wait_ratio
            ),
            reason: "A large number of concurrent IO requests are waiting in the queue, usually caused by file fragmentation or by insufficient IO resources".to_string(),
            suggestions: vec![
                "Merge small files to reduce the number of IO requests".to_string(),
                "Increase the io_tasks_per_scan_operator parameter".to_string(),
                "Check the IO performance of the storage system".to_string(),
                "Consider enabling the Data Cache for hot data".to_string(),
            ],
            parameter_suggestions: vec![],
            threshold_metadata: None,
        })
    }
}

/// Get all scan rules
pub fn get_rules() -> Vec<Box<dyn DiagnosticRule>> {
    vec![
        Box::new(S001DataSkew),
        Box::new(S002IOSkew),
        Box::new(S003PoorFilter),
        Box::new(S004PredicateNotPushed),
        Box::new(S005IOThreadPoolSaturation),
        Box::new(S006RowsetFragmentation),
        Box::new(S007ColdStorage),
        Box::new(S008ZoneMapNotEffective),
        Box::new(S009LowCacheHit),
        Box::new(S010RFNotEffective),
        Box::new(S011SoftDeletes),
        Box::new(S012BitmapIndexNotEffective),
        Box::new(S013BloomFilterNotEffective),
        Box::new(S014ColocateJoinOpportunity),
        Box::new(S016SmallFiles),
        Box::new(S017FileFragmentation),
        Box::new(S018IOWaitTime),
    ]
}
