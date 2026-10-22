//! Root Cause Analysis Engine (v5.0)
//!
//! Implements multi-dimensional causal analysis without LLM:
//! 1. Intra-Node Causality - same node, multiple diagnostics
//! 2. Inter-Node Propagation - DAG-based propagation
//! 3. Multiple Root Causes - identify all independent root causes
//! 4. Causal Graph - build and visualize causal relationships

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use super::rules::Diagnostic;

// ============================================================================
// Root Cause Analysis Result Types
// ============================================================================

/// Complete root cause analysis result
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RootCauseAnalysis {
    /// Identified root causes (sorted by impact)
    pub root_causes: Vec<RootCause>,
    /// Causal chains explaining how root causes lead to symptoms
    pub causal_chains: Vec<CausalChain>,
    /// Natural language summary
    pub summary: String,
    /// Total number of diagnostics analyzed
    pub total_diagnostics: usize,
}

/// A single root cause
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootCause {
    /// Unique ID (e.g., "RC001")
    pub id: String,
    /// Related diagnostic rule IDs
    pub diagnostic_ids: Vec<String>,
    /// Description of the root cause
    pub description: String,
    /// Impact percentage (0-100)
    pub impact_percentage: f64,
    /// Confidence score (0.0-1.0)
    pub confidence: f64,
    /// Node paths affected
    pub affected_nodes: Vec<String>,
    /// Evidence supporting this conclusion
    pub evidence: Vec<String>,
    /// Symptoms caused by this root cause
    pub symptoms: Vec<String>,
    /// Suggested actions
    pub suggestions: Vec<String>,
}

/// A causal chain from root cause to symptom
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalChain {
    /// Chain elements: ["Root Cause", "→", "Intermediate", "→", "Symptom"]
    pub chain: Vec<String>,
    /// Explanation of this causal relationship
    pub explanation: String,
    /// Confidence score
    pub confidence: f64,
}

// ============================================================================
// Intra-Node Causality Rules
// ============================================================================

/// Rule defining causality within the same node
struct IntraNodeRule {
    /// Cause diagnostic IDs (any of these)
    causes: &'static [&'static str],
    /// Effect diagnostic ID
    effect: &'static str,
    /// Description of the causal relationship
    description: &'static str,
}

/// Predefined intra-node causality rules based on v5.0 design
///
/// Rule Categories:
/// - S: SCAN node rules (S001-S016)
/// - J: JOIN node rules (J001-J011)
/// - A: AGGREGATE node rules (A001-A006)
/// - Q: Query-level rules (Q001-Q009)
/// - G: General rules (G001-G003)
/// - E: EXCHANGE node rules (E001-E003)
/// - T: SORT node rules (T001-T005)
/// - W: WINDOW node rules (W001)
/// - I: SINK/Insert rules (I001-I003)
/// - F: Fragment rules (F001-F003)
const INTRA_NODE_RULES: &[IntraNodeRule] = &[
    IntraNodeRule {
        causes: &["S016", "S006"], // Small files (S016), Rowset fragmentation (S006)
        effect: "S007",            // IO bottleneck
        description: "Small files/fragmentation cause an IO bottleneck",
    },
    IntraNodeRule {
        causes: &["S017"], // ORC Stripe fragmentation
        effect: "S007",    // IO bottleneck
        description: "Stripe fragmentation causes an IO bottleneck",
    },
    IntraNodeRule {
        causes: &["S017"], // ORC Stripe fragmentation
        effect: "S018",    // IO wait time
        description: "Stripe fragmentation causes a long IO wait time",
    },
    IntraNodeRule {
        causes: &["S017"], // ORC Stripe fragmentation
        effect: "G001",    // Time consuming node
        description: "Stripe fragmentation makes the SCAN slow",
    },
    IntraNodeRule {
        causes: &["S018"], // IO wait time
        effect: "G001",    // Time consuming node
        description: "A long IO wait time makes the SCAN slow",
    },
    IntraNodeRule {
        causes: &["S018"], // IO wait time
        effect: "Q004",    // Low CPU utilization
        description: "The IO wait leaves the CPU idle",
    },
    IntraNodeRule {
        causes: &["S009"], // Low cache hit ratio
        effect: "S007",    // IO bottleneck
        description: "A low cache hit rate causes an IO bottleneck",
    },
    IntraNodeRule {
        causes: &["S009"], // Low cache hit ratio
        effect: "G001",    // Time consuming node (SCAN)
        description: "A low cache hit rate makes the SCAN slow",
    },
    IntraNodeRule {
        causes: &["S008", "S012", "S013"], // ZoneMap/Bitmap/BloomFilter ineffective
        effect: "S003",                    // Poor filter effectiveness
        description: "The ineffective index causes poor filter effectiveness",
    },
    IntraNodeRule {
        causes: &["S001"], // Data skew in SCAN
        effect: "G003",    // Execution time skew
        description: "The data skew causes the execution time skew",
    },
    IntraNodeRule {
        causes: &["S003"], // Poor filter effectiveness
        effect: "S002",    // Full table scan
        description: "The poor filter effectiveness causes a full table scan",
    },
    IntraNodeRule {
        causes: &["S010"], // Large compressed ratio
        effect: "S007",    // IO bottleneck (decompression overhead)
        description: "The high compression ratio makes the decompression expensive",
    },
    IntraNodeRule {
        causes: &["S014"], // Too many segments
        effect: "S007",    // IO bottleneck
        description: "Too many Segments cause an IO bottleneck",
    },
    IntraNodeRule {
        causes: &["J002"], // Suboptimal join order
        effect: "J001",    // Hash table too large
        description: "A suboptimal Join order makes the hash table too large",
    },
    IntraNodeRule {
        causes: &["J005"], // Broadcast table too large
        effect: "E002",    // Network bottleneck
        description: "A too large Broadcast table causes a network bottleneck",
    },
    IntraNodeRule {
        causes: &["J003"], // Join probe rows skew
        effect: "G003",    // Execution time skew
        description: "The data skew on the Join probe side causes the time skew",
    },
    IntraNodeRule {
        causes: &["J001"], // Hash table too large
        effect: "Q003",    // Spill to disk
        description: "The too large hash table spills the memory to disk",
    },
    IntraNodeRule {
        causes: &["J006"], // Missing runtime filter
        effect: "S003",    // Poor filter effectiveness on probe side
        description: "The missing Runtime Filter makes the probe side filter poorly",
    },
    IntraNodeRule {
        causes: &["J007"], // Runtime filter not pushed down
        effect: "S003",    // Poor filter effectiveness
        description: "The Runtime Filter that is not pushed down causes poor filter effectiveness",
    },
    IntraNodeRule {
        causes: &["J008"], // Join condition not optimal
        effect: "J001",    // Large hash table
        description: "A suboptimal Join condition makes the hash table too large",
    },
    IntraNodeRule {
        causes: &["J009"], // Cross join detected
        effect: "G002",    // High CPU utilization
        description: "The cartesian product raises the CPU usage",
    },
    IntraNodeRule {
        causes: &["J010"], // Join type not optimal
        effect: "E002",    // Network bottleneck (wrong distribution)
        description: "A suboptimal Join type causes too much data transfer",
    },
    IntraNodeRule {
        causes: &["A001"], // Aggregation skew
        effect: "Q003",    // Spill occurred
        description: "The aggregation skew makes the memory overflow",
    },
    IntraNodeRule {
        causes: &["A001"], // Aggregation skew
        effect: "G003",    // Execution time skew
        description: "The aggregation skew causes the execution time skew",
    },
    IntraNodeRule {
        causes: &["A003"], // Too many distinct keys
        effect: "A002",    // Large hash table
        description: "Too many distinct keys make the hash table too large",
    },
    IntraNodeRule {
        causes: &["A002"], // Large hash table
        effect: "Q003",    // Spill to disk
        description: "The too large aggregation hash table causes a spill",
    },
    IntraNodeRule {
        causes: &["A004"], // Missing streaming aggregation
        effect: "A002",    // Large hash table
        description: "Not using a streaming aggregation makes the memory usage high",
    },
    IntraNodeRule {
        causes: &["A005"], // High aggregation cardinality
        effect: "G002",    // High CPU utilization
        description: "The too high aggregation cardinality raises the CPU usage",
    },
    IntraNodeRule {
        causes: &["T001"], // Sort data too large
        effect: "Q003",    // Spill to disk
        description: "The too large sorted data volume causes a spill",
    },
    IntraNodeRule {
        causes: &["T002"], // TopN not optimized
        effect: "T001",    // Large sort data
        description: "The TopN is not optimized, which makes the sorted data volume large",
    },
    IntraNodeRule {
        causes: &["T003"], // Sort without limit
        effect: "T001",    // Large sort data
        description: "The full sort makes the data volume too large",
    },
    IntraNodeRule {
        causes: &["T004"], // Multiple sort keys
        effect: "G002",    // High CPU utilization
        description: "Multiple sort keys raise the CPU usage",
    },
    IntraNodeRule {
        causes: &["E001"], // Large data shuffle
        effect: "E002",    // Network bottleneck
        description: "The large Shuffle volume causes a network bottleneck",
    },
    IntraNodeRule {
        causes: &["E003"], // Partition skew
        effect: "G003",    // Execution time skew
        description: "The partition skew causes the execution time skew",
    },
    IntraNodeRule {
        causes: &["Q003"], // Spill to disk
        effect: "Q001",    // Query timeout
        description: "The disk spill slows the query down and may time it out",
    },
    IntraNodeRule {
        causes: &["Q006"], // Resource queue waiting
        effect: "Q001",    // Query timeout
        description: "The resource queue wait may cause a timeout",
    },
    IntraNodeRule {
        causes: &["S007"], // IO bottleneck
        effect: "Q004",    // Low CPU utilization
        description: "The IO bottleneck lowers the CPU utilization",
    },
    IntraNodeRule {
        causes: &["S009"], // Low cache hit
        effect: "Q004",    // Low CPU (waiting for remote IO)
        description: "The cache misses cause a wait for the remote IO and leave the CPU idle",
    },
    IntraNodeRule {
        causes: &["E002"], // Network bottleneck
        effect: "Q004",    // Low CPU (waiting for network)
        description: "The network wait lowers the CPU utilization",
    },
    IntraNodeRule {
        causes: &["G001", "G001b"], // Most/Second consuming node (usually SCAN)
        effect: "Q005",             // Scan time ratio high
        description: "The slow scan operator makes the scan time share high",
    },
    IntraNodeRule {
        causes: &["G002"], // High memory usage node
        effect: "Q002",    // Query peak memory high
        description: "The high operator memory makes the query peak memory high",
    },
    IntraNodeRule {
        causes: &["G001"], // Most consuming node
        effect: "Q001",    // Query timeout
        description: "The slow operator makes the query time out",
    },
    IntraNodeRule {
        causes: &["J003"], // Join HashTable memory high
        effect: "G002",    // Node memory high
        description: "The high Join HashTable memory makes the node memory high",
    },
    IntraNodeRule {
        causes: &["A002"], // Aggregation HashTable memory high
        effect: "G002",    // Node memory high
        description: "The high aggregation HashTable memory makes the node memory high",
    },
    IntraNodeRule {
        causes: &["J005"], // Hash collision
        effect: "J003",    // HashTable memory high
        description: "The hash collisions grow the HashTable memory",
    },
    IntraNodeRule {
        causes: &["J011"], // Broadcast build side too large
        effect: "J003",    // HashTable memory high
        description: "The large Broadcast side makes the HashTable memory high",
    },
    IntraNodeRule {
        causes: &["J011"], // Broadcast build side too large
        effect: "G002",    // Node memory high
        description: "The large Broadcast side makes the node memory high",
    },
    IntraNodeRule {
        causes: &["E001"], // Large shuffle data
        effect: "Q008",    // Scheduling overhead high
        description: "The large Shuffle volume increases the scheduling overhead",
    },
    IntraNodeRule {
        causes: &["Q008"], // Scheduling overhead
        effect: "Q001",    // Query timeout
        description: "The large scheduling overhead slows the query down",
    },
    IntraNodeRule {
        causes: &["P001"], // Complex expression in project
        effect: "G002",    // High CPU utilization
        description: "The complex expression compute raises the CPU usage",
    },
    IntraNodeRule {
        causes: &["E002"], // Network bottleneck
        effect: "I001",    // Insert slow
        description: "The network bottleneck slows the data load down",
    },
    IntraNodeRule {
        causes: &["A001"], // Aggregation skew
        effect: "I002",    // Insert skew
        description: "The aggregation skew makes the loaded data skewed",
    },
];

// ============================================================================
// Inter-Node Propagation Rules
// ============================================================================

/// Propagation mode between nodes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PropagationMode {
    /// Data volume propagates: upstream data → downstream processing time
    DataVolume,
    /// Skew propagates: upstream skew → downstream skew
    Skew,
    /// Memory pressure propagates: sibling memory usage → spill
    Memory,
    /// IO wait propagates: upstream IO → downstream stall
    IoWait,
}

/// Rule for inter-node propagation
struct InterNodeRule {
    /// Upstream diagnostic pattern
    upstream: &'static str,
    /// Downstream diagnostic pattern
    downstream: &'static str,
    /// Propagation mode (reserved for future use)
    #[allow(dead_code)]
    mode: PropagationMode,
    /// Description
    description: &'static str,
}

/// Inter-node propagation rules - how issues propagate along the DAG
///
/// Propagation Modes:
/// - DataVolume: Data size propagates downstream
/// - Skew: Data skew propagates downstream
/// - Memory: Memory pressure causes spill
/// - IoWait: IO wait propagates to downstream stall
const INTER_NODE_RULES: &[InterNodeRule] = &[
    InterNodeRule {
        upstream: "S001",   // SCAN data skew
        downstream: "G003", // Execution time skew
        mode: PropagationMode::Skew,
        description: "The SCAN data skew propagates to the downstream execution time skew",
    },
    InterNodeRule {
        upstream: "S001",   // SCAN data skew
        downstream: "J003", // Join probe rows skew
        mode: PropagationMode::Skew,
        description: "The SCAN data skew propagates to the Join probe side",
    },
    InterNodeRule {
        upstream: "S001",   // SCAN data skew
        downstream: "A001", // Aggregation skew
        mode: PropagationMode::Skew,
        description: "The SCAN data skew propagates to the aggregation skew",
    },
    InterNodeRule {
        upstream: "S001",   // SCAN data skew
        downstream: "E003", // Exchange partition skew
        mode: PropagationMode::Skew,
        description: "The SCAN data skew propagates to the Shuffle partition skew",
    },
    InterNodeRule {
        upstream: "J003",   // Join probe skew
        downstream: "A001", // Aggregation skew
        mode: PropagationMode::Skew,
        description: "The Join skew propagates to the aggregation skew",
    },
    InterNodeRule {
        upstream: "E003",   // Exchange partition skew
        downstream: "G003", // Execution time skew
        mode: PropagationMode::Skew,
        description: "The Shuffle skew propagates to the execution time skew",
    },
    InterNodeRule {
        upstream: "S003",   // Poor filter effectiveness
        downstream: "J001", // Hash table too large
        mode: PropagationMode::DataVolume,
        description: "The poor filter effectiveness makes the downstream Join data volume large",
    },
    InterNodeRule {
        upstream: "S003",   // Poor filter effectiveness
        downstream: "A002", // Aggregation hash table large
        mode: PropagationMode::DataVolume,
        description: "The poor filter effectiveness makes the aggregation process a large data volume",
    },
    InterNodeRule {
        upstream: "S003",   // Poor filter effectiveness
        downstream: "E001", // Large shuffle data
        mode: PropagationMode::DataVolume,
        description: "The poor filter effectiveness makes the Shuffle data volume large",
    },
    InterNodeRule {
        upstream: "S003",   // Poor filter effectiveness
        downstream: "T001", // Sort data too large
        mode: PropagationMode::DataVolume,
        description: "The poor filter effectiveness makes the sorted data volume large",
    },
    InterNodeRule {
        upstream: "S002",   // Full table scan
        downstream: "J001", // Hash table too large
        mode: PropagationMode::DataVolume,
        description: "The full table scan makes the Join data volume large",
    },
    InterNodeRule {
        upstream: "S002",   // Full table scan
        downstream: "E001", // Large shuffle data
        mode: PropagationMode::DataVolume,
        description: "The full table scan makes the Shuffle data volume large",
    },
    InterNodeRule {
        upstream: "J001",   // Large join hash table
        downstream: "A002", // Large aggregation hash table
        mode: PropagationMode::DataVolume,
        description: "The large Join output makes the aggregation data volume large",
    },
    InterNodeRule {
        upstream: "J009",   // Cross join (cartesian product)
        downstream: "A002", // Large aggregation hash table
        mode: PropagationMode::DataVolume,
        description: "The cartesian product blows up the downstream data volume",
    },
    InterNodeRule {
        upstream: "J009",   // Cross join
        downstream: "T001", // Large sort data
        mode: PropagationMode::DataVolume,
        description: "The cartesian product blows up the sorted data volume",
    },
    InterNodeRule {
        upstream: "J001",   // Hash table large
        downstream: "Q003", // Spill
        mode: PropagationMode::Memory,
        description: "The high Join memory usage triggers a spill",
    },
    InterNodeRule {
        upstream: "A002",   // Aggregation hash table large
        downstream: "Q003", // Spill
        mode: PropagationMode::Memory,
        description: "The high aggregation memory usage triggers a spill",
    },
    InterNodeRule {
        upstream: "T001",   // Large sort data
        downstream: "Q003", // Spill
        mode: PropagationMode::Memory,
        description: "The large sorted data volume triggers a spill",
    },
    InterNodeRule {
        upstream: "W001",   // Window function memory
        downstream: "Q003", // Spill
        mode: PropagationMode::Memory,
        description: "The window function memory usage causes a spill",
    },
    InterNodeRule {
        upstream: "S007",   // IO bottleneck (SCAN level)
        downstream: "G001", // Time consuming node (if same SCAN node)
        mode: PropagationMode::IoWait,
        description: "The IO bottleneck makes the SCAN node slow",
    },
    InterNodeRule {
        upstream: "S017",   // ORC Stripe fragmentation
        downstream: "G001", // Time consuming node
        mode: PropagationMode::IoWait,
        description: "The Stripe fragmentation makes the SCAN node slow",
    },
    InterNodeRule {
        upstream: "S017",   // Stripe fragmentation
        downstream: "S018", // IO wait time
        mode: PropagationMode::IoWait,
        description: "The Stripe fragmentation causes the IO wait",
    },
    InterNodeRule {
        upstream: "S018",   // IO wait time
        downstream: "G001", // Time consuming node
        mode: PropagationMode::IoWait,
        description: "The IO wait makes the node slow",
    },
    InterNodeRule {
        upstream: "S018",   // IO wait time
        downstream: "Q004", // Low CPU
        mode: PropagationMode::IoWait,
        description: "The IO wait lowers the CPU utilization",
    },
    InterNodeRule {
        upstream: "Q003",   // Spill to disk
        downstream: "G001", // Time consuming node
        mode: PropagationMode::IoWait,
        description: "The disk spill makes the node slow",
    },
    InterNodeRule {
        upstream: "G001",   // Most consuming node (SCAN)
        downstream: "Q005", // Scan time ratio high (Query level)
        mode: PropagationMode::DataVolume,
        description: "The slow SCAN makes the scan time share high",
    },
    InterNodeRule {
        upstream: "G001b",  // Second consuming node
        downstream: "Q005", // Scan time ratio high
        mode: PropagationMode::DataVolume,
        description: "The second slowest SCAN makes the scan time share high",
    },
    InterNodeRule {
        upstream: "G002",   // High memory node
        downstream: "Q002", // Query peak memory high
        mode: PropagationMode::Memory,
        description: "The high node memory makes the query peak memory high",
    },
    InterNodeRule {
        upstream: "G001",   // Most consuming node
        downstream: "Q001", // Query timeout
        mode: PropagationMode::IoWait,
        description: "The slow operator makes the query time out",
    },
    InterNodeRule {
        upstream: "J003",   // Join HashTable memory
        downstream: "Q002", // Query peak memory
        mode: PropagationMode::Memory,
        description: "The high Join memory makes the query peak memory high",
    },
    InterNodeRule {
        upstream: "A002",   // Aggregation memory
        downstream: "Q002", // Query peak memory
        mode: PropagationMode::Memory,
        description: "The high aggregation memory makes the query peak memory high",
    },
    InterNodeRule {
        upstream: "E001",   // Large shuffle
        downstream: "Q008", // Scheduling overhead
        mode: PropagationMode::DataVolume,
        description: "The large Shuffle volume increases the scheduling overhead",
    },
    InterNodeRule {
        upstream: "Q008",   // Scheduling overhead
        downstream: "Q001", // Query timeout
        mode: PropagationMode::IoWait,
        description: "The large scheduling overhead makes the query time out",
    },
    InterNodeRule {
        upstream: "J011",   // Broadcast too large
        downstream: "G002", // High memory node
        mode: PropagationMode::Memory,
        description: "The large Broadcast data makes the node memory high",
    },
    InterNodeRule {
        upstream: "J005",   // Hash collision
        downstream: "G002", // High memory node
        mode: PropagationMode::Memory,
        description: "The hash collisions grow the node memory",
    },
    InterNodeRule {
        upstream: "Q002",   // Query peak memory high
        downstream: "Q001", // Query timeout
        mode: PropagationMode::Memory,
        description: "The excessive memory pressure may make the query time out",
    },
    InterNodeRule {
        upstream: "A002",   // Aggregation HashTable large
        downstream: "G001", // Most consuming node (AGG)
        mode: PropagationMode::Memory,
        description: "The too large aggregation HashTable makes the aggregate operator slow",
    },
    InterNodeRule {
        upstream: "J003",   // Join HashTable memory
        downstream: "G001", // Most consuming node (JOIN)
        mode: PropagationMode::Memory,
        description: "The too large Join HashTable makes the Join operator slow",
    },
    InterNodeRule {
        upstream: "J001",   // Join hash table too large
        downstream: "G001", // Most consuming node
        mode: PropagationMode::Memory,
        description: "The large Join data volume makes the Join operator slow",
    },
    InterNodeRule {
        upstream: "E001",   // Large shuffle data
        downstream: "G001", // Most consuming node (EXCHANGE)
        mode: PropagationMode::DataVolume,
        description: "The large Shuffle volume makes the EXCHANGE operator slow",
    },
    InterNodeRule {
        upstream: "E001",   // Large shuffle (cause)
        downstream: "E002", // Network ratio high (effect)
        mode: PropagationMode::IoWait,
        description: "The large Shuffle makes the network time share high",
    },
    InterNodeRule {
        upstream: "S009",   // Low cache hit
        downstream: "Q004", // Low CPU utilization
        mode: PropagationMode::IoWait,
        description: "The cache misses cause a wait for the remote IO and leave the CPU idle",
    },
    InterNodeRule {
        upstream: "S007",   // IO bottleneck
        downstream: "Q004", // Low CPU utilization
        mode: PropagationMode::IoWait,
        description: "The IO bottleneck lowers the CPU utilization",
    },
    InterNodeRule {
        upstream: "E002",   // Network time ratio high
        downstream: "Q004", // Low CPU (waiting for network)
        mode: PropagationMode::IoWait,
        description: "The network wait lowers the CPU utilization",
    },
];

// ============================================================================
// Root Cause Analyzer
// ============================================================================

/// Root cause analyzer implementing v5.0 design
pub struct RootCauseAnalyzer;

impl RootCauseAnalyzer {
    /// Analyze diagnostics and identify root causes
    pub fn analyze(diagnostics: &[Diagnostic]) -> RootCauseAnalysis {
        if diagnostics.is_empty() {
            return RootCauseAnalysis::default();
        }

        let diag_map = Self::build_diagnostic_map(diagnostics);

        let intra_edges = Self::find_intra_node_causality(diagnostics);

        let inter_edges = Self::find_inter_node_propagation(diagnostics);

        let all_edges: Vec<(String, String, String)> =
            intra_edges.into_iter().chain(inter_edges).collect();

        let root_causes = Self::identify_root_causes(diagnostics, &all_edges, &diag_map);

        let causal_chains = Self::build_causal_chains(&root_causes, &all_edges, &diag_map);

        let summary = Self::generate_summary(&root_causes);

        RootCauseAnalysis {
            root_causes,
            causal_chains,
            summary,
            total_diagnostics: diagnostics.len(),
        }
    }

    /// Build a map of rule_id -> diagnostics for quick lookup
    fn build_diagnostic_map(diagnostics: &[Diagnostic]) -> HashMap<String, Vec<&Diagnostic>> {
        let mut map: HashMap<String, Vec<&Diagnostic>> = HashMap::new();
        for diag in diagnostics {
            map.entry(diag.rule_id.clone()).or_default().push(diag);
        }
        map
    }

    /// Find intra-node causal relationships
    fn find_intra_node_causality(diagnostics: &[Diagnostic]) -> Vec<(String, String, String)> {
        let mut edges = Vec::new();

        let mut by_node: HashMap<&str, Vec<&Diagnostic>> = HashMap::new();
        for diag in diagnostics {
            by_node.entry(&diag.node_path).or_default().push(diag);
        }

        for (_node_path, node_diags) in by_node {
            let rule_ids: HashSet<&str> = node_diags.iter().map(|d| d.rule_id.as_str()).collect();

            for rule in INTRA_NODE_RULES {
                let has_cause = rule.causes.iter().any(|c| rule_ids.contains(*c));

                let has_effect = rule_ids.contains(rule.effect);

                if has_cause && has_effect {
                    for cause in rule.causes {
                        if rule_ids.contains(*cause) {
                            edges.push((
                                cause.to_string(),
                                rule.effect.to_string(),
                                rule.description.to_string(),
                            ));
                        }
                    }
                }
            }
        }

        edges
    }

    /// Find inter-node causal relationships based on DAG
    fn find_inter_node_propagation(diagnostics: &[Diagnostic]) -> Vec<(String, String, String)> {
        let mut edges = Vec::new();
        let rule_ids: HashSet<&str> = diagnostics.iter().map(|d| d.rule_id.as_str()).collect();

        for rule in INTER_NODE_RULES {
            if rule_ids.contains(rule.upstream) && rule_ids.contains(rule.downstream) {
                edges.push((
                    rule.upstream.to_string(),
                    rule.downstream.to_string(),
                    rule.description.to_string(),
                ));
            }
        }

        edges
    }

    /// Identify root causes (diagnostics with no incoming causal edges)
    fn identify_root_causes(
        diagnostics: &[Diagnostic],
        edges: &[(String, String, String)],
        diag_map: &HashMap<String, Vec<&Diagnostic>>,
    ) -> Vec<RootCause> {
        let effects: HashSet<&str> = edges.iter().map(|(_, effect, _)| effect.as_str()).collect();

        let _causes: HashSet<&str> = edges.iter().map(|(cause, _, _)| cause.as_str()).collect();

        let mut root_cause_ids: HashSet<&str> = HashSet::new();

        let all_ids: HashSet<&str> = diagnostics.iter().map(|d| d.rule_id.as_str()).collect();

        for id in &all_ids {
            if !effects.contains(id) {
                root_cause_ids.insert(id);
            }
        }

        if root_cause_ids.is_empty() {
            for diag in diagnostics.iter().take(3) {
                root_cause_ids.insert(&diag.rule_id);
            }
        }

        let mut root_causes: Vec<RootCause> = Vec::new();
        let mut rc_counter = 0;

        for rule_id in root_cause_ids {
            if let Some(diags) = diag_map.get(rule_id) {
                rc_counter += 1;
                let first_diag = diags[0];

                let symptoms: Vec<String> = edges
                    .iter()
                    .filter(|(cause, _, _)| cause == rule_id)
                    .map(|(_, effect, _)| effect.clone())
                    .collect();

                let base_impact = match first_diag.severity {
                    super::rules::RuleSeverity::Error => 40.0,
                    super::rules::RuleSeverity::Warning => 25.0,
                    super::rules::RuleSeverity::Info => 15.0,
                };
                let symptom_bonus = symptoms.len() as f64 * 10.0;
                let impact = (base_impact + symptom_bonus).min(100.0);

                let suggestions = Self::merge_suggestions_for_root_cause(diags);

                root_causes.push(RootCause {
                    id: format!("RC{:03}", rc_counter),
                    diagnostic_ids: vec![rule_id.to_string()],
                    description: first_diag.message.clone(),
                    impact_percentage: impact,
                    confidence: 1.0,
                    affected_nodes: diags.iter().map(|d| d.node_path.clone()).collect(),
                    evidence: vec![first_diag.reason.clone()],
                    symptoms,
                    suggestions,
                });
            }
        }

        root_causes.sort_by(|a, b| {
            b.impact_percentage
                .partial_cmp(&a.impact_percentage)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        root_causes
    }

    /// Merge suggestions from multiple diagnostics of the same rule
    /// Consolidates similar suggestions (e.g., different table names) into one
    fn merge_suggestions_for_root_cause(diags: &[&Diagnostic]) -> Vec<String> {
        if diags.len() == 1 {
            return diags[0].suggestions.clone();
        }

        let tables: Vec<String> = diags
            .iter()
            .filter_map(|d| Self::extract_table_name(&d.reason))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        let first_sug = diags[0]
            .suggestions
            .first()
            .map(|s| s.as_str())
            .unwrap_or("");

        if first_sug.contains("External table small file merge plan")
            || first_sug.contains("Run Compaction to merge fragments")
        {
            let tables_str = if tables.is_empty() {
                format!("{} tables", diags.len())
            } else if tables.len() <= 3 {
                tables.join(", ")
            } else {
                format!("{} and {} tables", tables[0], tables.len())
            };

            return if first_sug.contains("External table small file merge plan") {
                vec![format!(
                    "External table small file merge (involving: {}): ①ALTER TABLE <table> PARTITION(...) CONCATENATE; \
                     ②INSERT OVERWRITE TABLE <table> SELECT * FROM <table>; \
                     ③Spark: df.repartition(N).saveAsTable('<table>'); \
                     ④SET connector_io_tasks_per_scan_operator=64",
                    tables_str
                )]
            } else {
                vec![format!("Run Compaction: ALTER TABLE <{}> COMPACT", tables_str)]
            };
        }

        let mut seen = HashSet::new();
        diags
            .iter()
            .flat_map(|d| d.suggestions.iter())
            .filter(|s| seen.insert(s.as_str()))
            .take(3)
            .cloned()
            .collect()
    }

    /// Extract table name from reason like "外表「table_name」的 ORC..."
    fn extract_table_name(reason: &str) -> Option<String> {
        let start = reason.find('「')?;
        let end = reason.find('」')?;
        (end > start).then(|| reason[start + 3..end].to_string())
    }

    /// Build causal chains from root causes to symptoms
    /// Deduplicates by rule_name chain (not rule_id) to avoid visual duplicates
    fn build_causal_chains(
        root_causes: &[RootCause],
        edges: &[(String, String, String)],
        diag_map: &HashMap<String, Vec<&Diagnostic>>,
    ) -> Vec<CausalChain> {
        let mut chains = Vec::new();
        let mut seen_rule_id_chains: HashSet<String> = HashSet::new();
        let mut seen_name_chains: HashSet<String> = HashSet::new();

        for rc in root_causes {
            for diag_id in &rc.diagnostic_ids {
                let paths = Self::find_paths_from(diag_id, edges, 3);

                for path in paths {
                    if path.len() < 2 {
                        continue;
                    }

                    let id_key = path.join("->");
                    if seen_rule_id_chains.contains(&id_key) {
                        continue;
                    }
                    seen_rule_id_chains.insert(id_key);

                    let names: Vec<String> = path
                        .iter()
                        .map(|id| {
                            diag_map
                                .get(id)
                                .and_then(|d| d.first())
                                .map(|d| d.rule_name.clone())
                                .unwrap_or_else(|| id.clone())
                        })
                        .collect();

                    let name_key = names.join("->");
                    if seen_name_chains.contains(&name_key) {
                        continue;
                    }
                    seen_name_chains.insert(name_key);

                    let mut chain = Vec::new();
                    let mut explanations = Vec::new();

                    for (i, (node_id, name)) in path.iter().zip(names.iter()).enumerate() {
                        chain.push(name.clone());
                        if i < path.len() - 1 {
                            chain.push("→".to_string());
                            if let Some((_, _, desc)) = edges
                                .iter()
                                .find(|(c, e, _)| c == node_id && e == &path[i + 1])
                            {
                                explanations.push(desc.clone());
                            }
                        }
                    }

                    let explanation = if explanations.is_empty() {
                        format!(
                            "{} leads to {}",
                            names.first().unwrap_or(&path[0]),
                            names.last().unwrap_or(&path[0])
                        )
                    } else {
                        explanations.join("; ")
                    };

                    chains.push(CausalChain { chain, explanation, confidence: 1.0 });
                }
            }
        }

        chains.sort_by(|a, b| {
            a.chain
                .len()
                .cmp(&b.chain.len())
                .then_with(|| a.chain.join("").cmp(&b.chain.join("")))
        });

        chains.truncate(10);
        chains
    }

    /// Find all paths from a starting node using BFS
    fn find_paths_from(
        start: &str,
        edges: &[(String, String, String)],
        max_depth: usize,
    ) -> Vec<Vec<String>> {
        let mut paths = Vec::new();
        let mut queue: Vec<(Vec<String>, usize)> = vec![(vec![start.to_string()], 0)];

        while let Some((path, depth)) = queue.pop() {
            if depth >= max_depth {
                if path.len() > 1 {
                    paths.push(path);
                }
                continue;
            }

            let current = path.last().unwrap();
            let mut found_next = false;

            for (cause, effect, _) in edges {
                if cause == current && !path.contains(effect) {
                    found_next = true;
                    let mut new_path = path.clone();
                    new_path.push(effect.clone());
                    queue.push((new_path, depth + 1));
                }
            }

            if !found_next && path.len() > 1 {
                paths.push(path);
            }
        }

        paths
    }

    /// Generate a natural language summary
    fn generate_summary(root_causes: &[RootCause]) -> String {
        if root_causes.is_empty() {
            return "No obvious performance problem root cause was found".to_string();
        }

        if root_causes.len() == 1 {
            let rc = &root_causes[0];
            if rc.symptoms.is_empty() {
                format!("1 root cause found: {}", rc.description)
            } else {
                format!(
                    "1 root cause found: {}, which caused {} downstream problems",
                    rc.description,
                    rc.symptoms.len()
                )
            }
        } else {
            let top_causes: Vec<&str> = root_causes
                .iter()
                .take(3)
                .map(|rc| rc.diagnostic_ids.first().map(|s| s.as_str()).unwrap_or(""))
                .collect();

            format!(
                "{} independent root causes found, mainly: {}. Fix them in order of priority",
                root_causes.len(),
                top_causes.join(", ")
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::rules::RuleSeverity;
    use super::*;

    fn make_diag(rule_id: &str, node_path: &str, severity: RuleSeverity) -> Diagnostic {
        Diagnostic {
            rule_id: rule_id.to_string(),
            rule_name: format!("Rule {}", rule_id),
            severity,
            node_path: node_path.to_string(),
            plan_node_id: None,
            message: format!("Message for {}", rule_id),
            reason: format!("Reason for {}", rule_id),
            suggestions: vec![format!("Fix {}", rule_id)],
            parameter_suggestions: vec![],
            threshold_metadata: None,
        }
    }

    #[test]
    fn test_intra_node_causality() {
        let diagnostics = vec![
            make_diag("S016", "Fragment_1/Pipeline_0/SCAN", RuleSeverity::Warning),
            make_diag("S007", "Fragment_1/Pipeline_0/SCAN", RuleSeverity::Error),
        ];

        let result = RootCauseAnalyzer::analyze(&diagnostics);

        assert!(!result.root_causes.is_empty());
        assert!(
            result
                .root_causes
                .iter()
                .any(|rc| rc.diagnostic_ids.contains(&"S016".to_string()))
        );
        assert!(result.causal_chains.iter().any(|cc| {
            cc.chain
                .iter()
                .any(|s| s.contains("S016") || s.contains("small file"))
        }));
    }

    #[test]
    fn test_inter_node_propagation() {
        let diagnostics = vec![
            make_diag("S001", "Fragment_1/Pipeline_0/SCAN", RuleSeverity::Warning),
            make_diag("G003", "Fragment_1/Pipeline_1/JOIN", RuleSeverity::Warning),
        ];

        let result = RootCauseAnalyzer::analyze(&diagnostics);

        assert!(!result.root_causes.is_empty());
        assert!(
            result
                .root_causes
                .iter()
                .any(|rc| rc.diagnostic_ids.contains(&"S001".to_string()))
        );
    }

    #[test]
    fn test_multiple_root_causes() {
        let diagnostics = vec![
            make_diag("S016", "Fragment_1/Pipeline_0/SCAN_1", RuleSeverity::Warning),
            make_diag("J002", "Fragment_1/Pipeline_1/JOIN", RuleSeverity::Warning),
        ];

        let result = RootCauseAnalyzer::analyze(&diagnostics);

        assert_eq!(result.root_causes.len(), 2);
    }
}
