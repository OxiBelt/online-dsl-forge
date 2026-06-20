use crate::rulepack_render::error::{RenderResult, fail};
use crate::rulepack_render::types::RulepackSourceProvenance;
use crate::rulepack_render::validation::{
  validate_source_fingerprint, validate_source_sha256, validate_source_text,
};

pub(crate) fn validate_manifest_provenance(
  source: &str,
  source_url: Option<&str>,
  source_sha256: Option<&str>,
  source_openpgp_signature_url: Option<&str>,
  source_openpgp_signer_fingerprint: Option<&str>,
) -> RenderResult<()> {
  if let Some(value) = source_url {
    validate_source_text(source, "rulepack.source_url", value)?;
  }
  if let Some(value) = source_sha256 {
    validate_source_sha256(source, "rulepack.source_sha256", value)?;
  }
  if let Some(value) = source_openpgp_signature_url {
    validate_source_text(source, "rulepack.source_openpgp_signature_url", value)?;
  }
  if let Some(value) = source_openpgp_signer_fingerprint {
    validate_source_fingerprint(source, "rulepack.source_openpgp_signer_fingerprint", value)?;
  }
  Ok(())
}

pub(crate) fn set_rulepack_provenance(
  value: &mut toml::Value,
  provenance: RulepackSourceProvenance,
) -> RenderResult<()> {
  let Some(table) = value
    .get_mut("rulepack")
    .and_then(toml::Value::as_table_mut)
  else {
    return fail("rulepack manifest is missing [rulepack]");
  };
  for key in [
    "source_url",
    "source_sha256",
    "source_openpgp_signature_url",
    "source_openpgp_signer_fingerprint",
  ] {
    table.remove(key);
  }
  table.insert(
    "source_url".to_string(),
    toml::Value::String(provenance.source_url),
  );
  table.insert(
    "source_sha256".to_string(),
    toml::Value::String(provenance.source_sha256),
  );
  if let Some(value) = provenance.source_openpgp_signature_url {
    table.insert(
      "source_openpgp_signature_url".to_string(),
      toml::Value::String(value),
    );
  }
  if let Some(value) = provenance.source_openpgp_signer_fingerprint {
    table.insert(
      "source_openpgp_signer_fingerprint".to_string(),
      toml::Value::String(value),
    );
  }
  Ok(())
}
