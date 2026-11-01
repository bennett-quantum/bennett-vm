//! Fixed binding instances, canonical row order and §3.7 stack descriptors.
use super::*;
use crate::object::Function;

pub(super) struct Program {
    pub rows: Vec<Instr>,
    pub widths: BTreeMap<String, usize>,
    pub locations: Vec<(String, usize)>,
    pub labels: BTreeMap<String, usize>,
    pub pairs: BTreeMap<usize, usize>,
    functions: BTreeMap<String, Function>,
    ranges: Vec<(String, usize, usize)>,
}
fn bits(x: usize) -> usize {
    ((usize::BITS - x.leading_zeros()) as usize).max(1)
}
impl Program {
    pub fn new(object: &Object, root: &str) -> Result<Self, String> {
        let plan = crate::instances::Plan::form(object, root)?;
        let oracle = object.function(root).unwrap();
        let mut result = Self {
            rows: vec![],
            widths: BTreeMap::new(),
            locations: vec![],
            labels: BTreeMap::new(),
            pairs: BTreeMap::new(),
            functions: BTreeMap::new(),
            ranges: vec![],
        };
        for (instance, name) in plan.order.iter().enumerate() {
            let f = &plan.functions[name];
            let locals = crate::oracle::location_widths(f);
            let mut rename = BTreeMap::new();
            for (n, w) in &locals {
                let alias = if instance == 0 {
                    n.clone()
                } else {
                    format!("$oracle.instance{instance}.{n}")
                };
                rename.insert(n.clone(), alias.clone());
                result.widths.insert(alias.clone(), *w);
                result.locations.push((alias, *w));
            }
            for ((owner, param), &index) in &plan.arguments {
                if owner == name {
                    rename.insert(param.clone(), oracle.lists[4][index].name.clone());
                }
            }
            for ((owner, param), &index) in &plan.regions {
                if owner == name {
                    rename.insert(param.clone(), oracle.lists[3][index].name.clone());
                }
            }
            let rename_one = |n: &str| {
                if let Some(label) = n.strip_prefix('^') {
                    format!("^{name}.{label}")
                } else if let Some(region) = n
                    .strip_prefix('@')
                    .filter(|n| plan.regions.contains_key(&(name.clone(), (*n).to_string())))
                    .and_then(|n| rename.get(n))
                {
                    format!("@{region}")
                } else {
                    rename.get(n).cloned().unwrap_or_else(|| n.into())
                }
            };
            let first = result.rows.len();
            for (close, open) in oracle_cleanup::pairs(f)? {
                result.pairs.insert(first + close, first + open);
            }
            let mut transformed = f.clone();
            for params in &mut transformed.lists[..2] {
                for p in params {
                    p.name = rename_one(&p.name);
                }
            }
            for p in &mut transformed.regs {
                p.name = rename_one(&p.name);
            }
            for block in &mut transformed.blocks {
                result
                    .labels
                    .insert(format!("^{name}.{}", block.name), result.rows.len());
                for i in &mut block.instrs {
                    for n in &mut i.lhs {
                        *n = rename_one(n);
                    }
                    let lists: &[usize] = match i.op.as_str() {
                        "lift.call" => &[1, 2, 3, 4],
                        "lift.oracle_call" | "lift.oracle_uncall" => &[2, 5, 6],
                        "lift.perm_call" => &[1, 2, 3, 4],
                        "lift.parmov" => &[0],
                        _ => &[],
                    };
                    for (index, operand) in i.operands.iter_mut().enumerate() {
                        if lists.contains(&index) {
                            *operand = format!(
                                "({})",
                                crate::validate::list_operand(operand)?
                                    .iter()
                                    .map(|name| rename_one(name))
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            );
                        } else if !(matches!(
                            i.op.as_str(),
                            "lift.call"
                                | "lift.oracle_call"
                                | "lift.oracle_uncall"
                                | "lift.perm_call"
                        ) && index == 0)
                        {
                            *operand = rename_one(operand);
                        }
                    }
                    result.rows.push(i.clone());
                }
            }
            result.ranges.push((name.clone(), first, result.rows.len()));
            result.functions.insert(name.clone(), transformed);
        }
        Ok(result)
    }
    fn locations(&self, f: &Function) -> Json {
        Json::Arr(
            crate::oracle::location_widths(f)
                .iter()
                .enumerate()
                .map(|(slot, (name, width))| {
                    let id = self.locations.iter().position(|(n, _)| n == name).unwrap();
                    Json::obj(vec![
                        ("id", n(id)),
                        ("width", n(*width)),
                        ("slot", n(slot)),
                        ("location", s(name)),
                    ])
                })
                .collect(),
        )
    }
}
pub(super) struct Stack {
    sc: usize,
    dc: usize,
    cells: usize,
    cell: usize,
    did: usize,
    stack_limit: Json,
    depth_limit: Json,
    active: bool,
}
impl Stack {
    pub fn new(
        object: &Object,
        program: &Program,
        p: usize,
        budget: &crate::value::Budget,
        env: &mut Bindings,
    ) -> Result<Self, String> {
        let exponent = object.layout.as_ref().map_or(0, |l| l.s_cells);
        let sc = exponent + 1;
        let dc = (u128::BITS - (u128::from(budget.depth) + 1).leading_zeros()) as usize;
        let cells = 1usize
            .checked_shl(exponent as u32)
            .ok_or_else(|| err("host limit: stack cells"))?;
        let did = bits(program.locations.len().saturating_sub(1));
        let cell = program
            .locations
            .iter()
            .map(|(_, w)| *w)
            .max()
            .unwrap_or(1)
            .max(p)
            .max(sc)
            .max(did);
        Ok(Self {
            sc,
            dc,
            cells,
            cell,
            did,
            active: object.layout.is_some(),
            stack_limit: integer(budget.stack.to_string(), env)?,
            depth_limit: integer(budget.depth.to_string(), env)?,
        })
    }
    pub fn slots(&self, program: &Program, slots: &mut Vec<usize>) -> Result<(), String> {
        for i in &program.rows {
            let sizes = match i.op.as_str() {
                "lift.call" => {
                    let f = &program.functions[i.operands[0].trim_start_matches('@')];
                    let mut sizes = f.lists[0]
                        .iter()
                        .map(|p| match p.ty {
                            Type::Reg(w) => w,
                            _ => unreachable!(),
                        })
                        .collect::<Vec<_>>();
                    sizes.extend([self.sc, self.sc, self.dc]);
                    sizes
                }
                "lift.ret" => vec![
                    program.locations.iter().map(|(_, w)| *w).max().unwrap_or(1),
                    self.sc,
                    self.dc,
                ],
                _ => continue,
            };
            slots.resize(slots.len().max(sizes.len()), 0);
            for (slot, w) in slots.iter_mut().zip(sizes) {
                *slot = (*slot).max(w);
            }
        }
        Ok(())
    }
    pub fn state(&self, add: &mut impl FnMut(String, usize, Json)) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        for (name, w) in [
            ("$oracle.sp", self.sc),
            ("$oracle.fp", self.sc),
            ("$oracle.depth", self.dc),
            (
                "$oracle.stack",
                self.cells
                    .checked_mul(self.cell)
                    .ok_or_else(|| err("host limit: stack bits"))?,
            ),
        ] {
            add(name.into(), w, n(0));
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub fn row(
        &self,
        program: &Program,
        i: &Instr,
        k: usize,
        p: usize,
        fault: usize,
        slots: &[Json],
        slot_widths: &[usize],
        widths: &BTreeMap<String, usize>,
        flag: Json,
        succ: Json,
    ) -> Result<Option<(Vec<Json>, usize)>, String> {
        let slot = |index: usize, w: usize| -> Result<Json, String> {
            Ok(range(
                "$frame",
                field(&slots[index], "from")?.as_usize().unwrap(),
                w,
            ))
        };
        let tail = || vec![flag.clone(), s("$oracle.faulted"), s("$oracle.code")];
        let mut args;
        let success = if i.op == "lift.call" {
            let f = &program.functions[i.operands[0].trim_start_matches('@')];
            let locations = crate::oracle::location_widths(f);
            let record = 3 + locations.len();
            let m = f.lists[0].len();
            let dest = program
                .locations
                .iter()
                .position(|(n, _)| n == &i.lhs[0])
                .unwrap();
            args = vec![
                n(self.dc),
                n(self.sc),
                n(self.sc.max(bits(record)) + 1),
                n(record),
                n(self.cells),
                n(self.cell),
                n(p),
                n(self.did),
                n(fault),
                self.depth_limit.clone(),
                self.stack_limit.clone(),
                n(k + 1),
                n(dest),
                s("$oracle.depth"),
                s("$oracle.sp"),
                s("$oracle.fp"),
                s("$oracle.stack"),
                slot(m, self.sc)?,
                slot(m + 1, self.sc)?,
                slot(m + 2, self.dc)?,
            ];
            args.extend(tail());
            args.push(program.locations(f));
            args.push(Json::Arr(
                f.lists[0]
                    .iter()
                    .zip(crate::validate::list_operand(&i.operands[1])?)
                    .enumerate()
                    .map(|(index, (param, source))| {
                        let w = program.widths[&param.name];
                        Ok(Json::obj(vec![
                            ("position", n(index)),
                            ("width", n(w)),
                            ("source", s(source)),
                            ("parameter", s(&param.name)),
                            ("capture", slot(index, w)?),
                        ]))
                    })
                    .collect::<Result<Vec<_>, String>>()?,
            ));
            program.labels[&format!("^{}.{}", f.name, f.blocks[0].name)]
        } else if i.op == "lift.ret" {
            let (owner, _, _) = program
                .ranges
                .iter()
                .find(|(_, a, b)| k >= *a && k < *b)
                .unwrap();
            let f = &program.functions[owner];
            args = vec![
                n(self.dc),
                n(self.sc),
                n(self.cells),
                n(self.cell),
                n(p),
                n(self.did),
                n(fault),
                n(slot_widths[0]),
                n(widths[&i.operands[0]]),
                s("$oracle.depth"),
                s("$oracle.fp"),
                s("$oracle.stack"),
                slot(1, self.sc)?,
                slot(2, self.dc)?,
            ];
            args.extend(tail());
            args.extend([
                s(&i.operands[0]),
                slot(0, slot_widths[0])?,
                succ,
                program.locations(f),
            ]);
            args.push(Json::Arr(
                program
                    .locations
                    .iter()
                    .enumerate()
                    .map(|(id, (name, width))| {
                        Json::obj(vec![
                            ("id", n(id)),
                            ("width", n(*width)),
                            (
                                "slot",
                                n(program
                                    .functions
                                    .values()
                                    .find_map(|f| {
                                        crate::oracle::location_widths(f)
                                            .iter()
                                            .position(|(n, _)| n == name)
                                    })
                                    .unwrap()),
                            ),
                            ("location", s(name)),
                        ])
                    })
                    .collect(),
            ));
            0 // dynamic successor is already captured in frame.succ
        } else {
            return Ok(None);
        };
        Ok(Some((vec![invoke(&i.op, args)], success)))
    }
}
