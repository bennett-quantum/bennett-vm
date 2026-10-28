//! bennett-vm: the device runtime (launch 0.3, Bennett-ISA-Spec.md section 10).

/// Crate version string.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_nonempty() {
        assert!(!super::version().is_empty());
    }
}
