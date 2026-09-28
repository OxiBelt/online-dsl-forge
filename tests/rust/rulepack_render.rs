use std::collections::BTreeMap;

use online_dsl_forge::rulepack_render::{
  RulepackRenderLimits, inspect_rulepack, inspect_rulepack_inputs,
  inspect_rulepack_inputs_with_limits, render_rulepack_bundle_with_limits,
  render_rulepack_for_install_with_limits, render_text, render_text_with_limits,
};
use online_dsl_forge::{
  BlobFileResolver, BlobStore, FileResolver, MemoryFileResolver, RulepackActionSelector,
  RulepackException, RulepackOverride, RulepackOverrideSelector, RulepackRenderOptions,
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
  let rule: toml::Value = toml::from_str(&bundle.files[0].content).expect("rule TOML should parse");
  assert_eq!(
    rule.get("when").and_then(toml::Value::as_str),
    Some("Context.App == \"vault\"")
  );
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

#[test]
fn render_rejects_non_finite_rate_variables() {
  let finite = render_rulepack_for_install(
    &manifest_with_rate_variable(),
    "test rulepack",
    RulepackRenderOptions {
      variables: BTreeMap::from([("limit".to_string(), "5r/m".to_string())]),
      ..RulepackRenderOptions::default()
    },
  )
  .expect("finite positive rate should render");
  assert!(finite.contains("rate = \"5r/m\""));

  for value in ["0r/s", "-1r/s"] {
    let error = render_rulepack_for_install(
      &manifest_with_rate_variable(),
      "test rulepack",
      RulepackRenderOptions {
        variables: BTreeMap::from([("limit".to_string(), value.to_string())]),
        ..RulepackRenderOptions::default()
      },
    )
    .expect_err("nonpositive rate should fail closed");

    assert!(error.to_string().contains("greater than 0"));
  }

  for value in ["NaNr/s", "infr/m", "infinityr/h", "1e309r/s"] {
    let error = render_rulepack_for_install(
      &manifest_with_rate_variable(),
      "test rulepack",
      RulepackRenderOptions {
        variables: BTreeMap::from([("limit".to_string(), value.to_string())]),
        ..RulepackRenderOptions::default()
      },
    )
    .expect_err("non-finite rate variable should fail closed");

    assert!(error.to_string().contains("rate amount must be finite"));
  }
}

#[test]
fn render_rejects_non_finite_rate_overrides() {
  for value in ["NaNr/s", "infr/m", "infinityr/h", "1e309r/s"] {
    let error = render_rulepack_for_install(
      &manifest_with_rate_variable(),
      "test rulepack",
      RulepackRenderOptions {
        variables: BTreeMap::from([("limit".to_string(), "5r/m".to_string())]),
        local_overrides: vec![rate_override(value)],
        ..RulepackRenderOptions::default()
      },
    )
    .expect_err("non-finite rate override should fail closed");

    assert!(error.to_string().contains("rate amount must be finite"));
  }
}

#[test]
fn text_rendering_is_single_pass_and_does_not_rescan_replacements() {
  let variables = BTreeMap::from([
    ("a".to_string(), "{{b}}".to_string()),
    ("b".to_string(), "expanded".to_string()),
  ]);

  assert_eq!(render_text("{{a}}/{{b}}", &variables), "{{b}}/expanded");
  assert_eq!(
    render_text_with_limits("{{a}}/{{b}}", &variables, RulepackRenderLimits::default())
      .expect("bounded render should succeed"),
    "{{b}}/expanded"
  );
}

#[test]
fn bounded_text_rejects_unknown_unresolved_and_malformed_markers() {
  let variables = BTreeMap::from([("known".to_string(), "value".to_string())]);
  for raw in ["{{unknown}}", "{{known", "known}}", "{{}}", "{{{known}}}"] {
    assert!(
      render_text_with_limits(raw, &variables, RulepackRenderLimits::default()).is_err(),
      "marker {raw:?} must fail closed"
    );
  }

  let error = render_rulepack_for_install(
    &manifest_with_optional_placeholder("description = \"{{optional}}\""),
    "test rulepack",
    RulepackRenderOptions::default(),
  )
  .expect_err("declared but unresolved placeholder should fail");
  assert!(
    error
      .to_string()
      .contains("unresolved placeholder optional")
  );
}

#[test]
fn bounded_text_limits_accept_exact_values_and_reject_next_and_zero() {
  let variables = BTreeMap::from([("x".to_string(), "abc".to_string())]);
  let exact = RulepackRenderLimits {
    max_manifest_bytes: 5,
    max_variable_value_bytes: 3,
    max_total_variable_bytes: 4,
    max_variables: 1,
    max_placeholders: 1,
    max_total_input_bytes: 9,
    max_total_output_bytes: 3,
    ..RulepackRenderLimits::default()
  };
  assert_eq!(
    render_text_with_limits("{{x}}", &variables, exact).expect("exact limits should succeed"),
    "abc"
  );
  assert!(
    render_text_with_limits(
      "{{x}}",
      &variables,
      RulepackRenderLimits {
        max_total_output_bytes: 2,
        ..exact
      }
    )
    .is_err()
  );
  assert!(
    render_text_with_limits(
      "{{x}}",
      &variables,
      RulepackRenderLimits {
        max_placeholders: 0,
        ..exact
      }
    )
    .is_err()
  );
  assert_eq!(
    render_text_with_limits(
      "",
      &BTreeMap::new(),
      RulepackRenderLimits {
        max_manifest_bytes: 0,
        max_referenced_file_bytes: 0,
        max_variable_value_bytes: 0,
        max_total_variable_bytes: 0,
        max_variables: 0,
        max_profile_assignments: 0,
        max_rulepack_files: 0,
        max_overrides: 0,
        max_exceptions: 0,
        max_selector_work: 0,
        max_local_option_bytes: 0,
        max_override_body_bytes: 0,
        max_placeholders: 0,
        max_total_input_bytes: 0,
        max_total_output_bytes: 0,
      }
    )
    .expect("empty input should succeed at zero limits"),
    ""
  );
}

#[test]
fn supplied_unknown_variables_are_metered_before_name_resolution() {
  let error = render_rulepack_for_install_with_limits(
    &minimal_manifest(),
    "test rulepack",
    RulepackRenderOptions {
      variables: BTreeMap::from([("unknown".to_string(), "1234".to_string())]),
      ..RulepackRenderOptions::default()
    },
    RulepackRenderLimits {
      max_variable_value_bytes: 3,
      ..RulepackRenderLimits::default()
    },
  )
  .expect_err("oversized unknown input must fail resource admission first");
  assert!(error.to_string().contains("variable value bytes limit"));
}

#[test]
fn typed_when_rendering_contains_injection_payload_as_string_data() {
  let payload = "' || true || '\nnext";
  let resolver = MemoryFileResolver::new().with_file(
    "rules/login.oxirule.toml",
    "when = \"Context.RouteName == '{{route_name}}'\"\n",
  );
  let bundle = render_rulepack_bundle(
    &manifest_with_rule_path("route_name"),
    "test rulepack",
    RulepackRenderOptions {
      variables: BTreeMap::from([("route_name".to_string(), payload.to_string())]),
      ..RulepackRenderOptions::default()
    },
    &resolver,
  )
  .expect("payload should remain string data");
  let file: toml::Value = toml::from_str(&bundle.files[0].content).expect("rendered TOML parses");
  let when = file.get("when").and_then(toml::Value::as_str).unwrap();
  let ast = online_dsl_forge::parse_expression(when).expect("rendered expression parses");
  assert_eq!(online_dsl_forge::format_expression(&ast), when);
  assert!(when.contains("\\nnext"));
}

#[test]
fn dsl_string_placeholders_are_counted_after_parsing() {
  let manifest = manifest_with_rule_path("route_name");
  let resolver = MemoryFileResolver::new().with_file(
    "rules/login.oxirule.toml",
    r#"when = "Context.RouteName == '{{route_name}}'"
"#,
  );
  let options = RulepackRenderOptions {
    variables: BTreeMap::from([("route_name".to_string(), "vault".to_string())]),
    ..RulepackRenderOptions::default()
  };
  let error = render_rulepack_bundle_with_limits(
    &manifest,
    "test rulepack",
    options.clone(),
    &resolver,
    RulepackRenderLimits {
      max_placeholders: 0,
      ..RulepackRenderLimits::default()
    },
  )
  .expect_err("a decoded placeholder must consume the placeholder budget");
  assert!(error.to_string().contains("placeholders limit"));

  let bundle = render_rulepack_bundle_with_limits(
    &manifest,
    "test rulepack",
    options,
    &resolver,
    RulepackRenderLimits {
      max_placeholders: 1,
      ..RulepackRenderLimits::default()
    },
  )
  .expect("the decoded placeholder should be charged exactly once");
  let file: toml::Value = toml::from_str(&bundle.files[0].content).expect("rendered TOML parses");
  assert_eq!(
    file.get("when").and_then(toml::Value::as_str),
    Some("Context.RouteName == \"vault\"")
  );
}

#[test]
fn unsupported_escapes_in_rulepack_when_expressions_fail_closed() {
  let resolver = MemoryFileResolver::new().with_file(
    "rules/login.oxirule.toml",
    r#"when = "Context.RouteName == '\\{route_name}'"
"#,
  );
  let error = render_rulepack_bundle(
    &manifest_with_rule_path("route_name"),
    "test rulepack",
    RulepackRenderOptions {
      variables: BTreeMap::from([("route_name".to_string(), "vault".to_string())]),
      ..RulepackRenderOptions::default()
    },
    &resolver,
  )
  .expect_err("unsupported DSL escape should reject the rulepack");
  assert!(error.to_string().contains("unsupported string escape"));
}

#[test]
fn placeholders_in_toml_keys_and_dsl_syntax_are_rejected() {
  let outer_key = minimal_manifest().replace(
    "version = \"0.1.0\"",
    "version = \"0.1.0\"\n\"{{key}}\" = \"value\"",
  );
  assert!(
    render_rulepack_for_install(
      &outer_key,
      "test rulepack",
      RulepackRenderOptions::default()
    )
    .is_err()
  );

  let resolver = MemoryFileResolver::new().with_file(
    "rules/login.oxirule.toml",
    "when = \"Context.RouteName == {{route_name}}\"\n",
  );
  let error = render_rulepack_bundle(
    &manifest_with_rule_path("route_name"),
    "test rulepack",
    RulepackRenderOptions {
      variables: BTreeMap::from([("route_name".to_string(), "safe".to_string())]),
      ..RulepackRenderOptions::default()
    },
    &resolver,
  )
  .expect_err("DSL syntax placeholder must fail");
  assert!(error.to_string().contains("when expression"));

  let nested_key = MemoryFileResolver::new().with_file(
    "rules/login.oxirule.toml",
    "when = \"true\"\n\"{{route_name}}\" = \"value\"\n",
  );
  assert!(
    render_rulepack_bundle(
      &manifest_with_rule_path("route_name"),
      "test rulepack",
      RulepackRenderOptions {
        variables: BTreeMap::from([("route_name".to_string(), "safe".to_string())]),
        ..RulepackRenderOptions::default()
      },
      &nested_key,
    )
    .is_err()
  );
}

#[test]
fn embedded_rule_content_is_rendered_once_and_canonicalized() {
  let manifest = manifest_with_embedded_when();
  let rendered = render_rulepack_for_install(
    &manifest,
    "test rulepack",
    RulepackRenderOptions {
      variables: BTreeMap::from([
        ("first".to_string(), "{{second}}".to_string()),
        ("second".to_string(), "done".to_string()),
      ]),
      ..RulepackRenderOptions::default()
    },
  )
  .expect("embedded rule should render");
  let manifest_value: toml::Value = toml::from_str(&rendered).expect("manifest parses");
  let content = manifest_value["rules"][0]["content"].as_str().unwrap();
  let nested: toml::Value = toml::from_str(content).expect("nested content parses");
  assert_eq!(
    nested.get("when").and_then(toml::Value::as_str),
    Some("Context.Value == \"{{second}}\"")
  );
}

#[test]
fn aggregate_input_limit_covers_multiple_resolved_files() {
  let manifest = manifest_with_paths();
  let rule = "when = \"true\"\n";
  let group = "[[rule_groups]]\nname = \"common\"\n";
  let resolver = MemoryFileResolver::new()
    .with_file("rules/login.oxirule.toml", rule)
    .with_file("groups/common.oxirule-group.toml", group);
  let exact_bytes = manifest.len() + "app".len() + rule.len() + group.len();
  let limits = RulepackRenderLimits {
    max_total_input_bytes: exact_bytes,
    ..RulepackRenderLimits::default()
  };
  render_rulepack_bundle_with_limits(
    manifest,
    "test rulepack",
    RulepackRenderOptions {
      variables: BTreeMap::from([("app".to_string(), "".to_string())]),
      ..RulepackRenderOptions::default()
    },
    &resolver,
    limits,
  )
  .expect("exact aggregate input should succeed");
  assert!(
    render_rulepack_bundle_with_limits(
      manifest,
      "test rulepack",
      RulepackRenderOptions {
        variables: BTreeMap::from([("app".to_string(), "".to_string())]),
        ..RulepackRenderOptions::default()
      },
      &resolver,
      RulepackRenderLimits {
        max_total_input_bytes: exact_bytes - 1,
        ..limits
      },
    )
    .is_err()
  );
}

#[test]
fn aggregate_output_limit_is_charged_across_files() {
  let manifest = manifest_with_paths();
  let resolver = MemoryFileResolver::new()
    .with_file("rules/login.oxirule.toml", "when = \"true\"\n")
    .with_file(
      "groups/common.oxirule-group.toml",
      "[[rule_groups]]\nname = \"common\"\n",
    );
  let options = RulepackRenderOptions {
    variables: BTreeMap::from([("app".to_string(), "".to_string())]),
    ..RulepackRenderOptions::default()
  };
  let mut rejected = 0usize;
  let mut accepted = RulepackRenderLimits::default().max_total_output_bytes;
  while rejected + 1 < accepted {
    let candidate = rejected + (accepted - rejected) / 2;
    let result = render_rulepack_bundle_with_limits(
      manifest,
      "test rulepack",
      options.clone(),
      &resolver,
      RulepackRenderLimits {
        max_total_output_bytes: candidate,
        ..RulepackRenderLimits::default()
      },
    );
    if result.is_ok() {
      accepted = candidate;
    } else {
      rejected = candidate;
    }
  }
  render_rulepack_bundle_with_limits(
    manifest,
    "test rulepack",
    options.clone(),
    &resolver,
    RulepackRenderLimits {
      max_total_output_bytes: accepted,
      ..RulepackRenderLimits::default()
    },
  )
  .expect("exact conservative render-work boundary should succeed");
  assert!(
    render_rulepack_bundle_with_limits(
      manifest,
      "test rulepack",
      options,
      &resolver,
      RulepackRenderLimits {
        max_total_output_bytes: accepted - 1,
        ..RulepackRenderLimits::default()
      },
    )
    .is_err()
  );
}

#[test]
fn builtin_resolvers_offer_borrowed_size_admission() {
  let manifest = manifest_with_rule_path("prefix");
  let raw = "when = \"true\"\n";
  let memory = MemoryFileResolver::new().with_file("rules/login.oxirule.toml", raw);
  let file = referenced_rulepack_files(
    &manifest,
    "test rulepack",
    RulepackRenderOptions {
      variables: BTreeMap::from([("prefix".to_string(), "x".to_string())]),
      ..RulepackRenderOptions::default()
    },
  )
  .expect("reference inspection should succeed")
  .remove(0);
  assert_eq!(
    memory
      .resolve_file_borrowed(&file)
      .expect("memory resolver should offer borrowed admission")
      .expect("memory file should exist"),
    raw
  );

  let mut store = BlobStore::new();
  store.insert("rule", raw);
  let blobs = BlobFileResolver::new(store).with_mapping("rules/login.oxirule.toml", "rule");
  assert_eq!(
    blobs
      .resolve_file_borrowed(&file)
      .expect("blob resolver should offer borrowed admission")
      .expect("blob should exist"),
    raw
  );

  let error = render_rulepack_bundle_with_limits(
    &manifest,
    "test rulepack",
    RulepackRenderOptions {
      variables: BTreeMap::from([("prefix".to_string(), "x".to_string())]),
      ..RulepackRenderOptions::default()
    },
    &memory,
    RulepackRenderLimits {
      max_referenced_file_bytes: raw.len() - 1,
      ..RulepackRenderLimits::default()
    },
  )
  .expect_err("borrowed content must be rejected before rendering");
  assert!(error.to_string().contains("referenced file bytes limit"));
}

#[test]
fn inline_and_referenced_entries_share_the_file_count_limit() {
  let one = minimal_manifest();
  inspect_rulepack_inputs_with_limits(
    &one,
    "test rulepack",
    RulepackRenderLimits {
      max_rulepack_files: 1,
      ..RulepackRenderLimits::default()
    },
  )
  .expect("one inline rule should meet the exact file limit");
  assert!(
    inspect_rulepack_inputs_with_limits(
      &one,
      "test rulepack",
      RulepackRenderLimits {
        max_rulepack_files: 0,
        ..RulepackRenderLimits::default()
      },
    )
    .is_err()
  );

  let two =
    format!("{one}\n[[group_files]]\ncontent = '''\n[[rule_groups]]\nname = \"common\"\n'''\n");
  assert!(
    inspect_rulepack_inputs_with_limits(
      &two,
      "test rulepack",
      RulepackRenderLimits {
        max_rulepack_files: 1,
        ..RulepackRenderLimits::default()
      },
    )
    .is_err()
  );
}

#[test]
fn manifest_byte_limit_accepts_exact_and_rejects_next() {
  let manifest = minimal_manifest();
  inspect_rulepack_inputs_with_limits(
    &manifest,
    "test rulepack",
    RulepackRenderLimits {
      max_manifest_bytes: manifest.len(),
      ..RulepackRenderLimits::default()
    },
  )
  .expect("exact manifest limit should succeed");
  assert!(
    inspect_rulepack_inputs_with_limits(
      &manifest,
      "test rulepack",
      RulepackRenderLimits {
        max_manifest_bytes: manifest.len() - 1,
        ..RulepackRenderLimits::default()
      },
    )
    .is_err()
  );
}

#[test]
fn profile_assignment_limit_accepts_exact_and_rejects_next() {
  let manifest = manifest_with_profile_assignments();
  inspect_rulepack_inputs_with_limits(
    &manifest,
    "test rulepack",
    RulepackRenderLimits {
      max_profile_assignments: 2,
      ..RulepackRenderLimits::default()
    },
  )
  .expect("two profile assignments should meet the exact limit");
  let error = inspect_rulepack_inputs_with_limits(
    &manifest,
    "test rulepack",
    RulepackRenderLimits {
      max_profile_assignments: 1,
      ..RulepackRenderLimits::default()
    },
  )
  .expect_err("the next profile assignment must fail admission");
  assert!(error.to_string().contains("profile assignments limit"));
}

#[test]
fn local_override_and_exception_limits_are_inclusive() {
  render_rulepack_for_install_with_limits(
    &minimal_manifest(),
    "test rulepack",
    RulepackRenderOptions {
      local_overrides: vec![enabled_override()],
      ..RulepackRenderOptions::default()
    },
    RulepackRenderLimits {
      max_overrides: 1,
      ..RulepackRenderLimits::default()
    },
  )
  .expect("one override should meet the exact limit");
  let override_error = render_rulepack_for_install_with_limits(
    &minimal_manifest(),
    "test rulepack",
    RulepackRenderOptions {
      local_overrides: vec![enabled_override()],
      ..RulepackRenderOptions::default()
    },
    RulepackRenderLimits {
      max_overrides: 0,
      ..RulepackRenderLimits::default()
    },
  )
  .expect_err("the first override above zero must fail admission");
  assert!(override_error.to_string().contains("overrides limit"));

  render_rulepack_for_install_with_limits(
    &minimal_manifest(),
    "test rulepack",
    RulepackRenderOptions {
      local_exceptions: vec![login_exception()],
      ..RulepackRenderOptions::default()
    },
    RulepackRenderLimits {
      max_exceptions: 1,
      ..RulepackRenderLimits::default()
    },
  )
  .expect("one exception should meet the exact limit");
  let exception_error = render_rulepack_for_install_with_limits(
    &minimal_manifest(),
    "test rulepack",
    RulepackRenderOptions {
      local_exceptions: vec![login_exception()],
      ..RulepackRenderOptions::default()
    },
    RulepackRenderLimits {
      max_exceptions: 0,
      ..RulepackRenderLimits::default()
    },
  )
  .expect_err("the first exception above zero must fail admission");
  assert!(exception_error.to_string().contains("exceptions limit"));
}

#[test]
fn manifest_and_local_structure_limits_are_aggregate() {
  let mut local_exception = login_exception();
  local_exception.name = "local-login-exception".to_string();
  let options = RulepackRenderOptions {
    local_overrides: vec![enabled_override()],
    local_exceptions: vec![local_exception],
    ..RulepackRenderOptions::default()
  };
  let manifest = manifest_with_override_and_exception();
  render_rulepack_for_install_with_limits(
    &manifest,
    "test rulepack",
    options.clone(),
    RulepackRenderLimits {
      max_overrides: 2,
      max_exceptions: 2,
      ..RulepackRenderLimits::default()
    },
  )
  .expect("combined counts at their exact limits should pass");

  for limits in [
    RulepackRenderLimits {
      max_overrides: 1,
      ..RulepackRenderLimits::default()
    },
    RulepackRenderLimits {
      max_exceptions: 1,
      ..RulepackRenderLimits::default()
    },
  ] {
    assert!(
      render_rulepack_for_install_with_limits(&manifest, "test rulepack", options.clone(), limits,)
        .is_err()
    );
  }
}

#[test]
fn selector_work_and_local_option_byte_limits_are_inclusive() {
  let local_bytes = 512 + "demo".len();
  let options = RulepackRenderOptions {
    local_overrides: vec![enabled_override()],
    ..RulepackRenderOptions::default()
  };
  render_rulepack_for_install_with_limits(
    &minimal_manifest(),
    "test rulepack",
    options.clone(),
    RulepackRenderLimits {
      max_selector_work: 2,
      max_local_option_bytes: local_bytes,
      ..RulepackRenderLimits::default()
    },
  )
  .expect("selector work and local bytes at their exact limits should pass");

  let selector_error = render_rulepack_for_install_with_limits(
    &minimal_manifest(),
    "test rulepack",
    options.clone(),
    RulepackRenderLimits {
      max_selector_work: 1,
      ..RulepackRenderLimits::default()
    },
  )
  .expect_err("the second selector comparison must fail before application");
  assert!(
    selector_error
      .to_string()
      .contains("selector work units limit")
  );

  let byte_error = render_rulepack_for_install_with_limits(
    &minimal_manifest(),
    "test rulepack",
    options,
    RulepackRenderLimits {
      max_local_option_bytes: local_bytes - 1,
      ..RulepackRenderLimits::default()
    },
  )
  .expect_err("the next local option byte must fail admission");
  assert!(byte_error.to_string().contains("local option bytes limit"));
}

#[test]
fn local_collection_storage_counts_toward_the_byte_limit() {
  let exception = RulepackException {
    name: "e".to_string(),
    rule_ids: Vec::new(),
    rule_names: vec!["login".to_string()],
    tags: Vec::new(),
    routes: vec!["r".to_string(), "r".to_string()],
    methods: Vec::new(),
    path_prefixes: Vec::new(),
    source_cidrs: Vec::new(),
    reason: "x".to_string(),
    expires_at: None,
  };
  let options = RulepackRenderOptions {
    local_exceptions: vec![exception],
    ..RulepackRenderOptions::default()
  };
  let exact_bytes = 512 + 1 + (32 + 5) + (2 * 32 + 2) + 1;
  render_rulepack_for_install_with_limits(
    &minimal_manifest(),
    "test rulepack",
    options.clone(),
    RulepackRenderLimits {
      max_local_option_bytes: exact_bytes,
      ..RulepackRenderLimits::default()
    },
  )
  .expect("collection storage at the exact byte limit should pass");
  let error = render_rulepack_for_install_with_limits(
    &minimal_manifest(),
    "test rulepack",
    options,
    RulepackRenderLimits {
      max_local_option_bytes: exact_bytes - 1,
      ..RulepackRenderLimits::default()
    },
  )
  .expect_err("the next collection storage byte must fail admission");
  assert!(error.to_string().contains("local option bytes limit"));
}

#[test]
fn override_body_limit_is_inclusive_and_fanout_is_preflighted() {
  let body = "blocked";
  render_rulepack_for_install_with_limits(
    &manifest_with_reject_actions(1),
    "test rulepack",
    RulepackRenderOptions {
      local_overrides: vec![body_override(body)],
      ..RulepackRenderOptions::default()
    },
    RulepackRenderLimits {
      max_override_body_bytes: body.len(),
      ..RulepackRenderLimits::default()
    },
  )
  .expect("an override body at the exact limit should render");
  let body_error = render_rulepack_for_install_with_limits(
    &manifest_with_reject_actions(1),
    "test rulepack",
    RulepackRenderOptions {
      local_overrides: vec![body_override(body)],
      ..RulepackRenderOptions::default()
    },
    RulepackRenderLimits {
      max_override_body_bytes: body.len() - 1,
      ..RulepackRenderLimits::default()
    },
  )
  .expect_err("an oversized body must fail before override application");
  assert!(body_error.to_string().contains("override body bytes limit"));

  let large_body = "x".repeat(1024 * 1024);
  let fanout_error = render_rulepack_for_install_with_limits(
    &manifest_with_reject_actions(65),
    "test rulepack",
    RulepackRenderOptions {
      local_overrides: vec![body_override(&large_body)],
      ..RulepackRenderOptions::default()
    },
    RulepackRenderLimits::default(),
  )
  .expect_err("projected body fanout must fail before cloning bodies into rules");
  assert!(
    fanout_error
      .to_string()
      .contains("aggregate retained render bytes limit")
  );
}

#[test]
fn local_exception_growth_is_preflighted_against_output_budget() {
  let manifest = minimal_manifest();
  let options = RulepackRenderOptions {
    local_exceptions: vec![login_exception()],
    ..RulepackRenderOptions::default()
  };
  let mut rejected = 0usize;
  let mut accepted = RulepackRenderLimits::default().max_total_output_bytes;
  while rejected + 1 < accepted {
    let candidate = rejected + (accepted - rejected) / 2;
    if render_rulepack_for_install_with_limits(
      &manifest,
      "test rulepack",
      options.clone(),
      RulepackRenderLimits {
        max_total_output_bytes: candidate,
        ..RulepackRenderLimits::default()
      },
    )
    .is_ok()
    {
      accepted = candidate;
    } else {
      rejected = candidate;
    }
  }
  render_rulepack_for_install_with_limits(
    &manifest,
    "test rulepack",
    options.clone(),
    RulepackRenderLimits {
      max_total_output_bytes: accepted,
      ..RulepackRenderLimits::default()
    },
  )
  .expect("the exact projected local exception budget should pass");
  let error = render_rulepack_for_install_with_limits(
    &manifest,
    "test rulepack",
    options,
    RulepackRenderLimits {
      max_total_output_bytes: accepted - 1,
      ..RulepackRenderLimits::default()
    },
  )
  .expect_err("the next retained local exception byte must fail before mutation");
  assert!(
    error
      .to_string()
      .contains("aggregate retained render bytes limit")
  );
}

#[test]
fn exception_selector_budget_fails_before_template_rendering() {
  let manifest = manifest_with_override_and_exception()
    .replace("version = \"0.1.0\"", "version = \"{{missing}}\"");
  let mut local_exception = login_exception();
  local_exception.name = "local-login-exception".to_string();
  let options = RulepackRenderOptions {
    local_exceptions: vec![local_exception],
    ..RulepackRenderOptions::default()
  };
  let rendering_error = render_rulepack_for_install_with_limits(
    &manifest,
    "test rulepack",
    options.clone(),
    RulepackRenderLimits::default(),
  )
  .expect_err("the manifest must reach its later template-rendering failure with enough work");
  assert!(rendering_error.to_string().contains("unknown placeholder"));

  let error = render_rulepack_for_install_with_limits(
    &manifest,
    "test rulepack",
    options,
    RulepackRenderLimits {
      max_selector_work: 3,
      ..RulepackRenderLimits::default()
    },
  )
  .expect_err("combined exception selector work must fail before template rendering");
  assert!(error.to_string().contains("selector work units limit of 3"));
}

#[test]
fn infallible_text_rendering_returns_empty_at_default_limit() {
  let variables = BTreeMap::from([("x".to_string(), "x".repeat(1024 * 1024))]);
  assert_eq!(render_text("{{missing}}", &BTreeMap::new()), "{{missing}}");
  assert_eq!(render_text(&"{{x}}".repeat(65), &variables), "");
}

#[test]
fn inspect_and_install_reject_unsafe_referenced_paths() {
  let direct = manifest_with_unsafe_rule_path("../rules/login.oxirule.toml", false);
  assert!(inspect_rulepack_inputs(&direct, "test rulepack").is_err());
  assert!(inspect_rulepack(&direct, "test rulepack", RulepackRenderOptions::default()).is_err());
  assert!(
    render_rulepack_for_install(&direct, "test rulepack", RulepackRenderOptions::default())
      .is_err()
  );

  let templated = manifest_with_unsafe_rule_path("rules/{{segment}}.oxirule.toml", true);
  let options = RulepackRenderOptions {
    variables: BTreeMap::from([("segment".to_string(), "../../escape".to_string())]),
    ..RulepackRenderOptions::default()
  };
  assert!(inspect_rulepack(&templated, "test rulepack", options.clone()).is_err());
  assert!(render_rulepack_for_install(&templated, "test rulepack", options).is_err());
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

fn minimal_manifest() -> String {
  r#"[rulepack]
schema_version = 2
name = "demo"
version = "0.1.0"

[[rules]]
name = "login"
phase = "request"
priority = 100
content = '''
when = "true"
'''
"#
  .to_string()
}

fn manifest_with_optional_placeholder(field: &str) -> String {
  format!(
    r#"[rulepack]
schema_version = 2
name = "demo"
version = "0.1.0"
{field}

[[variables]]
name = "optional"
type = "string"
required = false

[[rules]]
name = "login"
phase = "request"
priority = 100
content = '''
when = "true"
'''
"#
  )
}

fn manifest_with_embedded_when() -> String {
  r#"[rulepack]
schema_version = 2
name = "demo"
version = "0.1.0"

[[variables]]
name = "first"
type = "string"
required = true

[[variables]]
name = "second"
type = "string"
required = true

[[rules]]
name = "login"
phase = "request"
priority = 100
content = '''
when = "Context.Value == '{{first}}'"
'''
"#
  .to_string()
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

fn manifest_with_rate_variable() -> String {
  r#"[rulepack]
schema_version = 2
name = "demo"
version = "0.1.0"

[[variables]]
name = "limit"
type = "rate"
required = true

[[rules]]
name = "login"
id = "demo.login"
tags = ["surface:login"]
phase = "request"
priority = 100
content = '''
when = "true"

[[actions]]
type = "rate_limit"
name = "login"
key = "client_ip"
rate = "{{limit}}"
burst = 5
'''
"#
  .to_string()
}

fn rate_override(rate: &str) -> RulepackOverride {
  RulepackOverride {
    selector: RulepackOverrideSelector {
      rulepack: None,
      tags: vec!["surface:login".to_string()],
      rule_id: None,
      rule_name: None,
    },
    action: Some(RulepackActionSelector {
      action_type: "rate_limit".to_string(),
      name: Some("login".to_string()),
    }),
    mode: None,
    priority: None,
    enabled: None,
    rate: Some(rate.to_string()),
    burst: None,
    status: None,
    body: None,
  }
}

fn enabled_override() -> RulepackOverride {
  RulepackOverride {
    selector: RulepackOverrideSelector {
      rulepack: Some("demo".to_string()),
      tags: Vec::new(),
      rule_id: None,
      rule_name: None,
    },
    action: None,
    mode: None,
    priority: None,
    enabled: Some(true),
    rate: None,
    burst: None,
    status: None,
    body: None,
  }
}

fn body_override(body: &str) -> RulepackOverride {
  RulepackOverride {
    selector: RulepackOverrideSelector {
      rulepack: Some("demo".to_string()),
      tags: Vec::new(),
      rule_id: None,
      rule_name: None,
    },
    action: Some(RulepackActionSelector {
      action_type: "reject".to_string(),
      name: None,
    }),
    mode: None,
    priority: None,
    enabled: None,
    rate: None,
    burst: None,
    status: None,
    body: Some(body.to_string()),
  }
}

fn login_exception() -> RulepackException {
  RulepackException {
    name: "temporary-login-exception".to_string(),
    rule_ids: Vec::new(),
    rule_names: vec!["login".to_string()],
    tags: Vec::new(),
    routes: vec!["login".to_string()],
    methods: Vec::new(),
    path_prefixes: Vec::new(),
    source_cidrs: Vec::new(),
    reason: "test".to_string(),
    expires_at: None,
  }
}

fn manifest_with_profile_assignments() -> String {
  format!(
    r#"{}
[[variables]]
name = "first"
type = "string"

[[variables]]
name = "second"
type = "string"

[[profiles]]
name = "production"

[profiles.values]
first = "one"
second = "two"
"#,
    minimal_manifest()
  )
}

fn manifest_with_override_and_exception() -> String {
  format!(
    r#"{}
[[overrides]]
enabled = true

[overrides.selector]
rulepack = "demo"

[[exceptions]]
name = "manifest-login-exception"
rule_names = ["login"]
routes = ["login"]
reason = "test"
"#,
    minimal_manifest()
  )
}

fn manifest_with_reject_actions(rule_count: usize) -> String {
  let mut manifest = r#"[rulepack]
schema_version = 2
name = "demo"
version = "0.1.0"
"#
  .to_string();
  for index in 0..rule_count {
    manifest.push_str(&format!(
      r#"
[[rules]]
name = "rule-{index}"
phase = "request"
priority = 100
content = '''
when = "true"

[[actions]]
type = "reject"
'''
"#
    ));
  }
  manifest
}

fn manifest_with_unsafe_rule_path(path: &str, declares_segment: bool) -> String {
  let variable = if declares_segment {
    r#"
[[variables]]
name = "segment"
type = "string"
required = true
"#
  } else {
    ""
  };
  format!(
    r#"[rulepack]
schema_version = 2
name = "demo"
version = "0.1.0"
{variable}
[[rules]]
name = "login"
phase = "request"
priority = 100
path = "{path}"
"#
  )
}
