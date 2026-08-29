use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

mod fuzz_support;

const REGRESSION_ROOT: &str = "../tests/fixtures/fuzz-regressions";

struct Regression {
  path: &'static str,
  target: &'static str,
}

/// Add every minimized reproducer here so an unreviewed fixture fails closed
/// instead of silently entering the tree.
const REGISTERED_FIXTURES: &[Regression] = &[
  Regression {
    path: "dsl_expression/ast-json-depth-limit.dsl",
    target: "dsl_expression",
  },
  Regression {
    path: "dsl_expression/ast-json-float-roundtrip.dsl",
    target: "dsl_expression",
  },
  Regression {
    path: "dsl_expression/ast-json-non-finite-float.dsl",
    target: "dsl_expression",
  },
];

fn fixture_files(root: &Path, directory: &Path, output: &mut BTreeSet<String>) {
  for entry in std::fs::read_dir(directory).expect("regression directory should be readable") {
    let entry = entry.expect("regression entry should be readable");
    let file_type = entry
      .file_type()
      .expect("regression entry type should load");
    assert!(
      !file_type.is_symlink(),
      "fuzz regression fixtures must not be symlinks"
    );
    let path = entry.path();
    if file_type.is_dir() {
      fixture_files(root, &path, output);
    } else if path.file_name().and_then(|name| name.to_str()) != Some("README.md") {
      let relative = path
        .strip_prefix(root)
        .expect("fixture should stay below its root")
        .to_string_lossy()
        .replace('\\', "/");
      output.insert(relative);
    }
  }
}

#[test]
fn every_fuzz_regression_fixture_is_registered_and_replayed() {
  let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(REGRESSION_ROOT);
  let mut actual = BTreeSet::new();
  fixture_files(&root, &root, &mut actual);
  let registered = REGISTERED_FIXTURES
    .iter()
    .map(|fixture| fixture.path.to_string())
    .collect::<BTreeSet<_>>();
  assert_eq!(
    actual, registered,
    "add every minimized fixture to REGISTERED_FIXTURES before committing it"
  );

  for fixture in REGISTERED_FIXTURES {
    let data =
      std::fs::read(root.join(fixture.path)).expect("registered fixture should be readable");
    fuzz_support::exercise_target(fixture.target, &data);
  }
}
