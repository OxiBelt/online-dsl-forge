use std::collections::{BTreeMap, BTreeSet};

use crate::parser::{AstExpression, BinaryOp, UnaryOp};
use serde::{Deserialize, Serialize};

use crate::sema::profile::{BodyAccess, BodyTarget, Phase};

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeClass {
  Null,
  Bool,
  Int,
  Float,
  String,
  Bytes,
  Array,
  Object,
  Dyn,
  RegexLiteral,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityKind {
  Function,
  Method,
  UnaryOp,
  BinaryOp,
}

#[derive(Debug, Clone, Copy, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegexFlavor {
  Default,
  HeaderName,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct RegexArgMeta {
  pub index: usize,
  pub flavor: RegexFlavor,
}

impl RegexArgMeta {
  pub fn new(index: usize, flavor: RegexFlavor) -> Self {
    Self { index, flavor }
  }
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CostModel {
  Constant(u32),
  LinearInput { factor: u32 },
  LinearCollection { factor: u32 },
  RegexMatch { factor: u32, precompiled: bool },
}

impl CostModel {
  pub fn static_cost(&self) -> u64 {
    match self {
      Self::Constant(value) => u64::from(*value),
      Self::LinearInput { factor }
      | Self::LinearCollection { factor }
      | Self::RegexMatch { factor, .. } => u64::from(*factor),
    }
  }
}

impl Default for CostModel {
  fn default() -> Self {
    Self::Constant(1)
  }
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct VariableMeta {
  pub name: String,
  pub type_class: TypeClass,
  pub phases: BTreeSet<Phase>,
}

impl VariableMeta {
  pub fn new(name: impl Into<String>) -> Self {
    Self {
      name: name.into(),
      type_class: TypeClass::Dyn,
      phases: BTreeSet::new(),
    }
  }

  pub fn with_type(mut self, type_class: TypeClass) -> Self {
    self.type_class = type_class;
    self
  }

  pub fn with_phases(mut self, phases: impl IntoIterator<Item = Phase>) -> Self {
    self.phases = phases.into_iter().collect();
    self
  }

  pub fn is_available_in(&self, phase: Phase) -> bool {
    self.phases.is_empty() || self.phases.contains(&phase)
  }
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilityMeta {
  pub name: String,
  pub kind: CapabilityKind,
  pub arity: usize,
  pub receiver: Option<TypeClass>,
  pub args: Vec<TypeClass>,
  pub result: TypeClass,
  pub phases: BTreeSet<Phase>,
  pub body_access: BodyAccess,
  pub regex_args: Vec<RegexArgMeta>,
  pub deterministic: bool,
  pub side_effect_free: bool,
  pub cost: CostModel,
}

impl CapabilityMeta {
  pub fn function(name: impl Into<String>, arity: usize) -> Self {
    Self::new(name, CapabilityKind::Function, arity)
  }

  pub fn method(name: impl Into<String>, arity: usize) -> Self {
    Self::new(name, CapabilityKind::Method, arity)
  }

  pub fn unary_operator(op: UnaryOp) -> Self {
    Self::new(op.as_str(), CapabilityKind::UnaryOp, 1)
  }

  pub fn binary_operator(op: BinaryOp) -> Self {
    Self::new(op.as_str(), CapabilityKind::BinaryOp, 2)
  }

  pub fn with_phases(mut self, phases: impl IntoIterator<Item = Phase>) -> Self {
    self.phases = phases.into_iter().collect();
    self
  }

  pub fn with_body_access(mut self, access: BodyAccess) -> Self {
    self.body_access = access;
    self
  }

  pub fn with_regex_arg(mut self, index: usize, flavor: RegexFlavor) -> Self {
    self.regex_args.push(RegexArgMeta::new(index, flavor));
    self
  }

  pub fn with_cost(mut self, cost: CostModel) -> Self {
    self.cost = cost;
    self
  }

  pub fn is_available_in(&self, phase: Phase) -> bool {
    self.phases.is_empty() || self.phases.contains(&phase)
  }

  fn new(name: impl Into<String>, kind: CapabilityKind, arity: usize) -> Self {
    Self {
      name: name.into(),
      kind,
      arity,
      receiver: None,
      args: vec![TypeClass::Dyn; arity],
      result: TypeClass::Dyn,
      phases: BTreeSet::new(),
      body_access: BodyAccess::None,
      regex_args: Vec::new(),
      deterministic: true,
      side_effect_free: true,
      cost: CostModel::default(),
    }
  }
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilityTicket {
  pub kind: CapabilityKind,
  pub name: String,
  pub arity: usize,
}

impl CapabilityTicket {
  pub fn new(kind: CapabilityKind, name: impl Into<String>, arity: usize) -> Self {
    Self {
      kind,
      name: name.into(),
      arity,
    }
  }
}

impl Ord for CapabilityTicket {
  fn cmp(&self, other: &Self) -> std::cmp::Ordering {
    (self.kind_order(), &self.name, self.arity).cmp(&(other.kind_order(), &other.name, other.arity))
  }
}

impl PartialOrd for CapabilityTicket {
  fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
    Some(self.cmp(other))
  }
}

impl CapabilityTicket {
  fn kind_order(&self) -> u8 {
    match self.kind {
      CapabilityKind::Function => 0,
      CapabilityKind::Method => 1,
      CapabilityKind::UnaryOp => 2,
      CapabilityKind::BinaryOp => 3,
    }
  }
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct BodyPathRule {
  pub path: Vec<String>,
  pub target: BodyTarget,
  pub access: BodyAccess,
}

impl BodyPathRule {
  pub fn new(
    path: impl IntoIterator<Item = impl Into<String>>,
    target: BodyTarget,
    access: BodyAccess,
  ) -> Self {
    Self {
      path: path.into_iter().map(Into::into).collect(),
      target,
      access,
    }
  }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
pub struct ExpressionFunction {
  pub name: String,
  pub params: Vec<String>,
  pub expression: AstExpression,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Serialize)]
pub struct RuntimeSchema {
  variables: BTreeMap<String, VariableMeta>,
  functions: BTreeMap<String, BTreeMap<usize, CapabilityMeta>>,
  methods: BTreeMap<String, BTreeMap<usize, CapabilityMeta>>,
  unary_ops: BTreeMap<String, CapabilityMeta>,
  binary_ops: BTreeMap<String, CapabilityMeta>,
  body_paths: Vec<BodyPathRule>,
  expression_functions: BTreeMap<String, ExpressionFunction>,
}

impl RuntimeSchema {
  pub fn new() -> Self {
    Self::default()
  }

  pub fn waf() -> Self {
    let mut schema = Self::new();
    schema
      .add_variable("Request")
      .add_variable("Response")
      .add_variable("Stream")
      .add_waf_body_paths()
      .add_method_capability(
        CapabilityMeta::method("contains", 1).with_body_access(BodyAccess::PrefixBytes),
      )
      .add_method_capability(
        CapabilityMeta::method("containsBytes", 1).with_body_access(BodyAccess::PrefixBytes),
      )
      .add_method_capability(
        CapabilityMeta::method("matches", 1)
          .with_body_access(BodyAccess::PrefixBytes)
          .with_regex_arg(0, RegexFlavor::Default),
      );
    schema
  }

  pub fn add_variable(&mut self, name: impl Into<String>) -> &mut Self {
    self.add_variable_meta(VariableMeta::new(name))
  }

  pub fn add_variable_meta(&mut self, variable: VariableMeta) -> &mut Self {
    self.variables.insert(variable.name.clone(), variable);
    self
  }

  pub fn add_function(&mut self, name: impl Into<String>, arity: usize) -> &mut Self {
    self.add_function_capability(CapabilityMeta::function(name, arity))
  }

  pub fn add_function_capability(&mut self, capability: CapabilityMeta) -> &mut Self {
    self
      .functions
      .entry(capability.name.clone())
      .or_default()
      .insert(capability.arity, capability);
    self
  }

  pub fn add_method(&mut self, name: impl Into<String>, arity: usize) -> &mut Self {
    self.add_method_capability(CapabilityMeta::method(name, arity))
  }

  pub fn add_method_capability(&mut self, capability: CapabilityMeta) -> &mut Self {
    self
      .methods
      .entry(capability.name.clone())
      .or_default()
      .insert(capability.arity, capability);
    self
  }

  pub fn add_unary_operator_capability(&mut self, capability: CapabilityMeta) -> &mut Self {
    self.unary_ops.insert(capability.name.clone(), capability);
    self
  }

  pub fn add_binary_operator_capability(&mut self, capability: CapabilityMeta) -> &mut Self {
    self.binary_ops.insert(capability.name.clone(), capability);
    self
  }

  pub fn add_body_path(
    &mut self,
    path: impl IntoIterator<Item = impl Into<String>>,
    target: BodyTarget,
    access: BodyAccess,
  ) -> &mut Self {
    self
      .body_paths
      .push(BodyPathRule::new(path, target, access));
    self
  }

  pub fn add_waf_body_paths(&mut self) -> &mut Self {
    for root in ["Request", "Response"] {
      let target = if root == "Request" {
        BodyTarget::Request
      } else {
        BodyTarget::Response
      };
      self.add_body_path([root, "Body", "Size"], target, BodyAccess::SizeOnly);
      self.add_body_path([root, "Body", "Bytes"], target, BodyAccess::PrefixBytes);
      self.add_body_path([root, "Body", "Text"], target, BodyAccess::PrefixBytes);
      self.add_body_path(
        [root, "Body", "IsTruncated"],
        target,
        BodyAccess::PrefixBytes,
      );
    }
    self.add_body_path(
      ["Stream", "Payload"],
      BodyTarget::Stream,
      BodyAccess::PrefixBytes,
    );
    self
  }

  pub fn add_expression_function(
    &mut self,
    name: impl Into<String>,
    params: impl IntoIterator<Item = impl Into<String>>,
    expression: AstExpression,
  ) -> &mut Self {
    let name = name.into();
    let params = params.into_iter().map(Into::into).collect::<Vec<_>>();
    self.add_function(name.clone(), params.len());
    self.expression_functions.insert(
      name.clone(),
      ExpressionFunction {
        name,
        params,
        expression,
      },
    );
    self
  }

  pub fn has_variable(&self, name: &str) -> bool {
    self.variables.contains_key(name)
  }

  pub fn variable(&self, name: &str) -> Option<&VariableMeta> {
    self.variables.get(name)
  }

  pub fn function_accepts(&self, name: &str, arity: usize) -> SignatureMatch {
    signature_accepts(&self.functions, name, arity)
  }

  pub fn method_accepts(&self, name: &str, arity: usize) -> SignatureMatch {
    signature_accepts(&self.methods, name, arity)
  }

  pub fn function_capability(&self, name: &str, arity: usize) -> Option<&CapabilityMeta> {
    self
      .functions
      .get(name)
      .and_then(|entries| entries.get(&arity))
  }

  pub fn method_capability(&self, name: &str, arity: usize) -> Option<&CapabilityMeta> {
    self
      .methods
      .get(name)
      .and_then(|entries| entries.get(&arity))
  }

  pub fn body_access_for_path(&self, path: &[String]) -> Option<(BodyTarget, BodyAccess)> {
    self
      .body_paths
      .iter()
      .find(|rule| rule.path == path)
      .map(|rule| (rule.target, rule.access))
  }

  pub fn expression_function(&self, name: &str) -> Option<&ExpressionFunction> {
    self.expression_functions.get(name)
  }

  pub fn expression_functions(&self) -> impl Iterator<Item = &ExpressionFunction> {
    self.expression_functions.values()
  }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SignatureMatch {
  Unknown,
  ArityMismatch,
  Matches,
}

fn signature_accepts(
  signatures: &BTreeMap<String, BTreeMap<usize, CapabilityMeta>>,
  name: &str,
  arity: usize,
) -> SignatureMatch {
  match signatures.get(name) {
    Some(accepted) if accepted.contains_key(&arity) => SignatureMatch::Matches,
    Some(_) => SignatureMatch::ArityMismatch,
    None => SignatureMatch::Unknown,
  }
}
