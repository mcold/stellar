//! Root Cause Analysis Scenario
//!
//! LLM-enhanced root cause analysis for query profile diagnostics.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::services::llm::models::LLMScenario;
use crate::services::llm::service::{LLMAnalysisRequestTrait, LLMAnalysisResponseTrait};

// ============================================================================
// System Prompt - Dynamic Generation
// ============================================================================

/// Base system prompt - the static foundation
const PROMPT_BASE: &str = r#"You are a senior StarRocks OLAP database performance expert with more than 20 years of experience.
You need to analyze Query Profile data, identify the true root cause, and give **directly executable** optimization suggestions.

## 🧠 Critical Thinking Requirements

Before giving any diagnosis or suggestion, you must perform **self-critical thinking**:

1. **Question assumptions**: Is my diagnosis based on sufficient evidence? Are there other possible explanations?
2. **Verify parameters**: Do the parameters I recommend really exist in the official StarRocks documentation? If unsure, better not to recommend it.
3. **Check applicability**: Does this suggestion apply to the current table type (internal/external)?
4. **Avoid speculation**: Have I made any guess without supporting data?
5. **Reflect on bias**: Am I over-relying on common patterns while ignoring the specific situation?

**Important principle**: Better to give fewer suggestions than to give wrong ones or to recommend non-existent parameters!

## Analysis Methodology (Chain-of-Thought)

### Step 1: Understand the query intent
- What type of query is this? (OLAP aggregation / point lookup / ETL load / Join-heavy)
- Which tables are involved? What is the data volume of each table?
- Ask yourself: Do I fully understand the business scenario of this query?

### Step 2: Identify the performance bottleneck
- Which operator takes the longest time? (time_pct > 30%)
- Is it an IO bottleneck or a CPU bottleneck?
- Is there data skew? (max/avg ratio)
- Ask yourself: Is my judgement supported by Profile metrics?

### Step 3: Trace the root cause
- What is upstream of the bottleneck operator?
- Is the root cause a data problem or a configuration problem?
- Are there implicit root causes the rule engine did not detect?
- Ask yourself: Am I confusing the symptom with the root cause?

### Step 4: Design the optimization plan
- Give suggestions for the root cause, not for the symptom
- Prioritize the optimizations with the highest return on investment
- Must be directly executable commands
- Ask yourself: Is this suggestion feasible in the user's environment?

### Step 5: Self-verification (mandatory)
- **Parameter existence**: Is every parameter I recommend listed in the "officially supported parameter list" below?
- **Table type match**: Recommending ALTER TABLE bucketing for an external table is wrong!
- **Configuration conflict**: Does it duplicate a value already present in session_variables?
- **Command completeness**: Can the SQL/SET command be copied and executed directly?

## ⚠️ Rules that must be strictly followed
1. **Check session_variables before suggesting**: do not suggest a parameter that is already enabled
2. **Distinguish table types**: internal and external tables require completely different optimization directions
3. **Parameters must exist**: only use the official StarRocks parameters listed below, never invent parameters!
4. **Suggestions must be executable**: give complete SQL/SET/ALTER commands
5. **Quality over quantity**: for an uncertain suggestion, better to give none than to mislead the user"#;

/// Dynamic prompt section for table types detected
fn build_table_type_prompt(scan_details: &[ScanDetailForLLM]) -> String {
    let mut internal_tables = Vec::new();
    let mut external_tables: HashMap<String, Vec<String>> = HashMap::new();

    for scan in scan_details {
        let table_name = &scan.table_name;
        if scan.table_type == "internal" {
            internal_tables.push(table_name.clone());
        } else {
            let connector = scan
                .connector_type
                .clone()
                .unwrap_or_else(|| "unknown".to_string());
            external_tables
                .entry(connector)
                .or_default()
                .push(table_name.clone());
        }
    }

    let mut prompt = String::from("\n\n## 📊 Tables Involved in This Query\n");

    if !internal_tables.is_empty() {
        prompt.push_str(&format!(
            "\n### StarRocks Internal Tables ({} tables)\nTable names: {}\n\n**Internal table optimization directions:**\n- ANALYZE TABLE to update statistics\n- Check whether the bucketing key is reasonable\n- Consider a materialized view to speed things up\n- ALTER TABLE can be used to adjust properties\n",
            internal_tables.len(),
            internal_tables.join(", ")
        ));
    }

    for (connector, tables) in &external_tables {
        let connector_prompt = match connector.as_str() {
            "hive" => format!(
                "\n### Hive External Tables ({} tables)\nTable names: {}\n\n**Hive table optimization directions:**\n- Enable DataCache: `SET enable_scan_datacache=true;`\n- Partition pruning: make sure the WHERE condition contains the partition column\n- Small file merging: run `ALTER TABLE xxx CONCATENATE;` on the Hive/Spark side\n- ⚠️ ALTER TABLE cannot change the bucketing key, do it on the Hive side\n",
                tables.len(),
                tables.join(", ")
            ),
            "iceberg" => format!(
                "\n### Iceberg External Tables ({} tables)\nTable names: {}\n\n**Iceberg table optimization directions:**\n- Enable DataCache: `SET enable_scan_datacache=true;`\n- File merging: use the Spark `rewrite_data_files` procedure\n- Leverage the hidden partitioning of Iceberg\n- Check whether there are too many delete files (V2 format)\n- ⚠️ ALTER TABLE cannot change the bucketing key, do it on the Iceberg side\n",
                tables.len(),
                tables.join(", ")
            ),
            "hudi" => format!(
                "\n### Hudi External Tables ({} tables)\nTable names: {}\n\n**Hudi table optimization directions:**\n- Enable DataCache\n- Check whether compaction keeps up\n- Consider adjusting the read mode for MOR tables\n",
                tables.len(),
                tables.join(", ")
            ),
            "jdbc" => format!(
                "\n### JDBC External Tables ({} tables)\nTable names: {}\n\n**JDBC table optimization directions:**\n- Predicate pushdown: make sure the WHERE condition can be pushed down to the source database\n- Reduce the SELECT columns: query only the columns that are needed\n- Consider syncing the data into an internal table to speed things up\n",
                tables.len(),
                tables.join(", ")
            ),
            "es" => format!(
                "\n### Elasticsearch External Tables ({} tables)\nTable names: {}\n\n**ES table optimization directions:**\n- Make sure the query condition can be pushed down to ES\n- Leverage the indexing capability of ES\n- Reduce the number of returned fields\n",
                tables.len(),
                tables.join(", ")
            ),
            _ => format!(
                "\n### {} External Tables ({} tables)\nTable names: {}\n\n**Generic external table optimization directions:**\n- Enable DataCache\n- Partition pruning\n- Predicate pushdown\n",
                connector,
                tables.len(),
                tables.join(", ")
            ),
        };
        prompt.push_str(&connector_prompt);
    }

    prompt
}

/// Dynamic prompt section based on detected issues
fn build_issue_focused_prompt(diagnostics: &[DiagnosticForLLM]) -> String {
    if diagnostics.is_empty() {
        return String::from(
            "\n\n## The Rule Engine Found No Obvious Issue\nPlease analyze the raw Profile data in depth to find implicit performance problems.\n",
        );
    }

    let mut prompt = String::from("\n\n## Issues Identified by the Rule Engine (for reference only)\n");
    for d in diagnostics.iter().take(5) {
        prompt.push_str(&format!("- **{}** [{}]: {}\n", d.rule_id, d.severity, d.message));
    }
    prompt.push_str("\n**Your task**: do not simply repeat these issues, but:\n1. Analyze the root causes behind these symptoms\n2. Find implicit issues the rule engine did not detect\n3. Build the causal chain\n");

    prompt
}

/// Dynamic prompt section for current session variables
///
/// Uses ALL passed session_vars (already filtered by CLUSTER_VARIABLE_NAMES at fetch time).
/// Dynamically detects `enable_*` prefix for boolean feature flags.
fn build_session_vars_prompt(session_vars: &HashMap<String, String>) -> String {
    if session_vars.is_empty() {
        return String::new();
    }

    let mut prompt = String::from("\n\n## ⚠️ Current Cluster Configuration (strictly forbidden to suggest again!)\n");

    let mut enabled_features = Vec::new();
    let mut disabled_features = Vec::new();
    let mut other_settings = Vec::new();

    for (var, value) in session_vars {
        let is_bool_flag = var.starts_with("enable_");
        let is_true = value == "true" || value == "1";

        if is_bool_flag {
            if is_true {
                enabled_features.push(var.as_str());
            } else {
                disabled_features.push(var.as_str());
            }
        } else {
            other_settings.push((var.as_str(), value.as_str()));
        }
    }

    enabled_features.sort();
    disabled_features.sort();
    other_settings.sort_by_key(|(k, _)| *k);

    if !enabled_features.is_empty() {
        prompt.push_str(&format!(
            "\n### 🟢 Enabled Features (forbidden to suggest enabling again!)\n{}\n",
            enabled_features
                .iter()
                .map(|v| format!("`{}`", v))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if !disabled_features.is_empty() {
        prompt.push_str(&format!(
            "\n### 🔴 Disabled Features (may be suggested for enabling)\n{}\n",
            disabled_features
                .iter()
                .map(|v| format!("`{}`", v))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    if !other_settings.is_empty() {
        prompt.push_str("\n### Other Settings\n");
        for (var, value) in &other_settings {
            prompt.push_str(&format!("- `{}` = `{}`\n", var, value));
        }
    }

    prompt.push_str(
        r#"
### 🚫 Strict Rules
1. **Never suggest** `SET enable_xxx = true` if the parameter is in the "Enabled Features" list
2. Only suggest enabling parameters that are in the "Disabled Features" list
3. Violating the rules above is considered a serious error!
"#,
    );

    prompt
}

/// Static prompt section for valid parameters (verified from StarRocks official docs)
const PROMPT_VALID_PARAMS: &str = r#"

## ✅ Officially Supported StarRocks Parameters (verified)

All the following parameters come from the official StarRocks documentation and are safe to use. If the parameter you want to recommend is not in this list, do not recommend it!

### Session Variables (SET xxx = yyy)

**Query resource control:**
- `query_mem_limit` - memory limit of a single query (bytes)
- `query_timeout` - query timeout (seconds, default 300)
- `exec_mem_limit` - memory limit of a single BE node

**Parallelism control:**
- `pipeline_dop` - Pipeline parallelism (0=auto)
- `parallel_fragment_exec_instance_num` - number of Fragment instances (default 1)
- `max_parallel_scan_instance_num` - number of Scan instances

**Spill (to disk):**
- `enable_spill` - enable spilling (true/false)
- `spill_mem_table_size` - threshold that triggers spilling
- `spill_mem_table_num` - number of spill tables

**DataCache (external tables only! Hive/Iceberg/Hudi etc.):**
- `enable_scan_datacache` - enable DataCache reads (external tables only)
- `enable_populate_datacache` - enable DataCache writes (external tables only)
- ⚠️ Internal tables need no DataCache configuration, they use PageCache (automatic)

**Query Cache (internal tables only! external tables are not supported!):**
- `enable_query_cache` - enable Query Cache (internal table aggregation queries only)
- `query_cache_entry_max_bytes` - maximum bytes of a single cache entry
- `query_cache_entry_max_rows` - maximum rows of a single cache entry
- ⚠️ Query Cache restrictions:
  - Only native OLAP tables and compute-storage separated tables are supported, **external tables are not supported**!
  - Only aggregation queries are supported (no GROUP BY or low-cardinality GROUP BY)
  - Non-deterministic functions such as rand/random/uuid/sleep are not supported
  - It only takes effect when the number of Tablets >= pipeline_dop
  - High-cardinality GROUP BY automatically bypasses the cache

**Runtime Filter:**
- `enable_global_runtime_filter` - global Runtime Filter
- `runtime_filter_wait_time_ms` - wait time
- `runtime_join_filter_push_down_limit` - limit of pushed down rows

**Join optimization:**
- `broadcast_row_limit` - Broadcast row limit (default 25M)
- `hash_join_push_down_right_table` - push down the right table

**Aggregation optimization:**
- `new_planner_agg_stage` - aggregation stage (0=auto,1/2/3/4)
- `streaming_preaggregation_mode` - pre-aggregation mode

### ALTER TABLE properties (StarRocks internal tables only!)

- `replication_num` - number of replicas
- `bloom_filter_columns` - Bloom Filter columns
- `colocate_with` - Colocate Group name
- `dynamic_partition.enable` - dynamic partition switch
- `storage_medium` - storage medium (SSD/HDD)

### Operations commands

- `ANALYZE TABLE db.table;` - update statistics (internal tables only)
- `REFRESH MATERIALIZED VIEW mv_name;` - refresh the materialized view
- `ADMIN SET REPLICA STATUS ...` - manage replicas

### SQL Hint format

```sql
SELECT /*+ SET_VAR(query_timeout=600, enable_spill=true) */ ...
```

## ❌ Forbidden parameters (non-existent or deprecated)

The following parameters **do not exist** in StarRocks and must not be recommended:
- ❌ `enable_short_key_index` - does not exist! Short Key is automatic
- ❌ `enable_zone_map_index` - does not exist! Zone Map is automatic
- ❌ `enable_bitmap_index` - does not exist! Use CREATE INDEX to build an index
- ❌ `enable_async_profile` - does not exist
- ❌ `enable_query_debug_trace` - does not exist
- ❌ `optimize_table` - does not exist! Use ADMIN COMPACT for internal tables
- ❌ any parameter you "guess" might exist

## ⚠️ External table restrictions (Hive/Iceberg/JDBC etc.)

External tables **do not support** the following operations, do not suggest them:
- ❌ `ALTER TABLE external_table SET ("xxx" = "yyy")` - external table properties are changed on the source side
- ❌ `ANALYZE TABLE external_catalog.db.table` - external table statistics live on the source side
- ❌ any suggestion that changes the bucketing/partitioning of an external table
- ❌ `enable_query_cache = true` - Query Cache does not support external tables! Use DataCache for external tables!

## 🔄 Cache strategy summary

| Cache type | Applicable table type | Parameter | Description |
|---------|-----------|------|------|
| Query Cache | Internal tables | `enable_query_cache` | caches aggregation results |
| DataCache | External tables | `enable_scan_datacache` | caches remote data locally |
| PageCache | Internal tables | automatic | caches disk data pages, no configuration needed |
"#;

/// Output format specification
const PROMPT_OUTPUT_FORMAT: &str = r#"

## 📤 Strict JSON Output Format"#;

/// Build the complete dynamic system prompt
pub fn build_system_prompt(request: &RootCauseAnalysisRequest) -> String {
    let mut prompt = String::from(PROMPT_BASE);

    if let Some(ref profile_data) = request.profile_data {
        prompt.push_str(&build_table_type_prompt(&profile_data.scan_details));
    }

    prompt.push_str(&build_issue_focused_prompt(&request.rule_diagnostics));

    prompt.push_str(&build_session_vars_prompt(&request.query_summary.session_variables));

    prompt.push_str(PROMPT_VALID_PARAMS);

    prompt.push_str(PROMPT_OUTPUT_FORMAT);

    prompt.push_str(PROMPT_JSON_FORMAT);

    prompt
}

/// Output format JSON schema (appended to dynamic prompt)
const PROMPT_JSON_FORMAT: &str = r#"

```json
{
  "root_causes": [
    {
      "root_cause_id": "RC001",
      "description": "root cause description based on raw metrics analysis",
      "confidence": 0.85,
      "evidence": ["Profile metric evidence 1", "evidence 2"],
      "symptoms": ["S001", "G003"],
      "is_implicit": false
    }
  ],
  "causal_chains": [
    {
      "chain": ["Root Cause", "->", "Intermediate", "->", "Symptom"],
      "explanation": "Causal analysis based on Profile data"
    }
  ],
  "recommendations": [
    {
      "priority": 1,
      "action": "Brief description of recommended action",
      "expected_improvement": "Quantitative improvement description",
      "sql_example": "Executable SQL or command"
    }
  ],
  "summary": "Overall analysis summary focusing on root causes and optimization direction",
  "hidden_issues": [
    {
      "issue": "Issue not detected by rule engine",
      "suggestion": "Executable solution command"
    }
  ]
}
```

Field descriptions:
- root_cause_id: Format as "RC001", "RC002", etc.
- evidence: MUST reference specific Profile metric values
- symptoms: Related rule IDs
- is_implicit: true if not detected by rule engine
- priority: 1 is highest priority
- sql_example: REQUIRED, executable SQL/command
"#;

/// Legacy static prompt for backward compatibility (minimal)
#[allow(dead_code)]
pub const ROOT_CAUSE_SYSTEM_PROMPT: &str = "You are a StarRocks OLAP database performance expert.";

// ============================================================================
// Request Types
// ============================================================================

/// Root Cause Analysis Request to LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootCauseAnalysisRequest {
    /// Query summary information
    pub query_summary: QuerySummaryForLLM,
    /// Raw profile data for deep analysis (NEW - 原始 Profile 数据)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_data: Option<ProfileDataForLLM>,
    /// Execution plan (simplified for token efficiency)
    pub execution_plan: ExecutionPlanForLLM,
    /// Rule engine diagnostics (for reference, LLM should go deeper)
    pub rule_diagnostics: Vec<DiagnosticForLLM>,
    /// Key performance metrics
    pub key_metrics: KeyMetricsForLLM,
    /// Optional user question for follow-up
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_question: Option<String>,
}

impl LLMAnalysisRequestTrait for RootCauseAnalysisRequest {
    fn scenario(&self) -> LLMScenario {
        LLMScenario::RootCauseAnalysis
    }

    /// Generate dynamic system prompt based on request context
    fn system_prompt(&self) -> String {
        build_system_prompt(self)
    }

    fn cache_key(&self) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.sql_hash().hash(&mut hasher);
        self.profile_hash().hash(&mut hasher);
        format!("rca:{:x}", hasher.finish())
    }

    fn sql_hash(&self) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.query_summary.sql_statement.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    fn profile_hash(&self) -> String {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();

        self.query_summary.scan_bytes.hash(&mut hasher);
        self.query_summary.output_rows.hash(&mut hasher);
        self.rule_diagnostics.len().hash(&mut hasher);

        self.query_summary.query_type.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }
}

/// Query summary for LLM analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuerySummaryForLLM {
    /// Full SQL statement (NOT truncated - LLM needs complete SQL for analysis)
    pub sql_statement: String,
    /// Query type: SELECT/INSERT/EXPORT/ANALYZE
    pub query_type: String,
    /// Query complexity level: "Simple" | "Medium" | "Complex" | "VeryComplex"
    /// Used for adaptive threshold selection and LLM context
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_complexity: Option<String>,
    /// Total execution time in seconds
    pub total_time_seconds: f64,
    /// Total bytes scanned
    pub scan_bytes: u64,
    /// Output row count
    pub output_rows: u64,
    /// Number of BE nodes
    pub be_count: u32,
    /// Whether spill occurred
    pub has_spill: bool,
    /// Spill details if spill occurred
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spill_bytes: Option<String>,
    /// Non-default session variables (important for analysis)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub session_variables: HashMap<String, String>,
}

// ============================================================================
// Raw Profile Data - NEW: 原始 Profile 数据
// ============================================================================

/// Raw profile data for LLM deep analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileDataForLLM {
    /// All operator nodes with their metrics
    pub operators: Vec<OperatorDetailForLLM>,
    /// Cross-node time distribution (for detecting skew)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_distribution: Option<TimeDistributionForLLM>,
    /// Scan node details (tables, partitions, files)
    #[serde(default)]
    pub scan_details: Vec<ScanDetailForLLM>,
    /// Join node details (join type, build/probe stats)
    #[serde(default)]
    pub join_details: Vec<JoinDetailForLLM>,
    /// Aggregation node details
    #[serde(default)]
    pub agg_details: Vec<AggDetailForLLM>,
    /// Exchange (shuffle) details
    #[serde(default)]
    pub exchange_details: Vec<ExchangeDetailForLLM>,
}

/// Detailed operator information with all metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorDetailForLLM {
    /// Operator name (SCAN, JOIN, AGG, etc.)
    pub operator: String,
    /// Plan node ID
    pub plan_node_id: i32,
    /// Execution time percentage
    pub time_pct: f64,
    /// Actual rows processed
    pub rows: u64,
    /// Estimated rows (for cardinality error detection)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_rows: Option<u64>,
    /// Memory used in bytes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_bytes: Option<u64>,
    /// All key metrics (raw from profile)
    pub metrics: HashMap<String, String>,
}

/// Time distribution across instances for skew detection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeDistributionForLLM {
    /// Max time across instances
    pub max_time_ms: f64,
    /// Min time across instances
    pub min_time_ms: f64,
    /// Average time
    pub avg_time_ms: f64,
    /// Skew ratio (max/avg)
    pub skew_ratio: f64,
    /// Per-instance times for top operators
    #[serde(default)]
    pub per_instance: Vec<InstanceTimeForLLM>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceTimeForLLM {
    pub operator: String,
    pub instance_id: i32,
    pub time_ms: f64,
    pub rows: u64,
}

/// Scan operator details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanDetailForLLM {
    pub plan_node_id: i32,
    pub table_name: String,
    /// OlapScan / HdfsScan / ConnectorScan etc.
    pub scan_type: String,
    /// Table storage type: "internal" (StarRocks native), "external" (foreign table)
    /// This is CRITICAL for LLM to give correct suggestions!
    pub table_type: String,
    /// Connector type for external tables: "hive", "iceberg", "hudi", "deltalake", "paimon", "jdbc", "es", "unknown"
    /// For internal tables this is "native"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connector_type: Option<String>,
    /// Total rows read
    pub rows_read: u64,
    /// Rows after filtering
    pub rows_returned: u64,
    /// Filter ratio
    pub filter_ratio: f64,
    /// Scan ranges (file/tablet count)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scan_ranges: Option<u64>,
    /// Bytes read
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_read: Option<u64>,
    /// IO wait time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub io_time_ms: Option<f64>,
    /// Cache hit rate (DataCache for external, PageCache for internal)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_hit_rate: Option<f64>,
    /// Predicates applied
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicates: Option<String>,
    /// Partition pruning info
    #[serde(skip_serializing_if = "Option::is_none")]
    pub partitions_scanned: Option<String>,
    /// For external tables: catalog.database.table format
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_table_path: Option<String>,
}

/// Join operator details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinDetailForLLM {
    pub plan_node_id: i32,
    /// HASH_JOIN, CROSS_JOIN, etc.
    pub join_type: String,
    /// Build side rows
    pub build_rows: u64,
    /// Probe side rows
    pub probe_rows: u64,
    /// Output rows
    pub output_rows: u64,
    /// Hash table memory
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash_table_memory: Option<u64>,
    /// Is broadcast join
    #[serde(default)]
    pub is_broadcast: bool,
    /// Runtime filter info
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_filter: Option<String>,
}

/// Aggregation operator details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AggDetailForLLM {
    pub plan_node_id: i32,
    /// Input rows
    pub input_rows: u64,
    /// Output rows after aggregation
    pub output_rows: u64,
    /// Aggregation ratio (output/input)
    pub agg_ratio: f64,
    /// GROUP BY keys
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_by_keys: Option<String>,
    /// Hash table memory
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash_table_memory: Option<u64>,
    /// Is streaming agg
    #[serde(default)]
    pub is_streaming: bool,
}

/// Exchange (shuffle) operator details
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExchangeDetailForLLM {
    pub plan_node_id: i32,
    /// SHUFFLE, BROADCAST, GATHER
    pub exchange_type: String,
    /// Data sent bytes
    pub bytes_sent: u64,
    /// Rows sent
    pub rows_sent: u64,
    /// Network time
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_time_ms: Option<f64>,
}

/// Simplified execution plan for LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPlanForLLM {
    /// DAG description in text format
    /// e.g., "SCAN(orders) -> JOIN -> SCAN(customers) -> AGG -> SINK"
    pub dag_description: String,
    /// Hotspot nodes (time_percentage > 15%)
    #[serde(default)]
    pub hotspot_nodes: Vec<HotspotNodeForLLM>,
}

/// Hotspot node information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotspotNodeForLLM {
    /// Operator name, e.g., "HASH_JOIN"
    pub operator: String,
    /// Plan node ID
    pub plan_node_id: i32,
    /// Time percentage (0-100)
    pub time_percentage: f64,
    /// Key metrics relevant to this operator
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub key_metrics: HashMap<String, String>,
    /// Upstream operator names
    #[serde(default)]
    pub upstream_operators: Vec<String>,
}

/// Rule engine diagnostic result for LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticForLLM {
    /// Rule ID, e.g., "S001"
    pub rule_id: String,
    /// Severity: Error/Warning/Info
    pub severity: String,
    /// Affected operator
    pub operator: String,
    /// Plan node ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_node_id: Option<i32>,
    /// Diagnostic message
    pub message: String,
    /// Evidence that triggered the rule
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub evidence: HashMap<String, String>,
    /// Threshold metadata for traceability (how the threshold was determined)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold_info: Option<ThresholdInfoForLLM>,
}

/// Threshold information for LLM to understand how diagnostics were triggered
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdInfoForLLM {
    /// Threshold value used (in appropriate unit, e.g., ms for time)
    pub threshold_value: f64,
    /// Source: "baseline" (adaptive from history) or "default" (static config)
    pub source: String,
    /// If baseline was used, P95 value from historical data (ms)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_p95_ms: Option<f64>,
    /// Number of samples used to calculate baseline
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample_count: Option<usize>,
}

/// Key performance metrics
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KeyMetricsForLLM {
    /// Data skew metrics
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skew_metrics: Option<SkewMetricsForLLM>,
    /// IO metrics
    #[serde(skip_serializing_if = "Option::is_none")]
    pub io_metrics: Option<IOMetricsForLLM>,
    /// Memory metrics
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_metrics: Option<MemoryMetricsForLLM>,
    /// Cardinality estimation errors
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cardinality_errors: Vec<CardinalityErrorForLLM>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkewMetricsForLLM {
    pub max_rows: u64,
    pub min_rows: u64,
    pub avg_rows: f64,
    pub skew_ratio: f64,
    pub affected_operator: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IOMetricsForLLM {
    pub total_bytes_read: u64,
    pub cache_hit_rate: f64,
    pub io_time_percentage: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryMetricsForLLM {
    pub peak_memory_bytes: u64,
    pub spill_bytes: u64,
    pub hash_table_memory: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardinalityErrorForLLM {
    pub operator: String,
    pub estimated_rows: u64,
    pub actual_rows: u64,
    pub error_ratio: f64,
}

// ============================================================================
// Response Types
// ============================================================================

/// Root Cause Analysis Response from LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootCauseAnalysisResponse {
    /// Identified root causes
    #[serde(default)]
    pub root_causes: Vec<LLMRootCause>,
    /// Causal chains with explanations
    #[serde(default)]
    pub causal_chains: Vec<LLMCausalChain>,
    /// Prioritized recommendations
    #[serde(default)]
    pub recommendations: Vec<LLMRecommendation>,
    /// Summary in natural language
    #[serde(default)]
    pub summary: String,
    /// Hidden issues not detected by rule engine
    #[serde(default)]
    pub hidden_issues: Vec<LLMHiddenIssue>,
}

impl LLMAnalysisResponseTrait for RootCauseAnalysisResponse {
    fn summary(&self) -> &str {
        &self.summary
    }

    fn confidence(&self) -> Option<f64> {
        if self.root_causes.is_empty() {
            None
        } else {
            Some(
                self.root_causes.iter().map(|r| r.confidence).sum::<f64>()
                    / self.root_causes.len() as f64,
            )
        }
    }
}

/// Root cause identified by LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMRootCause {
    /// Unique ID for this root cause
    pub root_cause_id: String,
    /// Description of the root cause
    pub description: String,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f64,
    /// Evidence supporting this conclusion
    #[serde(default)]
    pub evidence: Vec<String>,
    /// Symptom rule IDs caused by this root cause
    #[serde(default)]
    pub symptoms: Vec<String>,
    /// Whether this is an implicit root cause (not detected by rules)
    #[serde(default)]
    pub is_implicit: bool,
}

/// Causal chain with explanation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMCausalChain {
    /// Chain representation, e.g., ["统计信息过期", "→", "Join顺序不优", "→", "内存过高"]
    pub chain: Vec<String>,
    /// Natural language explanation
    pub explanation: String,
}

/// Recommendation from LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMRecommendation {
    /// Priority (1 = highest)
    pub priority: u32,
    /// Action to take
    pub action: String,
    /// Expected improvement
    #[serde(default)]
    pub expected_improvement: String,
    /// SQL example if applicable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sql_example: Option<String>,
}

/// Hidden issue not detected by rule engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LLMHiddenIssue {
    /// Issue description
    pub issue: String,
    /// Suggested action
    pub suggestion: String,
}

// ============================================================================
// Builder for RootCauseAnalysisRequest
// ============================================================================

impl RootCauseAnalysisRequest {
    /// Create a new builder
    pub fn builder() -> RootCauseAnalysisRequestBuilder {
        RootCauseAnalysisRequestBuilder::default()
    }
}

#[derive(Default)]
pub struct RootCauseAnalysisRequestBuilder {
    query_summary: Option<QuerySummaryForLLM>,
    profile_data: Option<ProfileDataForLLM>,
    execution_plan: Option<ExecutionPlanForLLM>,
    rule_diagnostics: Vec<DiagnosticForLLM>,
    key_metrics: KeyMetricsForLLM,
    user_question: Option<String>,
}

impl RootCauseAnalysisRequestBuilder {
    pub fn query_summary(mut self, summary: QuerySummaryForLLM) -> Self {
        self.query_summary = Some(summary);
        self
    }

    pub fn profile_data(mut self, data: ProfileDataForLLM) -> Self {
        self.profile_data = Some(data);
        self
    }

    pub fn execution_plan(mut self, plan: ExecutionPlanForLLM) -> Self {
        self.execution_plan = Some(plan);
        self
    }

    pub fn add_diagnostic(mut self, diag: DiagnosticForLLM) -> Self {
        self.rule_diagnostics.push(diag);
        self
    }

    pub fn diagnostics(mut self, diags: Vec<DiagnosticForLLM>) -> Self {
        self.rule_diagnostics = diags;
        self
    }

    pub fn key_metrics(mut self, metrics: KeyMetricsForLLM) -> Self {
        self.key_metrics = metrics;
        self
    }

    pub fn user_question(mut self, question: impl Into<String>) -> Self {
        self.user_question = Some(question.into());
        self
    }

    pub fn build(self) -> Result<RootCauseAnalysisRequest, &'static str> {
        Ok(RootCauseAnalysisRequest {
            query_summary: self.query_summary.ok_or("query_summary is required")?,
            profile_data: self.profile_data,
            execution_plan: self.execution_plan.ok_or("execution_plan is required")?,
            rule_diagnostics: self.rule_diagnostics,
            key_metrics: self.key_metrics,
            user_question: self.user_question,
        })
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Determine table type based on CATALOG prefix, not scan operator type!
///
/// StarRocks has two deployment modes:
/// 1. Shared-Nothing (存算一体): internal tables use OLAP_SCAN
/// 2. Shared-Data (存算分离): internal tables use CONNECTOR_SCAN
///
/// Both modes can access external tables (Hive/Iceberg/ES etc.) via catalogs.
///
/// ## The ONLY reliable rule:
/// - `default_catalog` → internal (StarRocks native table)
/// - Any other catalog name → external (foreign table)
///
/// # Arguments
/// * `table_name` - Full table name, may be "catalog.database.table" or "database.table" or just "table"
///
/// # Returns
/// * "internal" - StarRocks native table (in default_catalog)
/// * "external" - External table (any non-default catalog)
pub fn determine_table_type(table_name: &str) -> String {
    match table_name.split('.').collect::<Vec<_>>() {
        parts if parts.len() >= 3 => {
            if parts[0].eq_ignore_ascii_case("default_catalog") {
                "internal".to_string()
            } else {
                "external".to_string()
            }
        },
        parts if parts.len() == 2 => "internal".to_string(),
        _ => "internal".to_string(),
    }
}

/// Determine external table connector type from Profile metrics
///
/// StarRocks Profile 中各类外表的标识 (from be/src/exec/hdfs_scanner):
/// - **Iceberg**: Has "IcebergV2FormatTimer" section under ORC/Parquet
/// - **Hive**: Has "ORC" or "Parquet" section, but NO Iceberg indicators
/// - **Delta Lake**: Has "DeletionVector" section (Delta uses deletion vectors)
/// - **Hudi**: Has Hudi-specific metrics
/// - **Paimon**: Has Paimon-specific metrics (uses deletion vector too)
/// - **JDBC**: Has JDBC-related metrics
/// - **ES/Elasticsearch**: Has ES-specific metrics
///
/// # Arguments
/// * `metrics` - The unique_metrics map from SCAN node
///
/// # Returns
/// * "iceberg", "hive", "hudi", "paimon", "deltalake", "jdbc", "es", or "unknown"
pub fn determine_connector_type(metrics: &std::collections::HashMap<String, String>) -> String {
    let keys_str = metrics
        .keys()
        .map(|k| k.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    let has = |p: &str| keys_str.contains(p);
    match () {
        _ if has("iceberg") || has("deletefilebuild") => "iceberg",
        _ if has("deletionvector") => "deltalake",
        _ if has("hudi") => "hudi",
        _ if has("paimon") => "paimon",
        _ if has("jdbc") => "jdbc",
        _ if has("elasticsearch") || has("_es_") => "es",
        _ if ["orc", "parquet", "stripe", "rowgroup"]
            .iter()
            .any(|p| has(p)) =>
        {
            "hive"
        },
        _ => "unknown",
    }
    .to_string()
}
