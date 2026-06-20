use std::collections::BTreeMap;

use online_dsl_forge::{
  BlobFileResolver, BlobStore, MemoryFileResolver, RulepackRenderOptions,
  referenced_rulepack_files, render_rulepack_bundle, render_rulepack_for_install,
};

#[test]
fn memory_resolver_renders_referenced_rule_and_group_files() {
  let resolver = MemoryFileResolver::new()
    .with_file(
      "rules/login.oxirule.toml",
      "when = \"Context.App == '{{app}}'\"\n",
    )
    .with_file(
      "groups/common.oxirule-group.toml",
      "[[rule_groups]]\nname = \"{{app}}-group\"\n",
    );
  let options = RulepackRenderOptions {
    variables: BTreeMap::from([("app".to_string(), "vault".to_string())]),
    pin_variables: true,
    ..RulepackRenderOptions::default()
  };

  let bundle = render_rulepack_bundle(manifest_with_paths(), "test rulepack", options, &resolver)
    .expect("bundle should render");

  assert_eq!(bundle.summary.name, "demo");
  assert!(bundle.manifest.contains("default = \"vault\""));
  assert_eq!(bundle.files.len(), 2);
  assert!(bundle.files[0].content.contains("Context.App == 'vault'"));
  assert!(bundle.files[1].content.contains("vault-group"));
  assert_eq!(bundle.summary.loaded_files.len(), 2);
}

#[test]
fn blob_resolver_renders_referenced_files() {
  let mut store = BlobStore::new();
  store.insert(
    "rule-login",
    "when = \"Request.Path.starts_with('{{prefix}}')\"\n",
  );
  let resolver =
    BlobFileResolver::new(store).with_mapping("rules/login.oxirule.toml", "rule-login");
  let options = RulepackRenderOptions {
    variables: BTreeMap::from([("prefix".to_string(), "/admin".to_string())]),
    ..RulepackRenderOptions::default()
  };

  let bundle = render_rulepack_bundle(
    &manifest_with_rule_path("prefix"),
    "test rulepack",
    options,
    &resolver,
  )
  .expect("blob bundle should render");

  assert_eq!(bundle.files.len(), 1);
  assert!(bundle.files[0].content.contains("/admin"));
}

#[test]
fn missing_resolver_file_fails_closed() {
  let error = render_rulepack_bundle(
    &manifest_with_rule_path("prefix"),
    "test rulepack",
    RulepackRenderOptions {
      variables: BTreeMap::from([("prefix".to_string(), "/admin".to_string())]),
      ..RulepackRenderOptions::default()
    },
    &MemoryFileResolver::new(),
  )
  .expect_err("missing referenced file should fail");

  assert!(error.to_string().contains("referenced rulepack file"));
}

#[test]
fn unsafe_referenced_paths_are_rejected() {
  let error = referenced_rulepack_files(
    r#"[rulepack]
schema_version = 2
name = "demo"
version = "0.1.0"

[[rules]]
name = "login"
phase = "request"
priority = 100
path = "../rules/login.oxirule.toml"
"#,
    "test rulepack",
    RulepackRenderOptions::default(),
  )
  .expect_err("path traversal should fail");

  assert!(error.to_string().contains("safe relative path"));
}

#[test]
fn render_rejects_unknown_and_invalid_variables() {
  let unknown = render_rulepack_for_install(
    &manifest_with_rule_path("admin_cidr"),
    "test rulepack",
    RulepackRenderOptions {
      variables: BTreeMap::from([("unknown".to_string(), "value".to_string())]),
      ..RulepackRenderOptions::default()
    },
  )
  .expect_err("unknown variable should fail");
  assert!(unknown.to_string().contains("does not declare variable"));

  let invalid_cidr = render_rulepack_for_install(
    &manifest_with_rule_path("admin_cidr"),
    "test rulepack",
    RulepackRenderOptions {
      variables: BTreeMap::from([("admin_cidr".to_string(), "not-cidr".to_string())]),
      ..RulepackRenderOptions::default()
    },
  )
  .expect_err("invalid CIDR should fail");
  assert!(invalid_cidr.to_string().contains("valid CIDR"));
}

fn manifest_with_paths() -> &'static str {
  r#"[rulepack]
schema_version = 2
name = "demo"
version = "0.1.0"

[[variables]]
name = "app"
type = "string"
required = true

[[rules]]
name = "login"
phase = "request"
priority = 100
path = "rules/login.oxirule.toml"

[[group_files]]
path = "groups/common.oxirule-group.toml"
"#
}

fn manifest_with_rule_path(variable_name: &str) -> String {
  format!(
    r#"[rulepack]
schema_version = 2
name = "demo"
version = "0.1.0"

[[variables]]
name = "{variable_name}"
type = "{}"
required = true

[[rules]]
name = "login"
phase = "request"
priority = 100
path = "rules/login.oxirule.toml"
"#,
    if variable_name == "admin_cidr" {
      "cidr"
    } else {
      "string"
    }
  )
}
