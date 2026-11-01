//! Source-built constructor adapter. Raw catalog descriptors never enter here.
use super::*;
use crate::object::{FnKind, Instr, Object, Type};
#[path = "oracle_direct.rs"]
mod direct;
#[path = "oracle_memory.rs"]
mod memory;
#[path = "oracle_procedures.rs"]
mod procedures;
fn s(x: impl Into<String>) -> Json {
    Json::Str(x.into())
}
fn n(x: usize) -> Json {
    Json::u64(x as u64)
}
fn invoke(kind: &str, args: Vec<Json>) -> Json {
    Json::obj(vec![("invoke", s(kind)), ("args", Json::Arr(args))])
}
fn inv(body: Vec<Json>) -> Json {
    Json::obj(vec![
        ("inverse", Json::Bool(true)),
        ("body", Json::Arr(body)),
    ])
}
fn under(control: Json, body: Vec<Json>) -> Json {
    Json::obj(vec![("under", control), ("body", Json::Arr(body))])
}
fn flip(w: Json) -> Json {
    Json::obj(vec![("gate", s("x")), ("wires", Json::Arr(vec![w]))])
}
fn range(name: &str, from: usize, width: usize) -> Json {
    Json::obj(vec![
        ("range", s(name)),
        ("from", n(from)),
        ("to", n(from + width)),
    ])
}
fn xor(width: usize, a: Json, b: Json) -> Json {
    invoke("bind.qxor", vec![n(width), a, b])
}
fn literal(width: usize, value: Json, target: Json) -> Json {
    invoke("value.literal", vec![n(width), value, target])
}
fn integer(value: String, env: &mut Bindings) -> Result<Json, String> {
    let name = format!("$oracle.integer{}", env.values.len());
    env.values
        .insert(name.clone(), Bound::Integer(value.parse::<R>()?));
    Ok(s(name))
}
fn staged(i: &Instr) -> String {
    err(format!(
        "unsupported admitted source oracle row {} line {} has no constructor adapter",
        i.op, i.line
    ))
}
impl Catalog {
    /// Instantiate source-derived oracle rows and support. The context is K77's
    /// explicit record, separate from declared args; this is not raw descriptor
    /// admission or a realised schedule. Remaining row adapters fail explicitly.
    #[allow(clippy::too_many_arguments)]
    pub fn lower_oracle_with_cleanup(
        &self,
        object: &Object,
        name: &str,
        params: &Json,
        oracle_invocation: &Json,
        inherited_controls: usize,
        entries: &[CleanupEntry],
    ) -> Result<ActionExpansion, String> {
        let mut env = Bindings::new();
        env.link(object)?;
        let registry = env.registry.clone().unwrap();
        let object = &registry.object;
        let f = object
            .function(name)
            .ok_or_else(|| err("unresolved source oracle"))?;
        if f.kind != FnKind::Oracle {
            return Err(err("source is not an oracle"));
        }
        let types = f.lists[4].iter().map(|p| p.ty.clone()).collect::<Vec<_>>();
        let values = crate::invocation::decode_args(params, &types, object.ptr_width)?;
        for value in values {
            if let crate::value::Value::Token(b) = value {
                crate::cliargs::validate_budget(object, &b)?;
            }
        }
        for p in f.lists[0].iter().chain(&f.lists[1]).chain(&f.lists[2]) {
            let Type::Wire(width) = p.ty else {
                return Err(err("non-wire oracle interface"));
            };
            env.wire(&p.name, width);
        }
        for (p, value) in f.lists[4].iter().zip(arr(params)?) {
            let ty = if p.ty == Type::Ptr {
                Json::obj(vec![("ptr", n(object.ptr_width))])
            } else {
                s(p.ty.signature_text())
            };
            env.values
                .insert(p.name.clone(), Bound::Value(value.clone(), ty));
        }
        for p in &f.lists[3] {
            let Type::Region(address, cell) = p.ty else {
                return Err(err("non-region oracle memory"));
            };
            let width = (1usize
                .checked_shl(address as u32)
                .ok_or_else(|| err("host limit: region"))?)
            .checked_mul(cell)
            .ok_or_else(|| err("host limit: region"))?;
            env.wire(&p.name, width);
            let cells = wires(&s(&p.name), &env)?;
            env.values.insert(
                p.name.clone(),
                Bound::Region {
                    address,
                    cell,
                    role: "in".into(),
                    origin: format!("interface:{}", p.name),
                    cells,
                },
            );
        }
        env.oracle_invocation = Some(oracle_invocation.clone());
        env.cleanup.entries = Rc::new(entries.to_vec());
        env.wire("$controls", inherited_controls);
        env.add_controls(wires(&s("$controls"), &env)?);
        let body = body(object, name, &mut env)?;
        let leaves = self.expand("adj", &[body], &env)?;
        let cleanups = env.cleanup.carriers.borrow().clone();
        Ok(ActionExpansion {
            leaves,
            cleanups,
            cleanup_pairing_complete: !env.cleanup.incomplete.get(),
        })
    }
}

pub(super) fn invocation_budget(
    object: &Object,
    context: Option<&Json>,
) -> Result<crate::value::Budget, String> {
    let context = context.ok_or_else(|| err("missing oracle_invocation source context"))?;
    keys(context, &["token"])?;
    let budget = crate::invocation::decode_budget(field(context, "token")?)?;
    crate::cliargs::validate_budget(object, &budget)?;
    Ok(budget)
}

pub(super) fn body(object: &Object, name: &str, env: &mut Bindings) -> Result<Json, String> {
    let budget = invocation_budget(object, env.oracle_invocation.as_ref())?;
    let f = object
        .function(name)
        .ok_or_else(|| err("source oracle missing"))?;
    if f.kind != FnKind::Oracle {
        return Err(err("source is not an oracle"));
    }
    env.source_formed = true;
    env.address_oracle(object, name)?;
    // Proof is source-derived and completed before any expansion or selection.
    let program = procedures::Program::new(object, name)?;
    let pairs = &program.pairs;
    let rows = program.rows.iter().collect::<Vec<_>>();
    let count = rows.len();
    let p = (usize::BITS - count.leading_zeros()) as usize;
    let sig = crate::validate::oracle_sig(object, name)?;
    let mut widths = program.widths.clone();
    for param in &f.lists[0] {
        let Type::Wire(width) = param.ty else {
            return Err(err("non-wire oracle input"));
        };
        widths.insert(param.name.clone(), width);
    }
    let labels = &program.labels;
    let stack = procedures::Stack::new(object, &program, p, &budget, env)?;
    let mut slots = vec![];
    for i in &rows {
        for (j, name) in i.lhs.iter().enumerate() {
            if slots.len() <= j {
                slots.resize(j + 1, 0);
            }
            slots[j] = slots[j].max(widths[name]);
        }
    }
    memory::slots(object, &rows, &mut slots)?;
    stack.slots(&program, &mut slots)?;
    let q = 2 * p + slots.iter().sum::<usize>() + 1;
    let frame_slots = slots
        .iter()
        .scan(p, |pos, width| {
            let result = range("$frame", *pos, *width);
            *pos += width;
            Some(result)
        })
        .collect::<Vec<_>>();
    let succ = range("$frame", q - p, p);
    let steps = usize::try_from(budget.steps).map_err(|_| err("host limit: oracle frames"))?;
    let mut declarations = vec![];
    let mut state = vec![];
    let mut add_state = |name: String, width: usize, initial: Json| {
        declarations.push((name.clone(), width));
        state.push(Json::obj(vec![
            ("width", n(width)),
            ("initial", initial),
            ("I", s("")),
            ("storage", s(name)),
        ]));
    };
    for (name, width) in [
        ("$oracle.pc", p),
        ("$oracle.next", p),
        ("$oracle.faulted", 1),
        ("$oracle.code", sig.fault),
        ("$oracle.result", sig.tgt),
    ] {
        add_state(name.into(), width, n(0));
    }
    for (name, width) in &program.locations {
        add_state(name.clone(), *width, n(0));
    }
    stack.state(&mut add_state)?;
    let mut constants = vec![];
    for param in &f.lists[4] {
        if param.ty == Type::Token {
            continue;
        }
        let Some(Bound::Value(value, _)) = env.values.get(&param.name).cloned() else {
            return Err(err("unbound oracle classical argument"));
        };
        let (width, initial) = match param.ty {
            Type::I(w) => (
                w,
                integer(crate::invocation::unsigned(&value)?.to_decimal(), env)?,
            ),
            Type::Ptr => (
                object.ptr_width,
                integer(crate::invocation::unsigned(&value)?.to_decimal(), env)?,
            ),
            Type::F64 => (
                64,
                integer(
                    u64::from_str_radix(string(&value)?.trim_start_matches("0x"), 16)
                        .map_err(err)?
                        .to_string(),
                    env,
                )?,
            ),
            _ => return Err(err("invalid oracle argument")),
        };
        widths.insert(param.name.clone(), width);
        let alias = format!("$oracle.value.{}", param.name);
        widths.insert(alias.clone(), width);
        constants.push(Json::obj(vec![
            ("alloc", s(&alias)),
            ("width", n(width)),
            ("class", s("any")),
            ("state", s("clean")),
        ]));
        constants.push(literal(width, initial, s(alias)));
    }
    let memory = memory::Memory::new(object, f, budget.heap, env, &mut add_state)?;
    let frames = (0..steps)
        .map(|t| {
            let name = format!("$oracle.frame{t}");
            add_state(name.clone(), q, n(0));
            Json::obj(vec![("index", n(t)), ("frame", s(name))])
        })
        .collect();
    if let Some(memory) = &memory {
        let record = state
            .iter_mut()
            .find(|r| r.get("storage").and_then(Json::as_str) == Some("$oracle.memory"))
            .unwrap();
        if let Json::Obj(fields) = record {
            fields.insert("I".into(), s(&memory.image));
        }
    }
    let mut order = vec![
        "$oracle.pc".to_string(),
        "$oracle.next".into(),
        "$oracle.faulted".into(),
        "$oracle.code".into(),
        "$oracle.result".into(),
    ];
    if memory.is_some() {
        order.extend(
            [
                "$oracle.used",
                "$oracle.atop",
                "$oracle.sp",
                "$oracle.fp",
                "$oracle.depth",
                "$oracle.memory",
                "$oracle.stack",
            ]
            .map(str::to_string),
        );
    }
    order.extend(program.locations.iter().map(|(name, _)| name.clone()));
    if memory.is_some() {
        order.push("$oracle.table".into());
    }
    order.extend((0..steps).map(|t| format!("$oracle.frame{t}")));
    state.sort_by_key(|record| {
        order
            .iter()
            .position(|name| record.get("storage").and_then(Json::as_str) == Some(name))
            .unwrap()
    });
    let instantiated = rows
        .iter()
        .map(|i| {
            let mut i = (*i).clone();
            let wire = |name: &str| {
                if f.lists[4]
                    .iter()
                    .any(|p| p.name == name && p.ty != Type::Token)
                {
                    format!("$oracle.value.{name}")
                } else {
                    name.to_string()
                }
            };
            let list_indices: &[usize] = match i.op.as_str() {
                "lift.oracle_call" | "lift.oracle_uncall" => &[2],
                "lift.parmov" => &[0],
                "lift.call" => &[1],
                "lift.perm_call" => &[1, 2],
                _ => &[],
            };
            if list_indices.is_empty() {
                for operand in &mut i.operands {
                    *operand = wire(operand);
                }
            } else {
                for &index in list_indices {
                    i.operands[index] = format!(
                        "({})",
                        crate::validate::list_operand(&i.operands[index])?
                            .iter()
                            .map(|name| wire(name))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
            }
            Ok(i)
        })
        .collect::<Result<Vec<_>, String>>()?;
    let rows = instantiated.iter().collect::<Vec<_>>();
    let mut templates = vec![];
    let mut successors = vec![];
    for (k, i) in rows.iter().enumerate() {
        let mut row = vec![];
        let mut next = vec![literal(p, n(k + 1), s("$next"))];
        let flag = range("$frame", q - p - 1, 1);
        if let Some((effect, next)) = direct::row(
            object,
            &program,
            i,
            k,
            p,
            count,
            sig.fault,
            &widths,
            &frame_slots,
            flag.clone(),
            succ.clone(),
            env,
        )? {
            templates.push(template(q, effect));
            successors.push(next_template(q, p, next));
            continue;
        }
        if let Some((effect, success)) = stack.row(
            &program,
            i,
            k,
            p,
            sig.fault,
            &frame_slots,
            &slots,
            &widths,
            flag.clone(),
            succ.clone(),
        )? {
            templates.push(template(q, effect));
            let next = if i.op == "lift.ret" {
                vec![
                    under(flag.clone(), vec![literal(p, n(count), s("$next"))]),
                    flip(flag.clone()),
                    under(flag.clone(), vec![xor(p, succ.clone(), s("$next"))]),
                    flip(flag),
                ]
            } else {
                fault_next(p, count, success, flag)
            };
            successors.push(next_template(q, p, next));
            continue;
        }
        if let Some(memory) = &memory
            && let Some(effect) =
                memory.row(i, &widths, &frame_slots, flag.clone(), sig.fault, env)?
        {
            templates.push(template(q, effect));
            successors.push(next_template(q, p, fault_next(p, count, k + 1, flag)));
            continue;
        }
        match i.op.as_str() {
            "lift.oracle_call" | "lift.oracle_uncall" => {
                let emitted_call = call(object, i, env)?;
                if let Some(&open) = pairs.get(&k) {
                    let targets = i.operands[3..5].iter().map(s).collect();
                    let allocations = i.operands[3..5]
                        .iter()
                        .map(|name| {
                            Json::obj(vec![
                                ("name", s(name)),
                                ("width", n(widths[name])),
                                ("class", s("any")),
                            ])
                        })
                        .collect();
                    let point = env
                        .oracle_cleanup_pairs()?
                        .into_iter()
                        .find(|pair| {
                            pair.cleanup_point == env.oracle_address.as_ref().unwrap().rows[k]
                        })
                        .ok_or_else(|| err("missing source cleanup pair"))?;
                    let computing = vec![call(object, rows[open], env)?];
                    let pair = Json::obj(vec![
                        ("point", s(point.computing_point.point)),
                        ("perm", Json::Bool(true)),
                        ("target", s("")),
                        ("targets", Json::Arr(targets)),
                        ("allocations", Json::Arr(allocations)),
                        (
                            "initialising",
                            Json::Arr(
                                i.operands[3..5]
                                    .iter()
                                    .map(|name| Json::obj(vec![("acquire", s(name))]))
                                    .collect(),
                            ),
                        ),
                        (
                            "uninitialising",
                            Json::Arr(
                                i.operands[3..5]
                                    .iter()
                                    .rev()
                                    .map(|name| Json::obj(vec![("relinquish", s(name))]))
                                    .collect(),
                            ),
                        ),
                        ("computing", Json::Arr(computing.clone())),
                        ("uncomputing", Json::Arr(vec![inv(computing)])),
                    ]);
                    row.push(Json::obj(vec![
                        ("$cleanup", pair),
                        ("body", Json::Arr(vec![emitted_call])),
                    ]));
                } else {
                    row.push(emitted_call);
                }
            }
            "lift.retval" => {
                row.push(xor(sig.tgt, s(&i.operands[0]), s("$oracle.result")));
                next = vec![literal(p, n(count), s("$next"))];
            }
            "qjump" => next = vec![literal(p, n(labels[&i.operands[0]]), s("$next"))],
            "qbranch" => {
                let control = s(&i.operands[0]);
                row = vec![
                    under(
                        control.clone(),
                        vec![literal(p, n(labels[&i.operands[1]]), succ.clone())],
                    ),
                    flip(control.clone()),
                    under(
                        control.clone(),
                        vec![literal(p, n(labels[&i.operands[2]]), succ.clone())],
                    ),
                    flip(control),
                ];
                next = vec![xor(p, succ.clone(), s("$next"))];
            }
            _ if !i.lhs.is_empty() => {
                let operands = &i.operands;
                let base = i.op.strip_prefix("lift.").ok_or_else(|| staged(i))?;
                let op = base.split('.').next().unwrap();
                let width = widths[&i.lhs[0]];
                let slot = range("$frame", p, width);
                let args = match op {
                    "const" => vec![
                        n(width),
                        integer(
                            crate::oracle::parse_const(&operands[0], width)?.to_decimal(),
                            env,
                        )?,
                        slot,
                    ],
                    "label" => vec![n(width), n(labels[&operands[0]]), slot],
                    "mov" => vec![n(width), s(&operands[0]), slot],
                    "add" | "sub" | "mul" | "and" | "or" | "xor" | "shl" | "lshr" | "ashr" => {
                        vec![n(width), s(&operands[0]), s(&operands[1]), slot]
                    }
                    "icmp" => vec![
                        n(widths[&operands[0]]),
                        s(&operands[0]),
                        s(&operands[1]),
                        slot,
                    ],
                    "zext" | "sext" | "trunc" => vec![],
                    "select" => vec![
                        n(width),
                        s(&operands[1]),
                        s(&operands[2]),
                        s(&operands[0]),
                        slot,
                    ],
                    "parmov" => vec![],
                    _ => return Err(staged(i)),
                };
                if matches!(op, "zext" | "sext" | "trunc") {
                    let input_width = widths[&operands[0]];
                    for bit in 0..width.min(input_width) {
                        row.push(xor(
                            1,
                            range(&operands[0], bit, 1),
                            range("$frame", p + bit, 1),
                        ));
                    }
                    if op == "sext" {
                        for bit in input_width..width {
                            row.push(xor(
                                1,
                                range(&operands[0], input_width - 1, 1),
                                range("$frame", p + bit, 1),
                            ));
                        }
                    }
                } else if op == "parmov" {
                    for ((name, input), slot) in i
                        .lhs
                        .iter()
                        .zip(crate::validate::list_operand(&operands[0])?)
                        .zip(&frame_slots)
                    {
                        let from = field(slot, "from")?.as_usize().unwrap();
                        row.push(xor(
                            widths[name],
                            s(input),
                            range("$frame", from, widths[name]),
                        ));
                    }
                } else {
                    let recipe = match op {
                        "const" | "label" => "value.literal".to_string(),
                        "mov" => "bind.qxor".into(),
                        "icmp" => format!("bind.value.{base}"),
                        _ => format!("bind.value.{op}"),
                    };
                    row.push(invoke(&recipe, args));
                }
                for (name, slot) in i.lhs.iter().zip(&frame_slots) {
                    // A shared slot may be wider than this row's destination.
                    let slot = field(slot, "from")?.as_usize().unwrap();
                    row.push(invoke(
                        "bind.qexch",
                        vec![
                            n(widths[name]),
                            range("$frame", slot, widths[name]),
                            s(name),
                        ],
                    ));
                }
            }
            _ => return Err(staged(i)),
        }
        let parameters = vec![Json::obj(vec![
            ("name", s("$frame")),
            ("type", s("wire")),
            ("width", n(q)),
        ])];
        templates.push(Json::obj(vec![
            ("parameters", Json::Arr(parameters.clone())),
            ("body", Json::Arr(row)),
        ]));
        let mut parameters = parameters;
        parameters.push(Json::obj(vec![
            ("name", s("$next")),
            ("type", s("wire")),
            ("width", n(p)),
        ]));
        successors.push(Json::obj(vec![
            ("parameters", Json::Arr(parameters)),
            ("body", Json::Arr(next)),
        ]));
    }
    if !constants.is_empty() {
        for t in &mut templates {
            if let Json::Obj(fields) = t {
                let Json::Arr(body) = fields.get_mut("body").unwrap() else {
                    unreachable!()
                };
                let mut wrapped = constants.clone();
                wrapped.append(body);
                wrapped.push(inv(constants.clone()));
                *body = wrapped;
            }
        }
    }
    let mut result = Json::Arr(vec![invoke(
        "oracle.construct",
        vec![
            n(p),
            n(count),
            n(q),
            n(steps),
            n(sig.fault),
            n(sig.tgt),
            s("$oracle.pc"),
            s("$oracle.next"),
            s("$oracle.faulted"),
            s("$oracle.code"),
            s("$oracle.result"),
            s(&f.lists[1][0].name),
            s(&f.lists[2][0].name),
            Json::Arr(state),
            Json::Arr(frames),
            Json::Arr(templates),
            Json::Arr(successors),
        ],
    )]);
    result = Json::Arr(vec![Json::obj(vec![
        (
            "$oracle_storage",
            Json::Arr(
                declarations
                    .into_iter()
                    .map(|(name, width)| Json::obj(vec![("name", s(name)), ("width", n(width))]))
                    .collect(),
            ),
        ),
        ("body", result),
    ])]);
    Ok(result)
}
pub(super) fn call(object: &Object, i: &Instr, env: &Bindings) -> Result<Json, String> {
    let name = i.operands[0].trim_start_matches('@');
    let hash = env.registry.as_ref().unwrap().identities.closure[name]
        .as_ref()
        .ok_or_else(|| err("unresolved nested oracle"))?;
    let mut actuals = crate::validate::list_operand(&i.operands[2])?
        .into_iter()
        .map(s)
        .collect::<Vec<_>>();
    actuals.extend(i.operands[3..5].iter().map(s));
    actuals.extend(
        crate::validate::list_operand(&i.operands[5])?
            .into_iter()
            .map(s),
    );
    actuals.extend(
        crate::validate::list_operand(&i.operands[6])?
            .into_iter()
            .map(s),
    );
    let _ = crate::validate::oracle_sig(object, name)?;
    Ok(Json::obj(vec![
        ("$oracle_invocation", s(&i.operands[1])),
        (
            "body",
            Json::Arr(vec![invoke(&i.op, vec![s(hash), Json::Arr(actuals)])]),
        ),
    ]))
}

fn template(q: usize, body: Vec<Json>) -> Json {
    Json::obj(vec![
        (
            "parameters",
            Json::Arr(vec![Json::obj(vec![
                ("name", s("$frame")),
                ("type", s("wire")),
                ("width", n(q)),
            ])]),
        ),
        ("body", Json::Arr(body)),
    ])
}
fn next_template(q: usize, p: usize, body: Vec<Json>) -> Json {
    let mut t = template(q, body);
    if let Json::Obj(ref mut fields) = t
        && let Json::Arr(params) = fields.get_mut("parameters").unwrap()
    {
        params.push(Json::obj(vec![
            ("name", s("$next")),
            ("type", s("wire")),
            ("width", n(p)),
        ]));
    }
    t
}
fn fault_next(p: usize, halt: usize, success: usize, flag: Json) -> Vec<Json> {
    vec![
        under(flag.clone(), vec![literal(p, n(halt), s("$next"))]),
        flip(flag.clone()),
        under(flag.clone(), vec![literal(p, n(success), s("$next"))]),
        flip(flag),
    ]
}
