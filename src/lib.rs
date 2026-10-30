//! bennett-vm: the device runtime for ISA 0.8 (Bennett-ISA-Spec.md).
//!
//! Coupled to other Bennett repositories only through the published
//! spec. The reference interpreter is a CLI oracle in tests, never a
//! code dependency.

pub mod action;
pub mod bigint;
pub mod cc;
pub mod certificate;
pub mod cliargs;
pub mod exact;
pub mod identity;
pub mod interval;
pub mod jit;
pub mod json;
pub mod launch;
pub mod object;
pub mod oracle;
pub mod qcli;
mod qvalidate;
mod rational;
pub mod sched;
mod sched_abi;
pub mod sha256;
pub mod step;
pub mod stepcli;
pub mod target;
mod typing;
pub mod validate;
pub mod value;

/// Crate version string.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Parse, validate and (unless `hash_only`) verify stored hashes.
/// This is the `bennett_load` front half used by every CLI command.
pub fn load(text: &str, hash_only: bool) -> Result<(object::Object, identity::Identities), String> {
    let obj = object::parse(text)?;
    validate::validate(&obj)?;
    if let Some(coverage) = certificate::object_document(&obj, "coverage")? {
        certificate::validate_coverage_object(&obj, &coverage)?;
    }
    let ids = identity::compute(&obj)?;
    if !hash_only {
        identity::check_hashes(&obj, &ids)?;
    }
    Ok((obj, ids))
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_nonempty() {
        assert!(!super::version().is_empty());
    }
}
