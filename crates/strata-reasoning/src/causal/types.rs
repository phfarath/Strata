use serde::{Deserialize, Serialize};

/// Categorization of entities represented in the Causal Graph.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CausalNodeKind {
    File,
    Module,
    Struct,
    Enum,
    Trait,
    Function,
    Endpoint,
    DatabaseTable,
    ConfigOption,
    ContractInvariant,
}

impl std::fmt::Display for CausalNodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CausalNodeKind::File => write!(f, "file"),
            CausalNodeKind::Module => write!(f, "module"),
            CausalNodeKind::Struct => write!(f, "struct"),
            CausalNodeKind::Enum => write!(f, "enum"),
            CausalNodeKind::Trait => write!(f, "trait"),
            CausalNodeKind::Function => write!(f, "function"),
            CausalNodeKind::Endpoint => write!(f, "endpoint"),
            CausalNodeKind::DatabaseTable => write!(f, "database_table"),
            CausalNodeKind::ConfigOption => write!(f, "config_option"),
            CausalNodeKind::ContractInvariant => write!(f, "contract_invariant"),
        }
    }
}

/// Categorization of causal dependencies and coupling relationships.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CausalEdgeKind {
    Imports,
    Calls,
    Implements,
    Extends,
    ReadsFrom,
    WritesTo,
    ExposesEndpoint,
    EnforcesContract,
    DependsOn,
}

impl std::fmt::Display for CausalEdgeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CausalEdgeKind::Imports => write!(f, "imports"),
            CausalEdgeKind::Calls => write!(f, "calls"),
            CausalEdgeKind::Implements => write!(f, "implements"),
            CausalEdgeKind::Extends => write!(f, "extends"),
            CausalEdgeKind::ReadsFrom => write!(f, "reads_from"),
            CausalEdgeKind::WritesTo => write!(f, "writes_to"),
            CausalEdgeKind::ExposesEndpoint => write!(f, "exposes_endpoint"),
            CausalEdgeKind::EnforcesContract => write!(f, "enforces_contract"),
            CausalEdgeKind::DependsOn => write!(f, "depends_on"),
        }
    }
}

/// A node in the architecture's causal topology.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CausalNode {
    pub id: String,
    pub name: String,
    pub kind: CausalNodeKind,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub metadata: serde_json::Value,
    #[serde(default = "default_confidence")]
    pub confidence: f32,
}

fn default_confidence() -> f32 {
    1.0
}

impl CausalNode {
    pub fn new(id: impl Into<String>, name: impl Into<String>, kind: CausalNodeKind) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind,
            path: None,
            metadata: serde_json::json!({}),
            confidence: 1.0,
        }
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn with_confidence(mut self, confidence: f32) -> Self {
        self.confidence = confidence.clamp(0.0, 1.0);
        self
    }
}

/// A directed causal dependency edge with semantic coupling weight.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CausalEdge {
    pub kind: CausalEdgeKind,
    pub weight: f32, // [0.0, 1.0] coupling strength
    pub is_breaking_if_changed: bool,
    #[serde(default)]
    pub description: Option<String>,
}

impl CausalEdge {
    pub fn new(kind: CausalEdgeKind, weight: f32, is_breaking_if_changed: bool) -> Self {
        Self {
            kind,
            weight: weight.clamp(0.0, 1.0),
            is_breaking_if_changed,
            description: None,
        }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn imports(weight: f32) -> Self {
        Self::new(CausalEdgeKind::Imports, weight, false)
    }

    pub fn calls(weight: f32, is_breaking: bool) -> Self {
        Self::new(CausalEdgeKind::Calls, weight, is_breaking)
    }

    pub fn enforces_contract(weight: f32) -> Self {
        Self::new(CausalEdgeKind::EnforcesContract, weight, true)
    }

    pub fn writes_to(weight: f32) -> Self {
        Self::new(CausalEdgeKind::WritesTo, weight, true)
    }

    pub fn exposes_endpoint(weight: f32) -> Self {
        Self::new(CausalEdgeKind::ExposesEndpoint, weight, true)
    }
}

/// An entity impacted by a proposed change along a causal path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImpactedNode {
    pub node_id: String,
    pub name: String,
    pub kind: CausalNodeKind,
    pub path: Option<String>,
    pub distance: usize,
    pub cumulative_weight: f32,
    pub is_breaking_risk: bool,
    pub causal_path: Vec<String>,
    pub edge_kinds: Vec<CausalEdgeKind>,
}

/// Comprehensive blast radius evaluation report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BlastRadiusReport {
    pub target_id: String,
    pub target_name: String,
    pub max_depth: usize,
    pub total_nodes_scanned: usize,
    pub direct_impacts: Vec<ImpactedNode>,
    pub transitive_impacts: Vec<ImpactedNode>,
    pub triggered_anti_patterns: Vec<String>,
    pub triggered_invariants: Vec<String>,
    pub overall_risk_score: f32, // [0.0, 1.0]
    pub recommendations: Vec<String>,
}

/// Result of pre-flight simulation for a proposed patch/change set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PatchSimulationResult {
    pub modified_targets: Vec<String>,
    pub total_impacted_nodes: usize,
    pub highest_risk_score: f32,
    pub breaking_risks_count: usize,
    pub triggered_anti_patterns: Vec<String>,
    pub safe_to_apply: bool,
    pub blast_reports: Vec<BlastRadiusReport>,
}
