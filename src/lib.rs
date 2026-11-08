//! bennett-vm: the device runtime for ISA 0.8 (Bennett-ISA-Spec.md).
//!
//! Coupled to other Bennett repositories only through the published
//! spec. The reference interpreter is a CLI oracle in tests, never a
//! code dependency.

pub mod action;
pub mod bigint;
mod call_matching;
pub mod cc;
pub mod certificate;
mod classical_memory;
mod cleanup_discipline;
pub mod cliargs;
mod coherent_typing;
pub mod device;
pub mod estimate;
pub mod exact;
pub mod identity;
pub mod instances;
pub mod interval;
pub mod invocation;
pub mod jit;
pub mod json;
pub mod launch;
mod launch_locks;
mod lifted_typing;
pub mod link;
pub mod object;
pub mod oracle;
pub mod qcli;
mod qreg_typing;
mod qvalidate;
mod rational;
pub mod ref_library;
pub mod sched;
mod sched_abi;
pub mod sha256;
pub mod step;
pub mod stepcli;
pub mod target;
pub mod token_aliases;
mod typing;
pub mod validate;
pub mod value;

/// Crate version string.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn require_linked(obj: &object::Object) -> Result<(), String> {
    if !obj.imports.is_empty() {
        return Err("§9.3: object has unresolved imports; link before execution".into());
    }
    Ok(())
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
        if obj.isa == "0.9" {
            certificate::validate_metadata(&obj, &ids)?;
        }
        check_oracle_pairs(&obj)?;
        identity::check_hashes(&obj, &ids)?;
        if obj.isa == "0.9"
            && let Some(doc) = sched::object_schedule(&obj)?
        {
            sched::validate_embedded_support(&doc, &obj, &ids)?;
        }
    }
    Ok((obj, ids))
}

/// §5.5 at load. `load(_, true)` omits it because it also re-reads an object already
/// loaded when binding addresses, where `oracle_cleanup_pairs` decides the pairing
/// value-precisely; the `hash` command runs it separately.
pub fn check_oracle_pairs(obj: &object::Object) -> Result<(), String> {
    for f in &obj.functions {
        if matches!(f.kind, object::FnKind::Oracle | object::FnKind::Lproc) {
            call_matching::function(f)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_nonempty() {
        assert!(!super::version().is_empty());
    }
}

mod history;
mod persistent;

// Private work probes need cfg(test); integration libraries do not receive it.
#[cfg(test)]
extern crate self as bennett_vm;
#[cfg(test)]
mod execution_work;
#[cfg(test)]
#[path = "../tests/support/pins.rs"]
mod pins;
#[cfg(test)]
#[path = "../tests/step_scaling.rs"]
mod step_scaling;

#[cfg(test)]
mod history_work;
