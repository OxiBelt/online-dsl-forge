use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

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
    "FUZZ_ASAN_NIGHTLY=\"nightly-2026-08-04\"",
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
