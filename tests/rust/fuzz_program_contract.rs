use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::Value as JsonValue;

const EXPECTED_TARGETS: &[&str] = &["dsl_expression", "expression_pipeline", "rulepack_render"];

fn repo_root() -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    .parent()
    .expect("source crate should live below the repository root")
    .to_path_buf()
}

fn read_repo_file(path: &str) -> String {
  fs::read_to_string(repo_root().join(path))
    .unwrap_or_else(|error| panic!("{path} should be readable: {error}"))
}

fn parse_repo_toml(path: &str) -> toml::Value {
  toml::from_str(&read_repo_file(path))
    .unwrap_or_else(|error| panic!("{path} should contain valid TOML: {error}"))
}

fn parse_workflow_yaml(path: &str) -> JsonValue {
  serde_saphyr::from_str(&read_repo_file(path))
    .unwrap_or_else(|error| panic!("{path} should contain valid YAML: {error}"))
}

fn json_string_array(value: &JsonValue, context: &str) -> Vec<String> {
  value
    .as_array()
    .unwrap_or_else(|| panic!("{context} should be an array"))
    .iter()
    .map(|entry| {
      entry
        .as_str()
        .unwrap_or_else(|| panic!("{context} entries should be strings"))
        .to_string()
    })
    .collect()
}

fn workflow_steps<'a>(job: &'a JsonValue, context: &str) -> &'a [JsonValue] {
  job["steps"]
    .as_array()
    .unwrap_or_else(|| panic!("{context} should define steps"))
}

fn workflow_step<'a>(steps: &'a [JsonValue], name: &str) -> &'a JsonValue {
  steps
    .iter()
    .find(|step| step["name"].as_str() == Some(name))
    .unwrap_or_else(|| panic!("workflow should define step `{name}`"))
}

fn target_tables() -> Vec<toml::Table> {
  parse_repo_toml("fuzz/targets.toml")
    .get("target")
    .and_then(toml::Value::as_array)
    .expect("fuzz/targets.toml should define [[target]] entries")
    .iter()
    .map(|value| {
      value
        .as_table()
        .expect("every fuzz target should be a table")
        .clone()
    })
    .collect()
}

fn required_string(table: &toml::Table, field: &str, context: &str) -> String {
  table
    .get(field)
    .and_then(toml::Value::as_str)
    .filter(|value| !value.trim().is_empty())
    .unwrap_or_else(|| panic!("{context} must define nonempty `{field}`"))
    .to_string()
}

fn required_string_array(table: &toml::Table, field: &str, context: &str) -> Vec<String> {
  let entries = table
    .get(field)
    .and_then(toml::Value::as_array)
    .unwrap_or_else(|| panic!("{context} must define `{field}` as an array"));
  assert!(!entries.is_empty(), "{context} `{field}` must not be empty");
  entries
    .iter()
    .map(|entry| {
      entry
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| panic!("{context} `{field}` entries must be nonempty strings"))
        .to_string()
    })
    .collect()
}

fn assert_safe_relative_path(path: &str, context: &str) {
  let path = Path::new(path);
  assert!(!path.is_absolute(), "{context} must be repository relative");
  assert!(
    path
      .components()
      .all(|component| matches!(component, Component::Normal(_))),
    "{context} must not contain traversal or special path components"
  );
}

fn regular_files(directory: &Path) -> Vec<PathBuf> {
  let mut files = fs::read_dir(directory)
    .unwrap_or_else(|error| panic!("{} should be readable: {error}", directory.display()))
    .map(|entry| entry.expect("directory entry should be readable").path())
    .filter(|path| path.is_file())
    .collect::<Vec<_>>();
  files.sort();
  files
}

#[test]
fn fuzz_catalog_and_harnesses_are_consistent_and_bounded() {
  let catalog = parse_repo_toml("fuzz/targets.toml");
  assert_eq!(
    catalog.get("version").and_then(toml::Value::as_integer),
    Some(1)
  );
  let program = catalog
    .get("program")
    .and_then(toml::Value::as_table)
    .expect("fuzz catalog should define [program]");
  let expected_program_values = [
    ("max_seed_files_per_target", 128),
    ("max_seed_bytes_per_target", 524_288),
    ("max_working_corpus_files_per_target", 16_384),
    ("max_cached_corpus_files_per_target", 8_192),
    ("max_cached_corpus_bytes_per_target", 67_108_864),
    ("max_artifact_files_per_target", 8),
    ("smoke_runs", 256),
    ("campaign_seconds", 900),
    ("input_timeout_seconds", 10),
    ("rss_limit_mb", 3_072),
    ("allocation_limit_mb", 512),
  ];
  for (field, expected) in expected_program_values {
    assert_eq!(
      program.get(field).and_then(toml::Value::as_integer),
      Some(expected),
      "unexpected fuzz program `{field}`"
    );
  }
  assert_eq!(
    program
      .get("smoke_leak_detection")
      .and_then(toml::Value::as_bool),
    Some(false)
  );
  assert_eq!(
    program
      .get("campaign_leak_detection")
      .and_then(toml::Value::as_bool),
    Some(true)
  );

  let mut actual_targets = BTreeSet::new();
  for table in target_tables() {
    let name = required_string(&table, "name", "fuzz target");
    assert!(
      actual_targets.insert(name.clone()),
      "duplicate fuzz target {name}"
    );
    let context = format!("fuzz target {name}");
    for field in ["owner", "input_contract", "leak_policy"] {
      required_string(&table, field, &context);
    }
    for field in ["invariants", "unsupported_states", "coverage_landmarks"] {
      required_string_array(&table, field, &context);
    }
    let max_input_bytes = table
      .get("max_input_bytes")
      .and_then(toml::Value::as_integer)
      .unwrap_or_else(|| panic!("{context} must define max_input_bytes"));
    assert!(
      (1..=65_536).contains(&max_input_bytes),
      "{context} input limit must be positive and at most 64 KiB"
    );

    let seed_dir = required_string(&table, "seed_dir", &context);
    assert_eq!(seed_dir, format!("fuzz/seeds/{name}"));
    assert_safe_relative_path(&seed_dir, "seed directory");
    let seed_files = regular_files(&repo_root().join(&seed_dir));
    assert!(!seed_files.is_empty(), "{context} must have reviewed seeds");
    assert!(
      seed_files.len() <= 128,
      "{context} has too many reviewed seeds"
    );
    let seed_bytes = seed_files
      .iter()
      .map(|path| fs::metadata(path).expect("seed metadata should load").len())
      .sum::<u64>();
    assert!(
      seed_bytes <= 524_288,
      "{context} reviewed seeds are too large"
    );
    for seed in seed_files {
      let metadata = fs::symlink_metadata(&seed).expect("seed metadata should load");
      assert!(
        !metadata.file_type().is_symlink(),
        "seeds must not be symlinks"
      );
      assert!(
        metadata.len() <= u64::try_from(max_input_bytes).expect("input limit should fit u64"),
        "{} exceeds its target input limit",
        seed.display()
      );
    }

    let dictionary = required_string(&table, "dictionary", &context);
    assert_safe_relative_path(&dictionary, "dictionary");
    assert!(dictionary.starts_with("fuzz/dictionaries/"));
    let dictionary_metadata =
      fs::symlink_metadata(repo_root().join(&dictionary)).expect("dictionary metadata should load");
    assert!(dictionary_metadata.is_file() && !dictionary_metadata.file_type().is_symlink());
    assert!((1..=65_536).contains(&dictionary_metadata.len()));

    let regression_path = required_string(&table, "regression_path", &context);
    assert_eq!(
      regression_path,
      format!("tests/fixtures/fuzz-regressions/{name}")
    );
    assert_safe_relative_path(&regression_path, "regression path");
    assert!(repo_root().join(regression_path).is_dir());

    for landmark in required_string_array(&table, "coverage_landmarks", &context) {
      let (path, symbol) = landmark
        .split_once(':')
        .unwrap_or_else(|| panic!("invalid coverage landmark {landmark}"));
      assert_safe_relative_path(path, "coverage landmark path");
      assert!(
        !symbol.is_empty(),
        "coverage landmark symbol must not be empty"
      );
      assert!(
        repo_root().join(path).is_file(),
        "coverage landmark source is missing"
      );
    }

    assert!(
      repo_root()
        .join(format!("fuzz/fuzz_targets/{name}.rs"))
        .is_file(),
      "{context} harness is missing"
    );
  }

  assert_eq!(
    actual_targets,
    EXPECTED_TARGETS
      .iter()
      .map(|target| (*target).to_string())
      .collect::<BTreeSet<_>>()
  );

  let fuzz_manifest = parse_repo_toml("fuzz/Cargo.toml");
  let manifest_targets = fuzz_manifest
    .get("bin")
    .and_then(toml::Value::as_array)
    .expect("fuzz manifest should define binaries")
    .iter()
    .map(|entry| {
      entry
        .get("name")
        .and_then(toml::Value::as_str)
        .expect("fuzz binary should have a name")
        .to_string()
    })
    .collect::<BTreeSet<_>>();
  assert_eq!(manifest_targets, actual_targets);

  let workspace = parse_repo_toml("Cargo.toml");
  let members = workspace["workspace"]["members"]
    .as_array()
    .expect("workspace members should be an array");
  assert!(members.iter().any(|member| member.as_str() == Some("fuzz")));
  assert_eq!(
    workspace["workspace"]["default-members"]
      .as_array()
      .expect("default members should be an array")
      .iter()
      .filter_map(toml::Value::as_str)
      .collect::<Vec<_>>(),
    vec!["source"]
  );
}

#[test]
fn fuzz_runner_matches_the_catalog_security_contract() {
  let runner = read_repo_file("tests/scripts/run-fuzz-target.sh");
  for expected in [
    "set -Eeuo pipefail",
    "umask 077",
    "FUZZ_STABLE_TOOLCHAIN=\"1.98.0\"",
    "FUZZ_ASAN_NIGHTLY=\"nightly-2026-08-24\"",
    "CARGO_FUZZ_VERSION=\"0.13.2\"",
    "MAX_CACHED_CORPUS_FILES=8192",
    "MAX_CORPUS_BYTES=67108864",
    "MAX_ARTIFACT_FILES=8",
    "FUZZ_TIMEOUT_SECONDS=10",
    "FUZZ_RSS_LIMIT_MB=3072",
    "FUZZ_MALLOC_LIMIT_MB=512",
    "-runs=256",
    "-max_total_time=$duration_seconds",
    "assert_no_symlinks",
    "cargo \"+$FUZZ_ASAN_NIGHTLY\" fuzz cmin",
    "cargo \"+$FUZZ_ASAN_NIGHTLY\" fuzz coverage",
    "cargo \"+$FUZZ_ASAN_NIGHTLY\" fuzz tmin",
  ] {
    assert!(
      runner.contains(expected),
      "fuzz runner must contain {expected}"
    );
  }

  let minimize = runner
    .split_once("  minimize)\n")
    .expect("fuzz runner should define minimize mode")
    .1
    .split_once("\n  report)")
    .expect("minimize mode should end before report mode")
    .0;
  for expected in [
    "head -z -n \"$MAX_ARTIFACT_FILES\"",
    "timeout --signal=TERM --kill-after=15s 300s",
    "--sanitizer address -r 255",
    "\"-timeout=$FUZZ_TIMEOUT_SECONDS\"",
    "\"-rss_limit_mb=$FUZZ_RSS_LIMIT_MB\"",
    "\"-malloc_limit_mb=$FUZZ_MALLOC_LIMIT_MB\"",
    "-detect_leaks=1",
    "\"-artifact_prefix=$artifact_dir/\"",
    "Minimization failed or timed out; retaining raw input",
  ] {
    assert!(
      minimize.contains(expected),
      "fuzz minimization must contain {expected}"
    );
  }
  assert!(
    !minimize.contains("-max_len="),
    "fuzz minimization must not pass libFuzzer's incompatible -max_len option"
  );

  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(repo_root().join("tests/scripts/run-fuzz-target.sh"))
      .expect("runner metadata should load")
      .permissions()
      .mode();
    assert_ne!(mode & 0o111, 0, "fuzz runner must be executable");
  }
}

#[test]
fn fuzz_workflows_cover_every_target_and_profile_with_pinned_actions() {
  let expected_targets = EXPECTED_TARGETS
    .iter()
    .map(|target| (*target).to_string())
    .collect::<Vec<_>>();

  let checks = parse_workflow_yaml(".github/workflows/check-online-dsl-forge.yml");
  let smoke = &checks["jobs"]["fuzz-smoke"];
  assert_eq!(smoke["runs-on"].as_str(), Some("ubuntu-latest"));
  assert_eq!(smoke["timeout-minutes"].as_u64(), Some(30));
  assert_eq!(smoke["permissions"]["contents"].as_str(), Some("read"));
  assert_eq!(smoke["strategy"]["fail-fast"].as_bool(), Some(false));
  assert_eq!(
    json_string_array(
      &smoke["strategy"]["matrix"]["fuzz_target"],
      "smoke fuzz target matrix"
    ),
    expected_targets
  );
  let profiles = smoke["strategy"]["matrix"]["fuzz_profile"]
    .as_array()
    .expect("smoke workflow should define a fuzz profile matrix");
  assert_eq!(profiles.len(), 2);
  assert!(profiles.iter().any(|profile| {
    profile["name"].as_str() == Some("stable") && profile["toolchain"].as_str() == Some("1.98.0")
  }));
  assert!(profiles.iter().any(|profile| {
    profile["name"].as_str() == Some("asan")
      && profile["toolchain"].as_str() == Some("nightly-2026-08-24")
  }));
  let smoke_steps = workflow_steps(smoke, "smoke fuzz job");
  let smoke_run = workflow_step(smoke_steps, "Run bounded smoke target");
  assert_eq!(
    smoke_run["run"].as_str(),
    Some("tests/scripts/run-fuzz-target.sh smoke ${{ matrix.fuzz_target }}")
  );
  assert_eq!(
    smoke_run["env"]["ONLINE_DSL_FORGE_FUZZ_PROFILE"].as_str(),
    Some("${{ matrix.fuzz_profile.name }}")
  );
  let smoke_upload = workflow_step(smoke_steps, "Upload fuzz failure evidence");
  assert_eq!(smoke_upload["if"].as_str(), Some("failure()"));
  assert_eq!(
    smoke_upload["uses"].as_str(),
    Some("actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a")
  );
  assert_eq!(smoke_upload["with"]["retention-days"].as_u64(), Some(90));

  let sustained = parse_workflow_yaml(".github/workflows/fuzz-sustained.yml");
  assert_eq!(
    sustained["on"]["schedule"][0]["cron"].as_str(),
    Some("17 3 * * *")
  );
  assert!(sustained["on"].get("workflow_dispatch").is_some());
  assert_eq!(sustained["permissions"]["contents"].as_str(), Some("read"));
  assert_eq!(
    sustained["concurrency"]["cancel-in-progress"].as_bool(),
    Some(false)
  );
  let campaign = &sustained["jobs"]["fuzz-sustained"];
  assert_eq!(
    campaign["if"].as_str(),
    Some("github.ref_name == github.event.repository.default_branch")
  );
  assert_eq!(campaign["timeout-minutes"].as_u64(), Some(60));
  assert_eq!(campaign["permissions"]["contents"].as_str(), Some("read"));
  assert_eq!(campaign["strategy"]["fail-fast"].as_bool(), Some(false));
  assert_eq!(
    json_string_array(
      &campaign["strategy"]["matrix"]["fuzz_target"],
      "sustained fuzz target matrix"
    ),
    expected_targets
  );

  let campaign_steps = workflow_steps(campaign, "sustained fuzz job");
  let restore = workflow_step(campaign_steps, "Restore bounded target corpus");
  let save = workflow_step(campaign_steps, "Save minimized target corpus");
  assert_eq!(
    restore["uses"].as_str(),
    Some("actions/cache/restore@55cc8345863c7cc4c66a329aec7e433d2d1c52a9")
  );
  assert_eq!(
    save["uses"].as_str(),
    Some("actions/cache/save@55cc8345863c7cc4c66a329aec7e433d2d1c52a9")
  );
  assert_eq!(
    restore["with"]["path"].as_str(),
    Some("${{ runner.temp }}/online-dsl-forge-fuzz-corpus/${{ matrix.fuzz_target }}")
  );
  assert_eq!(
    save["if"].as_str(),
    Some("steps.campaign.outcome == 'success' && steps.cmin.outcome == 'success'")
  );
  let campaign_run = workflow_step(campaign_steps, "Run fifteen-minute campaign");
  assert_eq!(campaign_run["continue-on-error"].as_bool(), Some(true));
  assert_eq!(
    campaign_run["run"].as_str(),
    Some("tests/scripts/run-fuzz-target.sh campaign ${{ matrix.fuzz_target }} 900")
  );
  let coverage = workflow_step(campaign_steps, "Generate source coverage");
  assert_eq!(
    coverage["if"].as_str(),
    Some("always() && steps.cmin.outcome == 'success'")
  );
  assert_eq!(coverage["continue-on-error"].as_bool(), Some(true));

  let coverage_upload = workflow_step(campaign_steps, "Upload coverage evidence");
  let corpus_upload = workflow_step(campaign_steps, "Upload reviewed corpus candidate");
  let failure_upload = workflow_step(campaign_steps, "Upload failure evidence");
  for upload in [coverage_upload, corpus_upload, failure_upload] {
    assert_eq!(
      upload["uses"].as_str(),
      Some("actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a")
    );
  }
  assert_eq!(coverage_upload["with"]["retention-days"].as_u64(), Some(30));
  assert_eq!(corpus_upload["with"]["retention-days"].as_u64(), Some(30));
  assert_eq!(failure_upload["with"]["retention-days"].as_u64(), Some(90));
  assert!(
    workflow_step(campaign_steps, "Propagate campaign and evidence failures")["run"]
      .as_str()
      .is_some_and(|script| script.contains("exit \"$failed\""))
  );
}
