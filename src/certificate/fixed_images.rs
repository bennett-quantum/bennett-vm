//! ISA 0.9 section 12.2: fixed image admission, independent of replay verdicts.
use super::*;
use crate::object::{Function, Type};
fn invalid(why: &str) -> String {
    err(format!("§12.2: {why}"))
}

pub(super) fn validate(function: &Function, domain: &Json) -> Result<(), String> {
    let Some(images) = domain.get("fixed_regions") else {
        return Ok(());
    };
    let group = if function.kind == FnKind::Oracle {
        3
    } else {
        2
    };
    let mut seen = BTreeSet::new();
    for image in array(images).map_err(|_| invalid("fixed_regions must be an array"))? {
        crate::target::keys(image, &["name", "A", "W", "image", "sha256"])
            .map_err(|_| invalid("fixed image fields"))?;
        let name = string(image, "name")?;
        let region = function.lists[group]
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| invalid("unknown fixed region"))?;
        let Type::Region(address, width) = region.ty else {
            return Err(invalid("fixed binding is not a region"));
        };
        if region.role.as_deref() != Some("in")
            || field(image, "A")?.as_usize() != Some(address)
            || field(image, "W")?.as_usize() != Some(width)
        {
            return Err(invalid(
                "fixed region must match an in-role region's A and W",
            ));
        }
        if !seen.insert(name)
            || array(field(domain, "regions")?)?
                .iter()
                .any(|r| r.get("name").and_then(Json::as_str) == Some(name))
        {
            return Err(invalid("fixed region occurs more than once in the domain"));
        }
        validate_bytes(image, address, width)?;
    }
    Ok(())
}
fn validate_bytes(image: &Json, address: usize, width: usize) -> Result<(), String> {
    let text = string(image, "image")?;
    if !text
        .bytes()
        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(invalid("image must be lowercase hexadecimal"));
    }
    let bytes = crate::sha256::from_hex(text).map_err(|_| invalid("invalid image encoding"))?;
    let bits = U::from_u64(width as u64).shl(address);
    let (whole, tail) = bits.divmod(&U::from_u64(8));
    let required = whole.add(&U::from_u64(u64::from(!tail.is_zero())));
    if required != U::from_u64(bytes.len() as u64) {
        return Err(invalid("fixed image byte length differs from the region"));
    }
    let tail = tail.to_usize().unwrap();
    if tail != 0 && bytes.last().is_some_and(|byte| *byte >> tail != 0) {
        return Err(invalid("fixed image has nonzero padding bits"));
    }
    if string(image, "sha256")? != crate::sha256::sha256_hex(&bytes) {
        return Err(invalid("fixed image raw-byte SHA256 mismatch"));
    }
    Ok(())
}
