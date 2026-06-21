use crate::sema::profile::{BodyAccess, BodyTarget, Phase};
use crate::sema::schema::{CapabilityMeta, RegexFlavor, RuntimeSchema, VariableMeta};

impl RuntimeSchema {
  pub fn oxirule_waf() -> Self {
    let mut schema = Self::new();
    schema
      .add_variable_meta(VariableMeta::new("Context").with_phases([
        Phase::Request,
        Phase::Response,
        Phase::Stream,
      ]))
      .add_variable_meta(VariableMeta::new("Request").with_phases([
        Phase::Request,
        Phase::Response,
        Phase::Stream,
      ]))
      .add_variable_meta(VariableMeta::new("DynamicPolicy").with_phases([
        Phase::Request,
        Phase::Response,
        Phase::Stream,
      ]))
      .add_variable_meta(VariableMeta::new("Response").with_phases([Phase::Response]))
      .add_variable_meta(VariableMeta::new("Stream").with_phases([Phase::Stream]))
      .add_waf_body_paths()
      .add_oxirule_body_paths()
      .add_oxirule_methods();
    schema
  }

  pub fn add_oxirule_body_paths(&mut self) -> &mut Self {
    for root in ["Request", "Response"] {
      let target = if root == "Request" {
        BodyTarget::Request
      } else {
        BodyTarget::Response
      };
      self.add_body_path([root, "Http", "Body", "Size"], target, BodyAccess::SizeOnly);
      self.add_body_path(
        [root, "Http", "Body", "Bytes"],
        target,
        BodyAccess::PrefixBytes,
      );
      self.add_body_path(
        [root, "Http", "Body", "Text"],
        target,
        BodyAccess::PrefixBytes,
      );
      self.add_body_path(
        [root, "Http", "Body", "IsTruncated"],
        target,
        BodyAccess::PrefixBytes,
      );
    }
    self
  }

  pub fn add_oxirule_methods(&mut self) -> &mut Self {
    self
      .add_waf_body_methods()
      .add_waf_regex_methods()
      .add_oxirule_string_methods()
      .add_oxirule_collection_methods()
      .add_oxirule_token_binding_methods()
  }

  pub fn add_oxirule_string_methods(&mut self) -> &mut Self {
    self
      .add_method_capability(
        CapabilityMeta::method("contains", 1).with_body_access(BodyAccess::PrefixBytes),
      )
      .add_method_capability(CapabilityMeta::method("startsWith", 1))
      .add_method_capability(CapabilityMeta::method("endsWith", 1))
      .add_method_capability(
        CapabilityMeta::method("matches", 1)
          .with_body_access(BodyAccess::PrefixBytes)
          .with_regex_arg(0, RegexFlavor::Default),
      )
      .add_method_capability(CapabilityMeta::method("lowerAscii", 0))
      .add_method_capability(CapabilityMeta::method("upperAscii", 0))
      .add_method_capability(CapabilityMeta::method("size", 0))
      .add_method_capability(CapabilityMeta::method("inCidr", 1));
    self
  }

  pub fn add_oxirule_collection_methods(&mut self) -> &mut Self {
    self
      .add_method_capability(CapabilityMeta::method("count", 0))
      .add_method_capability(CapabilityMeta::method("has", 1))
      .add_method_capability(CapabilityMeta::method("get", 1))
      .add_method_capability(CapabilityMeta::method("getAll", 1))
      .add_method_capability(CapabilityMeta::method("anyValueContains", 1))
      .add_method_capability(
        CapabilityMeta::method("anyNameMatches", 1)
          .with_regex_arg(0, RegexFlavor::Default)
          .with_regex_arg(0, RegexFlavor::HeaderName),
      )
      .add_method_capability(
        CapabilityMeta::method("anyValueMatches", 1).with_regex_arg(0, RegexFlavor::Default),
      )
      .add_method_capability(
        CapabilityMeta::method("anyKeyMatches", 1).with_regex_arg(0, RegexFlavor::Default),
      )
      .add_method_capability(
        CapabilityMeta::method("anyMatches", 1).with_regex_arg(0, RegexFlavor::Default),
      )
      .add_method_capability(
        CapabilityMeta::method("anyEntryMatches", 2)
          .with_regex_arg(0, RegexFlavor::Default)
          .with_regex_arg(0, RegexFlavor::HeaderName)
          .with_regex_arg(1, RegexFlavor::Default),
      )
      .add_method_capability(
        CapabilityMeta::method("allEntriesMatch", 2)
          .with_regex_arg(0, RegexFlavor::HeaderName)
          .with_regex_arg(1, RegexFlavor::Default),
      );
    self
  }

  pub fn add_oxirule_token_binding_methods(&mut self) -> &mut Self {
    self
      .add_method_capability(CapabilityMeta::method("directPeerIpNetworkPrefix", 2))
      .add_method_capability(CapabilityMeta::method("tcpMaxHop", 1));
    self
  }
}
