//! K12 aggregation/accessor boundary. These rows describe the supplied scheduled
//! events; only complete source lowering can produce a launch estimate certificate.
use crate::{
    json::Json,
    target::{Event, LiveInterval, Rational as R, Target},
};
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceRow {
    pub decimal: String,
    pub numerator: String,
    pub denominator: String,
}
pub fn escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
fn rational(s: &str) -> Result<R, String> {
    if let Some((a, b)) = s.split_once('/') {
        a.parse::<R>()?.div(&b.parse()?)
    } else {
        s.parse()
    }
}
pub fn resource_rows(
    target: &Target,
    events: &[Event],
    live: &[LiveInterval],
) -> Result<BTreeMap<String, ResourceRow>, String> {
    let account = target.account(events, live)?;
    let mut rows = BTreeMap::new();
    fn add(out: &mut BTreeMap<String, ResourceRow>, name: String, v: &Json) -> Result<(), String> {
        let q = rational(v.as_str().ok_or("§10.1: expected exact resource")?)?;
        let (numerator, denominator) = q.fraction();
        if out
            .insert(
                name,
                ResourceRow {
                    decimal: q.upper_decimal34()?,
                    numerator,
                    denominator,
                },
            )
            .is_some()
        {
            return Err("§10.1: duplicate encoded resource".into());
        }
        Ok(())
    }
    for (name, value) in account.as_obj().ok_or("§10.1: expected resource map")? {
        if let Json::Obj(classes) = value {
            for (class, v) in classes {
                add(&mut rows, format!("{}[{}]", escape(name), escape(class)), v)?;
            }
        } else {
            add(&mut rows, escape(name), value)?;
        }
    }
    Ok(rows)
}
