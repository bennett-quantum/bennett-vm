//! Remaining flagged/direct row adapters; recipes own their inverse traversal.
use super::*;
type Row = (Vec<Json>, Vec<Json>);
fn alloc(name: &str, width: usize) -> Json {
    Json::obj(vec![
        ("alloc", s(name)),
        ("width", n(width)),
        ("class", s("any")),
        ("state", s("clean")),
    ])
}
#[allow(clippy::too_many_arguments)]
pub(super) fn row(
    object: &Object,
    program: &procedures::Program,
    i: &Instr,
    k: usize,
    p: usize,
    count: usize,
    fault: usize,
    widths: &BTreeMap<String, usize>,
    slots: &[Json],
    flag: Json,
    succ: Json,
    env: &mut Bindings,
) -> Result<Option<Row>, String> {
    let mut effect = vec![];
    let next = match i.op.as_str() {
        "lift.udiv" | "lift.urem" | "lift.sdiv" | "lift.srem" => {
            let width = widths[&i.lhs[0]];
            let slot = range(
                "$frame",
                field(&slots[0], "from")?.as_usize().unwrap(),
                width,
            );
            effect.push(invoke(
                &i.op,
                vec![
                    n(width),
                    n(fault),
                    s(&i.operands[0]),
                    s(&i.operands[1]),
                    slot,
                    s(&i.lhs[0]),
                    flag.clone(),
                    s("$oracle.faulted"),
                    s("$oracle.code"),
                ],
            ));
            fault_next(p, count, k + 1, flag)
        }
        "lift.faultif" => {
            let cause = integer(
                crate::bigint::BigUint::parse_int(&i.operands[1])?.to_decimal(),
                env,
            )?;
            effect.push(invoke(
                "lift.faultif",
                vec![
                    n(fault),
                    cause,
                    s(&i.operands[0]),
                    flag.clone(),
                    s("$oracle.faulted"),
                    s("$oracle.code"),
                ],
            ));
            fault_next(p, count, k + 1, flag)
        }
        "fault" => {
            let cause = crate::bigint::BigUint::parse_int(&i.operands[0])?;
            let cause = if cause.is_zero() {
                "4".into()
            } else {
                cause.to_decimal()
            };
            effect.extend([
                flip(flag),
                flip(s("$oracle.faulted")),
                literal(fault, integer(cause, env)?, s("$oracle.code")),
            ]);
            vec![literal(p, n(count), s("$next"))]
        }
        "qjumpr" => {
            let width = widths[&i.operands[0]];
            let wide = width.max(p) + 1;
            let (first, end) = program.range(k);
            let setup = vec![
                alloc("$indirect.a", wide),
                alloc("$indirect.lo", wide),
                alloc("$indirect.hi", wide),
                xor(width, s(&i.operands[0]), range("$indirect.a", 0, width)),
                literal(wide, n(first), s("$indirect.lo")),
                literal(wide, n(end), s("$indirect.hi")),
            ];
            effect.extend(setup.clone());
            effect.extend([
                invoke(
                    "bind.value.icmp.ult",
                    vec![n(wide), s("$indirect.a"), s("$indirect.lo"), flag.clone()],
                ),
                invoke(
                    "bind.value.icmp.ult",
                    vec![n(wide), s("$indirect.a"), s("$indirect.hi"), flag.clone()],
                ),
                flip(flag.clone()),
            ]);
            // Only the low P bits can contribute to a valid canonical index.
            effect.extend([
                flip(flag.clone()),
                under(
                    flag.clone(),
                    vec![xor(
                        width.min(p),
                        range(&i.operands[0], 0, width.min(p)),
                        range(
                            "$frame",
                            field(&succ, "from")?.as_usize().unwrap(),
                            width.min(p),
                        ),
                    )],
                ),
                flip(flag.clone()),
            ]);
            effect.push(inv(setup));
            effect.extend([under(
                flag.clone(),
                vec![
                    flip(s("$oracle.faulted")),
                    literal(fault, n(4), s("$oracle.code")),
                ],
            )]);
            vec![
                under(flag.clone(), vec![literal(p, n(count), s("$next"))]),
                flip(flag.clone()),
                under(flag.clone(), vec![xor(p, succ, s("$next"))]),
                flip(flag),
            ]
        }
        "lift.perm_call" => {
            let name = i.operands[0].trim_start_matches('@');
            let hash = env.registry.as_ref().unwrap().identities.closure[name]
                .as_ref()
                .ok_or_else(|| err("unresolved permutation action"))?;
            let actuals = i.operands[1..]
                .iter()
                .map(|list| crate::validate::list_operand(list))
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .map(s)
                .collect();
            let _ = object
                .function(name)
                .ok_or_else(|| err("missing permutation source"))?;
            effect.push(invoke("lift.perm_call", vec![s(hash), Json::Arr(actuals)]));
            vec![literal(p, n(k + 1), s("$next"))]
        }
        _ => return Ok(None),
    };
    Ok(Some((effect, next)))
}
