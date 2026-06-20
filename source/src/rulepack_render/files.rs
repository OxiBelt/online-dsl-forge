use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use crate::rulepack_render::error::{RenderResult, fail};
use crate::rulepack_render::types::{
  RenderedRulepackFile, RulepackDocument, RulepackGroupFile, RulepackReferencedFile,
  RulepackReferencedFileKind, RulepackRule,
};

pub trait FileResolver {
  fn resolve_file(&self, file: &RulepackReferencedFile) -> RenderResult<String>;
}

#[derive(Debug, Clone, Default)]
pub struct MemoryFileResolver {
  files: BTreeMap<String, String>,
}

impl MemoryFileResolver {
  pub fn new() -> Self {
    Self::default()
  }

  pub fn insert(&mut self, path: impl Into<String>, content: impl Into<String>) -> Option<String> {
    self.files.insert(path.into(), content.into())
  }

  pub fn with_file(mut self, path: impl Into<String>, content: impl Into<String>) -> Self {
    self.insert(path, content);
    self
  }
}

impl FileResolver for MemoryFileResolver {
  fn resolve_file(&self, file: &RulepackReferencedFile) -> RenderResult<String> {
    let key = logical_path_key(&file.path)?;
    self.files.get(&key).cloned().ok_or_else(|| {
      crate::rulepack_render::RulepackRenderError::new(format!(
        "referenced rulepack file {key} is missing"
      ))
    })
  }
}

#[derive(Debug, Clone, Default)]
pub struct BlobStore {
  blobs: BTreeMap<String, String>,
}

impl BlobStore {
  pub fn new() -> Self {
    Self::default()
  }

  pub fn insert(&mut self, id: impl Into<String>, content: impl Into<String>) -> Option<String> {
    self.blobs.insert(id.into(), content.into())
  }

  pub fn get(&self, id: &str) -> Option<&str> {
    self.blobs.get(id).map(String::as_str)
  }
}

#[derive(Debug, Clone, Default)]
pub struct BlobFileResolver {
  blobs: BlobStore,
  path_to_blob: BTreeMap<String, String>,
}

impl BlobFileResolver {
  pub fn new(blobs: BlobStore) -> Self {
    Self {
      blobs,
      path_to_blob: BTreeMap::new(),
    }
  }

  pub fn insert_mapping(
    &mut self,
    path: impl Into<String>,
    blob_id: impl Into<String>,
  ) -> Option<String> {
    self.path_to_blob.insert(path.into(), blob_id.into())
  }

  pub fn with_mapping(mut self, path: impl Into<String>, blob_id: impl Into<String>) -> Self {
    self.insert_mapping(path, blob_id);
    self
  }
}

impl FileResolver for BlobFileResolver {
  fn resolve_file(&self, file: &RulepackReferencedFile) -> RenderResult<String> {
    let key = logical_path_key(&file.path)?;
    let blob_id = self.path_to_blob.get(&key).ok_or_else(|| {
      crate::rulepack_render::RulepackRenderError::new(format!(
        "referenced rulepack file {key} has no blob mapping"
      ))
    })?;
    self.blobs.get(blob_id).map(str::to_string).ok_or_else(|| {
      crate::rulepack_render::RulepackRenderError::new(format!(
        "referenced rulepack file {key} maps to missing blob {blob_id}"
      ))
    })
  }
}

pub(crate) fn referenced_rulepack_files(
  document: &RulepackDocument,
) -> RenderResult<Vec<RulepackReferencedFile>> {
  let mut files = Vec::new();
  for rule in &document.rules {
    if let Some(path) = &rule.path {
      validate_relative_rulepack_path(
        &format!("rulepack {} rule {}", document.rulepack.name, rule.name),
        path,
        ".oxirule.toml",
      )?;
      files.push(RulepackReferencedFile {
        kind: RulepackReferencedFileKind::Rule,
        path: path.clone(),
      });
    }
  }
  for group_file in &document.group_files {
    if let Some(path) = &group_file.path {
      validate_relative_rulepack_path(
        &format!("rulepack {} group file", document.rulepack.name),
        path,
        ".oxirule-group.toml",
      )?;
      files.push(RulepackReferencedFile {
        kind: RulepackReferencedFileKind::Group,
        path: path.clone(),
      });
    }
  }
  Ok(files)
}

pub(crate) fn validate_rule_content_or_path(label: &str, rule: &RulepackRule) -> RenderResult<()> {
  validate_content_or_path(
    label,
    rule.content.as_deref(),
    rule.path.as_deref(),
    ".oxirule.toml",
  )
}

pub(crate) fn validate_group_content_or_path(
  label: &str,
  group_file: &RulepackGroupFile,
) -> RenderResult<()> {
  validate_content_or_path(
    label,
    group_file.content.as_deref(),
    group_file.path.as_deref(),
    ".oxirule-group.toml",
  )
}

pub(crate) fn embedded_or_resolved_file<R: FileResolver + ?Sized>(
  file: RulepackReferencedFile,
  embedded: Option<&str>,
  resolver: &R,
  variables: &BTreeMap<String, String>,
) -> RenderResult<RenderedRulepackFile> {
  let raw = match embedded {
    Some(content) => content.to_string(),
    None => resolver.resolve_file(&file)?,
  };
  Ok(RenderedRulepackFile {
    kind: file.kind,
    path: file.path,
    content: super::render_text(&raw, variables),
  })
}

fn validate_content_or_path(
  label: &str,
  content: Option<&str>,
  path: Option<&Path>,
  suffix: &str,
) -> RenderResult<()> {
  match (content, path) {
    (Some(_), Some(_)) => fail(format!("{label} must use either content or path, not both")),
    (None, None) => fail(format!("{label} must include content or path")),
    (Some(content), None) => {
      if content.trim().is_empty() {
        return fail(format!("{label} content must not be empty"));
      }
      Ok(())
    }
    (None, Some(path)) => validate_relative_rulepack_path(label, path, suffix),
  }
}

fn validate_relative_rulepack_path(label: &str, path: &Path, suffix: &str) -> RenderResult<()> {
  let value = logical_path_key(path)?;
  if !value.ends_with(suffix) {
    return fail(format!("{label} path must end with {suffix}"));
  }
  Ok(())
}

fn logical_path_key(path: &Path) -> RenderResult<String> {
  let Some(value) = path.to_str() else {
    return fail(format!(
      "rulepack path is not valid UTF-8: {}",
      path.display()
    ));
  };
  if value.trim().is_empty()
    || value.contains('\\')
    || value.bytes().any(|byte| byte.is_ascii_control())
  {
    return fail(format!(
      "rulepack path is not a safe relative path: {value}"
    ));
  }
  if path.is_absolute() {
    return fail(format!("rulepack path must be relative: {value}"));
  }
  let mut components = 0usize;
  for component in path.components() {
    match component {
      Component::Normal(part) if !part.is_empty() => components += 1,
      Component::CurDir | Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
        return fail(format!(
          "rulepack path is not a safe relative path: {value}"
        ));
      }
      Component::Normal(_) => {
        return fail(format!(
          "rulepack path is not a safe relative path: {value}"
        ));
      }
    }
  }
  if components == 0 || value.split('/').any(str::is_empty) {
    return fail(format!(
      "rulepack path is not a safe relative path: {value}"
    ));
  }
  Ok(value.to_string())
}

#[allow(dead_code)]
fn _pathbuf_from_key(value: &str) -> PathBuf {
  PathBuf::from(value)
}
