//! ISA 0.9 §12.3: equality classes are metadata, separate from token tuples.
use crate::{json::Json, value::{Budget, Value}};
use std::collections::BTreeMap;

fn error(message: &str) -> String {
    format!("§5.4 K91: E_ARG: {message}")
}

pub fn validate(labels: &Json, count: usize) -> Result<Vec<usize>, String> {
    let labels = labels.as_arr().filter(|a| a.len() == count)
        .ok_or_else(|| error("token_aliases arity differs from token list"))?;
    let mut next = 0;
    labels.iter().map(|v| {
        let label = v.as_usize().ok_or_else(|| error("token_aliases labels must be nonnegative integers"))?;
        if label > next {
            return Err(error("token_aliases labels must start at zero without gaps"));
        }
        if label == next { next += 1; }
        Ok(label)
    }).collect()
}

/// Bind tuple-only requests before execution. Every new class gets a fresh id.
pub fn bind(args: &mut [Value], primary: Option<Budget>, aliases: Option<&Json>) -> Result<Option<Value>, String> {
    let tokens = args.iter().filter(|v| matches!(v, Value::Token(_) | Value::TokenId(..))).count();
    let count = tokens + usize::from(primary.is_some());
    let labels = match aliases {
        Some(a) => validate(a, count)?,
        None if tokens == 0 => (0..count).collect(),
        None => return Err(error("tuple-only invocation tokens require token_aliases")),
    };
    let mut classes = BTreeMap::new();
    let mut label = labels.into_iter();
    let mut token = |budget| -> Result<Value, String> {
        let class = label.next().expect("validated partition arity");
        let (id, tuple) = classes.entry(class).or_insert_with(|| (crate::value::fresh_identity(), budget));
        if *tuple != budget { return Err(error("token_aliases equates unequal tuples")); }
        Ok(Value::TokenId(*id, budget))
    };
    let primary = primary.map(&mut token).transpose()?;
    for value in args {
        if let Value::Token(b) | Value::TokenId(_, b) = value { *value = token(*b)?; }
    }
    Ok(primary)
}

pub fn parse(raw: Option<&str>) -> Result<Option<Json>, String> {
    raw.map(|raw| {
        let j = crate::json::parse(raw)?;
        if j.to_jcs() != raw { return Err(error("token_aliases must be canonical JCS")); }
        Ok(j)
    }).transpose()
}

pub fn partition(primary: Option<&Value>, args: &[Value]) -> Result<Json, String> {
    let mut classes = BTreeMap::new();
    let mut labels = vec![];
    for value in primary.into_iter().chain(args) {
        match value {
            Value::TokenId(id, _) => {
                let next = classes.len();
                labels.push(Json::u64(*classes.entry(*id).or_insert(next) as u64));
            }
            Value::Token(_) => return Err(error("token identity is unbound")),
            _ => {}
        }
    }
    Ok(Json::Arr(labels))
}
