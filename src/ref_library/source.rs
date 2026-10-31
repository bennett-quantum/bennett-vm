//! Source formation from this consumer's parsed objects and the published ISA.
use super::*;
use crate::object::{FnKind, Instr, Object, Type};

fn text(s: impl Into<String>) -> Json {
    Json::Str(s.into())
}
fn num(n: usize) -> Json {
    Json::u64(n as u64)
}
fn invocation(kind: &str, args: Vec<Json>) -> Json {
    Json::obj(vec![("invoke", text(kind)), ("args", Json::Arr(args))])
}
fn inverse(body: Vec<Json>) -> Json {
    Json::obj(vec![
        ("inverse", Json::Bool(true)),
        ("body", Json::Arr(body)),
    ])
}
#[derive(Clone)]
struct Region {
    address: usize,
    cell: usize,
    role: String,
    local: bool,
}
struct Source {
    widths: BTreeMap<String, usize>,
    regions: BTreeMap<String, Region>,
    declarations: Vec<(String, usize, String, String)>,
    locals: Vec<Instr>,
}
impl Catalog {
    /// Form a structured source action from resolved classical parameters,
    /// retaining arbitrary symbolic interface inputs. This boundary precedes
    /// invocation-address assignment and target/schedule realisation.
    pub fn lower_action(
        &self,
        object: &Object,
        name: &str,
        params: &Json,
        adjoint: bool,
        inherited_controls: usize,
    ) -> Result<Vec<Leaf>, String> {
        let f = object
            .function(name)
            .ok_or_else(|| err(format!("unresolved action @{name}")))?;
        if !matches!(f.kind, FnKind::Action | FnKind::PermAction) {
            return Err(err("source is not an action"));
        }
        let types = f.lists[3].iter().map(|p| p.ty.clone()).collect::<Vec<_>>();
        crate::invocation::decode_args(params, &types, object.ptr_width)?;
        let mut env = Bindings::new();
        let mut source = Source {
            widths: BTreeMap::new(),
            regions: BTreeMap::new(),
            declarations: vec![],
            locals: vec![],
        };
        for p in f.lists[0].iter().chain(&f.lists[1]) {
            let Type::Wire(width) = p.ty else {
                return Err(err("non-wire source interface"));
            };
            source.widths.insert(p.name.clone(), width);
            env.wire(&p.name, width);
        }
        for p in &f.lists[2] {
            let Type::Region(address, cell) = p.ty else {
                return Err(err("non-region memory interface"));
            };
            let region = Region {
                address,
                cell,
                role: p.role.clone().ok_or_else(|| err("region role missing"))?,
                local: false,
            };
            let width = source.region_width(&region)?;
            env.wire(&p.name, width);
            source.regions.insert(p.name.clone(), region);
        }
        for (p, value) in f.lists[3].iter().zip(arr(params)?) {
            let ty = match p.ty {
                Type::Ptr => Json::obj(vec![("ptr", num(object.ptr_width))]),
                _ => text(p.ty.signature_text()),
            };
            env.values
                .insert(p.name.clone(), Bound::Value(value.clone(), ty));
        }
        env.wire("$controls", inherited_controls);
        env.add_controls(wires(&text("$controls"), &env)?);
        let instructions = f
            .blocks
            .iter()
            .flat_map(|b| b.instrs.clone())
            .collect::<Vec<_>>();
        source.collect_regions(&instructions)?;
        source.declarations(&instructions)?;
        let mut body = source.sequence(&instructions, adjoint)?;
        for (name, width, class, state) in source.declarations.iter().rev() {
            body = vec![Json::obj(vec![
                ("declare", text(name)),
                ("width", num(*width)),
                ("class", Json::obj(vec![("class", text(class))])),
                ("state", text(state)),
                ("body", Json::Arr(body)),
            ])];
        }
        for local in source.locals.iter().rev() {
            let name = &local.operands[0];
            let region = &source.regions[name];
            let cells = source.region_width(region)?;
            let image = if let Some(image) = local.operands.get(3) {
                object
                    .data
                    .get(image.trim_start_matches('@'))
                    .ok_or_else(|| err("unbound source image"))?
                    .clone()
            } else {
                String::new()
            };
            let template = Json::obj(vec![
                (
                    "parameters",
                    Json::Arr(vec![Json::obj(vec![
                        ("name", text(name)),
                        ("type", text("wire")),
                        ("width", num(cells)),
                    ])]),
                ),
                ("body", Json::Arr(body)),
            ]);
            body = vec![invocation(
                "qregion",
                vec![
                    num(cells / region.cell),
                    num(region.cell),
                    text(image),
                    Json::Arr(vec![template]),
                    Json::Arr(vec![]),
                ],
            )];
        }
        self.expand("adj", &[Json::Arr(body)], &env)
    }
}
impl Source {
    fn region_width(&self, r: &Region) -> Result<usize, String> {
        u32::try_from(r.address)
            .ok()
            .and_then(|a| 1usize.checked_shl(a))
            .and_then(|n| n.checked_mul(r.cell))
            .ok_or_else(|| err("source region width exceeds host limit"))
    }
    fn width(&self, wire: &str) -> Result<usize, String> {
        if let Some((base, sub)) = wire.split_once('[') {
            let width = *self
                .widths
                .get(base)
                .ok_or_else(|| err(format!("unbound source bundle {base}")))?;
            let bit = sub
                .strip_suffix(']')
                .ok_or_else(|| err("malformed source wire"))?
                .parse::<usize>()
                .map_err(err)?;
            if bit >= width {
                return Err(err("source wire outside width"));
            }
            Ok(1)
        } else {
            self.widths
                .get(wire)
                .copied()
                .ok_or_else(|| err(format!("unbound source bundle {wire}")))
        }
    }
    fn collect_regions(&mut self, instructions: &[Instr]) -> Result<(), String> {
        for i in instructions {
            if i.op == "qregion" {
                let name = i.operands[0].clone();
                let r = Region {
                    address: i.operands[1].parse::<usize>().map_err(err)?,
                    cell: i.operands[2].parse::<usize>().map_err(err)?,
                    role: "mut".into(),
                    local: true,
                };
                if self.regions.insert(name, r).is_some() {
                    return Err(err("duplicate source region declaration"));
                }
                self.locals.push(i.clone());
            }
            for arm in &i.nested {
                self.collect_regions(arm)?;
            }
        }
        Ok(())
    }
    fn declarations(&mut self, instructions: &[Instr]) -> Result<(), String> {
        for i in instructions {
            let allocation = match i.op.as_str() {
                "qalloc" | "qborrow" => Some((
                    i.operands[1].parse::<usize>().map_err(err)?,
                    i.operands[0].clone(),
                    if i.op == "qborrow" {
                        "borrowed"
                    } else {
                        "clean"
                    },
                )),
                "qcopy" => Some((self.width(&i.operands[0])?, "any".into(), "clean")),
                "qload" => Some((
                    self.regions
                        .get(&i.operands[0])
                        .ok_or_else(|| err("unbound source region"))?
                        .cell,
                    "any".into(),
                    "clean",
                )),
                _ => None,
            };
            if let Some((width, class, state)) = allocation {
                let name = i
                    .lhs
                    .first()
                    .ok_or_else(|| err("allocation result missing"))?;
                if self.widths.insert(name.clone(), width).is_some() {
                    return Err(err("duplicate source allocation result"));
                }
                self.declarations
                    .push((name.clone(), width, class, state.into()));
            }
            for arm in &i.nested {
                self.declarations(arm)?;
            }
        }
        Ok(())
    }
    fn sequence(&self, instructions: &[Instr], adjoint: bool) -> Result<Vec<Json>, String> {
        let mut out = vec![];
        let mut index = 0;
        while index < instructions.len() {
            let i = &instructions[index];
            if i.op == "qpred" {
                let mut depth = 1;
                let mut end = index + 1;
                while end < instructions.len() {
                    match instructions[end].op.as_str() {
                        "qpred" => depth += 1,
                        "qunpred" => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    end += 1;
                }
                if end == instructions.len() || instructions[end].operands != i.operands {
                    return Err(err("unmatched source predicate scope"));
                }
                let body = self.sequence(&instructions[index + 1..end], adjoint)?;
                out.push(invocation(
                    "qpred.scope",
                    vec![text(&i.operands[0]), Json::Arr(body)],
                ));
                index = end + 1;
            } else {
                out.extend(self.instruction(i, adjoint)?);
                index += 1;
            }
        }
        if adjoint {
            out.reverse();
        }
        Ok(out)
    }
    fn instruction(&self, i: &Instr, adjoint: bool) -> Result<Vec<Json>, String> {
        let op = i.op.as_str();
        let operands = &i.operands;
        if crate::action::is_gate_name(op) {
            let op = if adjoint {
                match op {
                    "s" => "sdg",
                    "sdg" => "s",
                    "t" => "tdg",
                    "tdg" => "t",
                    _ => op,
                }
            } else {
                op
            };
            let args = if ["rx", "ry", "rz"].contains(&op) {
                let angle = if let Ok(value) = operands[0].parse::<f64>() {
                    text(format!(
                        "0x{:016x}",
                        value.to_bits() ^ if adjoint { 1 << 63 } else { 0 }
                    ))
                } else {
                    if adjoint {
                        return Err(err(
                            "unavailable.reason: resolved argument rotation adjoint staged in bennett-vm-47t",
                        ));
                    }
                    text(&operands[0])
                };
                vec![angle, text(&operands[1]), Json::Arr(vec![])]
            } else if op == "mcx" {
                let controls = crate::validate::list_operand(&operands[0])?;
                vec![
                    num(controls.len()),
                    Json::Arr(controls.iter().map(text).collect()),
                    text(&operands[1]),
                    Json::Arr(vec![]),
                ]
            } else {
                operands
                    .iter()
                    .map(text)
                    .chain(std::iter::once(Json::Arr(vec![])))
                    .collect()
            };
            let kind = if ["x", "cx", "ccx", "swap"].contains(&op) {
                format!("bind.gate.{op}")
            } else {
                format!("gate.{op}")
            };
            return Ok(vec![invocation(&kind, args)]);
        }
        let (kind, args) = match op {
            "qregion" => return Ok(vec![]),
            "qunpred" => return Err(err("unmatched source qunpred")),
            "qalloc" | "qborrow" => (
                match (op, adjoint) {
                    ("qalloc", false) => "qalloc",
                    ("qalloc", true) => "qfree",
                    ("qborrow", false) => "qborrow",
                    _ => "qreturn",
                },
                vec![text(&i.lhs[0])],
            ),
            "qfree" | "qreturn" => (
                if !adjoint {
                    op
                } else if op == "qfree" {
                    "qalloc"
                } else {
                    "qborrow"
                },
                vec![text(&operands[0])],
            ),
            "qcopy" => (
                if adjoint {
                    "quncopy.bound"
                } else {
                    "qcopy.bound"
                },
                vec![
                    num(self.width(&operands[0])?),
                    text(&operands[0]),
                    text(&i.lhs[0]),
                ],
            ),
            "quncopy" => (
                if adjoint {
                    "qcopy.bound"
                } else {
                    "quncopy.bound"
                },
                vec![
                    num(self.width(&operands[0])?),
                    text(&operands[0]),
                    text(&operands[1]),
                ],
            ),
            "qxor" | "qexch" => (
                if op == "qxor" {
                    "bind.qxor"
                } else {
                    "bind.qexch"
                },
                vec![
                    num(self.width(&operands[0])?),
                    text(&operands[0]),
                    text(&operands[1]),
                ],
            ),
            "qload" | "qunload" | "qstore" | "qexchm" => {
                let name = &operands[0];
                let r = self
                    .regions
                    .get(name)
                    .ok_or_else(|| err("unbound source region"))?;
                let width = self.region_width(r)?;
                let region = Json::obj(vec![
                    ("address", num(r.address)),
                    ("cell", num(r.cell)),
                    ("role", text(&r.role)),
                    (
                        "origin",
                        text(format!(
                            "{}:{name}",
                            if r.local { "local" } else { "interface" }
                        )),
                    ),
                    ("cells", text(name)),
                ]);
                let load = op == "qload" || op == "qunload";
                let value = if op == "qload" {
                    &i.lhs[0]
                } else {
                    &operands[2]
                };
                let kind = if load {
                    format!(
                        "{}.{}",
                        if (op == "qload") != adjoint {
                            "qload"
                        } else {
                            "qunload"
                        },
                        r.role
                    )
                } else {
                    op.into()
                };
                return Ok(vec![invocation(
                    &kind,
                    vec![
                        num(r.address),
                        num(width / r.cell),
                        num(r.cell),
                        text(&operands[1]),
                        if load { region } else { text(name) },
                        text(value),
                    ],
                )]);
            }
            "qif" => {
                let body = vec![invocation(
                    "qif",
                    vec![
                        text(&operands[0]),
                        Json::Arr(self.sequence(&i.nested[0], false)?),
                        Json::Arr(self.sequence(&i.nested[1], false)?),
                    ],
                )];
                return Ok(if adjoint { vec![inverse(body)] } else { body });
            }
            "qloop" => {
                let count = operands[0].parse::<usize>().map_err(err)?;
                if operands.len() != 1 {
                    let body = vec![invocation(
                        "qloop.while",
                        vec![
                            num(count),
                            text(&operands[2]),
                            Json::Arr(self.sequence(&i.nested[0], false)?),
                        ],
                    )];
                    return Ok(if adjoint { vec![inverse(body)] } else { body });
                }
                (
                    "qloop",
                    vec![num(count), Json::Arr(self.sequence(&i.nested[0], adjoint)?)],
                )
            }
            _ => {
                return Err(err(format!(
                    "unavailable.reason: source {op} symbol/row formation staged in bennett-vm-47t"
                )));
            }
        };
        Ok(vec![invocation(kind, args)])
    }
}
