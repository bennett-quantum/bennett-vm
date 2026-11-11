//! Load-time contract checks do not claim a target-dependent proof verdict.
use super::*;
use crate::{identity::Identities, object::Type, target::field as required};

fn region_name<'a>(ids: &'a Identities, hash: &str) -> Result<&'a str, String> {
    ids.closure
        .iter()
        .find(|(_, h)| h.as_deref() == Some(hash))
        .map(|(name, _)| name.as_str())
        .ok_or_else(|| err("unknown certificate closure"))
}

fn claim_shape(obj: &Object, name: &str, claim: &Json) -> Result<(), String> {
    if claim.get("kind").and_then(Json::as_str) == Some("structural")
        && claim.get("rule").and_then(Json::as_str) == Some("adjoint_table")
        && field(claim, "witness")?.as_obj().is_none_or(|m| !m.is_empty()) {
        return Err(err("adjoint_table witness must be {}"));
    }
    if matches!(string(claim, "kind")?, "exhaustive" | "sampled") {
        field(claim, "expected")?;
        let mut domain = field(claim, "domain")?.clone();
        let controls = match claim.get("subject") {
            Some(subject) => {
                let Json::Num(count) = field(subject, "inherited_controls")? else {
                    return Err(err("invalid inherited_controls"));
                };
                count
                    .parse::<usize>()
                    .map_err(|_| err("invalid inherited_controls"))?
            }
            None => 0,
        };
        if controls != 0 {
            let Json::Obj(ref mut fields) = domain else {
                return Err(err("invalid domain"));
            };
            let bundles = array(field(claim.get("domain").unwrap(), "bundles")?)?;
            if bundles.len() < controls {
                return Err(err("missing inherited controls"));
            }
            fields.insert("bundles".into(), Json::Arr(bundles[controls..].to_vec()));
        }
        domain_roles(obj, name, &domain, true)?;
    }
    if let Some(invocation) = claim.get("invocation") {
        if invocation.get("args").and_then(Json::as_str) == Some("all") {
            let function = obj.function(name).ok_or_else(|| err("unknown claim region"))?;
            let group = if function.kind == FnKind::Oracle { 4 } else { 3 };
            let token_args = function.lists[group].iter().any(|p| p.ty == Type::Token);
            if (token_args || invocation.get("token_aliases").is_some())
                && invocation.get("token_aliases").and_then(Json::as_str) != Some("all") {
                return Err(err("K91: universal invocation must quantify token_aliases"));
            }
        } else {
            crate::invocation::resolve(obj, name, invocation)?;
        }
    }
    Ok(())
}

pub(crate) fn validate_metadata(obj: &Object, ids: &Identities) -> Result<(), String> {
    let cert = object_document(obj, "certificate")?;
    let schedule = crate::sched::object_schedule(obj)?;
    if let Some(doc) = &cert {
        if string(field(doc, "bindings")?, "object_hash")? != ids.object_hash {
            return Err(err("certificate object_hash mismatch"));
        }
        for (hash, region) in field(doc, "regions")?
            .as_obj()
            .ok_or_else(|| err("regions must be an object"))?
        {
            let name = region_name(ids, hash)?;
            for claim in array(field(region, "claims")?)? {
                claim_shape(obj, name, claim)?;
            }
        }
        let binding = field(doc, "bindings")?;
        if let Some(invocation) = binding.get("invocation") {
            let name = region_name(ids, string(binding, "closure_hash")?)?;
            crate::invocation::resolve(obj, name, invocation)?;
        }
    }
    if let Some(doc) = &schedule {
        let binding = field(doc, "bindings")?;
        let name = string(binding, "region")?;
        let f = obj
            .function(name)
            .ok_or_else(|| err("unknown schedule region"))?;
        let group = if f.kind == FnKind::Oracle { 4 } else { 3 };
        let count = f.lists[group]
            .iter()
            .filter(|p| p.ty == Type::Token)
            .count()
            + usize::from(f.kind == FnKind::Oracle);
        if let Some(aliases) = binding.get("token_aliases") {
            crate::token_aliases::validate(aliases, count)?;
        } else if count > usize::from(f.kind == FnKind::Oracle) {
            return Err(err("K91: token-bearing schedule requires token_aliases"));
        }
        if let Some(invocation) = cert
            .as_ref()
            .and_then(|c| c.get("bindings"))
            .and_then(|b| b.get("invocation"))
        {
            let labels = invocation.get("token_aliases");
            if labels != binding.get("token_aliases")
                && (labels.is_some() || binding.get("token_aliases").is_some())
            {
                return Err(err(
                    "K91: certificate invocation token_aliases differs from schedule",
                ));
            }
            if crate::invocation::params_hash(required(invocation, "args")?)
                != string(binding, "params_hash")?
            {
                return Err(err(
                    "K91: certificate invocation params_hash differs from schedule",
                ));
            }
        }
        crate::sched::validate_token_cleanup_ties(doc, obj, ids)?;
    }
    Ok(())
}
