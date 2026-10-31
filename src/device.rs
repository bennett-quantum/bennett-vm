//! ISA 0.9 K10: contiguous complete-sector block-image decoder. Construct the
//! entire initial map before publishing device state. Zero sectors stay present.
use std::collections::BTreeMap;
pub fn block_image(hex: &str) -> Result<BTreeMap<u64, Vec<u8>>, String> {
    validate_block_image(hex)?;
    let bytes = crate::sha256::from_hex(hex)?;
    let mut sectors = BTreeMap::new();
    for (s, sector) in bytes.as_chunks::<512>().0.iter().enumerate() {
        let s = u64::try_from(s).map_err(|_| "§7.3: host limit: block sector number")?;
        sectors.insert(s, sector.to_vec());
    }
    Ok(sectors)
}

pub(crate) fn validate_block_image(hex: &str) -> Result<(), String> {
    if !hex.len().is_multiple_of(1024) {
        return Err("§7.3: block image must have complete 512-byte sectors".into());
    }
    if !hex
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("§7.3: block image requires lowercase hexadecimal byte pairs".into());
    }
    Ok(())
}
