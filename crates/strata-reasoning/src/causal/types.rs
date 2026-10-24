use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    Assumption,
    Precondition,
    Postcondition,
    EnvironmentVariable,
    Action,
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
            CausalNodeKind::Assumption => write!(f, "assumption"),
            CausalNodeKind::Precondition => write!(f, "precondition"),
            CausalNodeKind::Postcondition => write!(f, "postcondition"),
            CausalNodeKind::EnvironmentVariable => write!(f, "environment_variable"),
            CausalNodeKind::Action => write!(f, "action"),
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
    CausallyPredicts,
    CounterfactualAlternative,
    InvalidatedBy,
    RequiresPrecondition,
    ProducesPostcondition,
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
            CausalEdgeKind::CausallyPredicts => write!(f, "causally_predicts"),
            CausalEdgeKind::CounterfactualAlternative => write!(f, "counterfactual_alternative"),
            CausalEdgeKind::InvalidatedBy => write!(f, "invalidated_by"),
            CausalEdgeKind::RequiresPrecondition => write!(f, "requires_precondition"),
            CausalEdgeKind::ProducesPostcondition => write!(f, "produces_postcondition"),
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

    pub fn depends_on(weight: f32) -> Self {
        Self::new(CausalEdgeKind::DependsOn, weight, false)
    }

    pub fn causally_predicts(weight: f32) -> Self {
        Self::new(CausalEdgeKind::CausallyPredicts, weight, false)
    }

    pub fn requires_precondition(weight: f32) -> Self {
        Self::new(CausalEdgeKind::RequiresPrecondition, weight, true)
    }

    pub fn counterfactual_alternative(weight: f32) -> Self {
        Self::new(CausalEdgeKind::CounterfactualAlternative, weight, false)
    }

    pub fn invalidated_by(weight: f32) -> Self {
        Self::new(CausalEdgeKind::InvalidatedBy, weight, true)
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

/// Precondition required for an action or tool invocation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CausalPrecondition {
    pub id: String,
    pub description: String,
    #[serde(default)]
    pub required_state: serde_json::Value,
    #[serde(default)]
    pub observed_state: Option<serde_json::Value>,
    #[serde(default = "default_condition_satisfied")]
    pub satisfied: bool,
}

fn default_condition_satisfied() -> bool {
    true
}

impl CausalPrecondition {
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            required_state: serde_json::json!({}),
            observed_state: None,
            satisfied: true,
        }
    }

    pub fn with_required_state(mut self, state: serde_json::Value) -> Self {
        self.required_state = state;
        self
    }

    pub fn with_observed(mut self, observed: Option<serde_json::Value>, satisfied: bool) -> Self {
        self.observed_state = observed;
        self.satisfied = satisfied;
        self
    }
}

/// Expected or observed postcondition for an action.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CausalPostcondition {
    pub id: String,
    pub description: String,
    #[serde(default)]
    pub expected_state: serde_json::Value,
    #[serde(default)]
    pub observed_state: Option<serde_json::Value>,
    #[serde(default = "default_condition_satisfied")]
    pub satisfied: bool,
}

impl CausalPostcondition {
    pub fn new(id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            expected_state: serde_json::json!({}),
            observed_state: None,
            satisfied: true,
        }
    }

    pub fn with_expected_state(mut self, state: serde_json::Value) -> Self {
        self.expected_state = state;
        self
    }

    pub fn with_observed(mut self, observed: Option<serde_json::Value>, satisfied: bool) -> Self {
        self.observed_state = observed;
        self.satisfied = satisfied;
        self
    }
}

/// Formal expectation model for an action: preconditions -> action -> expected postconditions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActionExpectation {
    pub action_id: String,
    #[serde(default)]
    pub action_input: serde_json::Value,
    #[serde(default)]
    pub preconditions: Vec<CausalPrecondition>,
    #[serde(default)]
    pub expected_postconditions: Vec<CausalPostcondition>,
    #[serde(default)]
    pub context_variables: HashMap<String, serde_json::Value>,
    #[serde(default = "default_condition_satisfied")]
    pub expected_success: bool,
}

impl ActionExpectation {
    pub fn new(action_id: impl Into<String>) -> Self {
        Self {
            action_id: action_id.into(),
            action_input: serde_json::json!({}),
            preconditions: Vec::new(),
            expected_postconditions: Vec::new(),
            context_variables: HashMap::new(),
            expected_success: true,
        }
    }

    pub fn with_input(mut self, input: serde_json::Value) -> Self {
        self.action_input = input;
        self
    }

    pub fn with_precondition(mut self, pre: CausalPrecondition) -> Self {
        self.preconditions.push(pre);
        self
    }

    pub fn with_expected_postcondition(mut self, post: CausalPostcondition) -> Self {
        self.expected_postconditions.push(post);
        self
    }

    pub fn with_variable(mut self, key: impl Into<String>, val: serde_json::Value) -> Self {
        self.context_variables.insert(key.into(), val);
        self
    }

    pub fn with_expected_success(mut self, expected: bool) -> Self {
        self.expected_success = expected;
        self
    }
}

/// Actual observed outcome of an executed action.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ObservedOutcome {
    pub success: bool,
    #[serde(default)]
    pub actual_output: Option<serde_json::Value>,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub observed_postconditions: Vec<CausalPostcondition>,
    #[serde(default)]
    pub observed_variables: HashMap<String, serde_json::Value>,
}

impl ObservedOutcome {
    pub fn success(output: serde_json::Value) -> Self {
        Self {
            success: true,
            actual_output: Some(output),
            error_message: None,
            exit_code: Some(0),
            observed_postconditions: Vec::new(),
            observed_variables: HashMap::new(),
        }
    }

    pub fn failure(error: impl Into<String>, exit_code: Option<i32>) -> Self {
        Self {
            success: false,
            actual_output: None,
            error_message: Some(error.into()),
            exit_code,
            observed_postconditions: Vec::new(),
            observed_variables: HashMap::new(),
        }
    }

    pub fn with_postconditions(mut self, posts: Vec<CausalPostcondition>) -> Self {
        self.observed_postconditions = posts;
        self
    }

    pub fn with_variable(mut self, key: impl Into<String>, val: serde_json::Value) -> Self {
        self.observed_variables.insert(key.into(), val);
        self
    }
}

/// Categorization of prediction error mismatch.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PredictionMismatchType {
    /// Action failed unexpectedly when success was predicted.
    UnexpectedFailure,
    /// Action succeeded unexpectedly when failure or error was expected.
    UnexpectedSuccess,
    /// Action returned success, but one or more postconditions were violated.
    PostconditionViolation,
    /// A required precondition was assumed true but was unmet.
    UnsatisfiedPrecondition,
    /// Collateral invariant or contract broken unexpectedly.
    InvariantBreach,
}

impl std::fmt::Display for PredictionMismatchType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PredictionMismatchType::UnexpectedFailure => write!(f, "unexpected_failure"),
            PredictionMismatchType::UnexpectedSuccess => write!(f, "unexpected_success"),
            PredictionMismatchType::PostconditionViolation => write!(f, "postcondition_violation"),
            PredictionMismatchType::UnsatisfiedPrecondition => {
                write!(f, "unsatisfied_precondition")
            }
            PredictionMismatchType::InvariantBreach => write!(f, "invariant_breach"),
        }
    }
}

/// Quantified discrepancy between expectation and reality.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PredictionError {
    pub id: String,
    pub action_id: String,
    pub mismatch_type: PredictionMismatchType,
    /// Discrepancy score [0.0, 1.0], where 0.0 is perfect prediction and 1.0 is complete divergence.
    pub discrepancy_score: f32,
    pub unmet_postconditions: Vec<CausalPostcondition>,
    pub violated_preconditions: Vec<CausalPrecondition>,
    pub error_details: String,
    pub timestamp: DateTime<Utc>,
}

/// A counterfactual "what if" hypothesis for an intervention.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CounterfactualHypothesis {
    pub variable_name: String,
    pub observed_value: serde_json::Value,
    pub counterfactual_value: serde_json::Value,
    pub intervention_description: String,
    /// Estimated probability [0.0, 1.0] that this intervention would flip the outcome to expected success.
    pub estimated_reversal_probability: f32,
    pub causal_mechanism: String,
}

/// Causal attribution report isolating the pivot variable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CounterfactualAttribution {
    pub error_id: String,
    pub action_id: String,
    pub pivot_variable: String,
    pub attribution_confidence: f32,
    pub hypotheses: Vec<CounterfactualHypothesis>,
    pub minimal_sufficient_intervention: Option<CounterfactualHypothesis>,
    pub root_cause_explanation: String,
}

/// Dynamic belief update instructions to feed into CausalGraph and JTMS.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CausalBeliefUpdate {
    pub action_id: String,
    /// Assumptions / semantic facts that should be invalidated in JTMS.
    pub invalidated_assumptions: Vec<String>,
    /// New causal edges to register in the causal graph (source_id, target_id, edge).
    pub new_causal_edges: Vec<(String, String, CausalEdge)>,
    /// Nodes whose confidence should be penalized/downgraded.
    pub node_confidence_penalties: HashMap<String, f32>,
    /// New invariants or anti-patterns discovered from this counterfactual learning cycle.
    pub learned_invariants: Vec<String>,
}

/// Result of counterfactual simulation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CounterfactualSimulationResult {
    pub action_id: String,
    pub tested_intervention: CounterfactualHypothesis,
    pub predicted_success_rate: f32,
    pub bypasses_failure: bool,
    pub rationale: String,
}
