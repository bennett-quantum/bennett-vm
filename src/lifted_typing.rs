//! ISA 0.9 lifted operand schemas (§3.3, §3.7, §4.2).
use crate::{
    bigint::BigUint,
    object::{FnKind, Function, Instr, Object, Param, Type},
    validate::{action_sig, err, list_operand, oracle_sig},
};
use std::collections::BTreeMap;

fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(err("3.3", message)) }
}

fn unsigned_literal(text: &str) -> Result<BigUint, String> {
    let n = BigUint::parse_decimal(text.strip_prefix('-').unwrap_or(text))
        .map_err(|_| err("3.3", "expected a nonnegative integer literal"))?;
    check(
        !text.starts_with('-') || n.is_zero(),
        "negative unsigned literal",
    )?;
    Ok(n)
}

struct Schema<'a> {
    obj: &'a Object,
    f: &'a Function,
    reads: BTreeMap<String, usize>,
    writes: BTreeMap<String, usize>,
}
impl<'a> Schema<'a> {
    fn new(obj: &'a Object, f: &'a Function) -> Self {
        let writes: BTreeMap<_, _> = crate::oracle::location_widths(f).into_iter().collect();
        let mut reads = writes.clone();
        let groups: &[usize] = if f.kind == FnKind::Oracle {
            &[0, 4]
        } else {
            &[3]
        };
        for p in groups.iter().flat_map(|g| &f.lists[*g]) {
            let w = match p.ty {
                Type::I(n) | Type::Wire(n) => Some(n),
                Type::Ptr => Some(obj.ptr_width),
                Type::F64 => Some(64),
                _ => None,
            };
            if let Some(w) = w {
                reads.insert(p.name.clone(), w);
            }
        }
        Self {
            obj,
            f,
            reads,
            writes,
        }
    }
    fn read(&self, name: &str) -> Result<usize, String> {
        self.reads.get(name).copied().ok_or_else(|| {
            err(
                "3.7",
                format!("{name:?} is not a readable local bit-vector"),
            )
        })
    }
    fn write(&self, name: &str) -> Result<usize, String> {
        self.writes
            .get(name)
            .copied()
            .ok_or_else(|| err("3.7", format!("{name:?} is not a location")))
    }
    fn width(&self, name: &str, want: usize) -> Result<(), String> {
        check(self.read(name)? == want, "lifted operand width mismatch")
    }
    fn destination(&self, ins: &Instr, want: usize) -> Result<(), String> {
        check(
            self.write(&ins.lhs[0])? == want,
            "lifted destination width mismatch",
        )
    }
    fn fixed(&self, name: &str) -> Option<&Param> {
        self.f
            .lists
            .iter()
            .enumerate()
            .filter(|(g, _)| {
                if self.f.kind == FnKind::Oracle {
                    *g == 4
                } else {
                    matches!(g, 2 | 3)
                }
            })
            .flat_map(|(_, ps)| ps)
            .find(|p| p.name == name)
    }
    fn arguments(&self, actual: &str, want: &[Type]) -> Result<(), String> {
        let actual = list_operand(actual)?;
        check(
            actual.len() == want.len(),
            "lifted classical argument count mismatch",
        )?;
        for (name, ty) in actual.iter().zip(want) {
            check(
                self.fixed(name).is_some_and(|p| p.ty == *ty),
                "lifted argument must be an identically typed immutable entry binding",
            )?;
        }
        Ok(())
    }
    fn regions(&self, actual: &str, want: &[(usize, usize, String)]) -> Result<(), String> {
        let actual = list_operand(actual)?;
        check(actual.len() == want.len(), "lifted region count mismatch")?;
        let group = if self.f.kind == FnKind::Oracle { 3 } else { 4 };
        for (name, (a, w, role)) in actual.iter().zip(want) {
            let p = self.f.lists[group]
                .iter()
                .find(|p| p.name == *name)
                .ok_or_else(|| err("3.7", format!("unknown mapped region {name:?}")))?;
            check(
                p.ty == Type::Region(*a, *w) && (role == "in" || p.role.as_deref() == Some("mut")),
                "lifted region shape/role mismatch",
            )?;
        }
        Ok(())
    }
    fn bundles(&self, actual: &str, want: &[usize], writable: bool) -> Result<(), String> {
        let actual = list_operand(actual)?;
        check(actual.len() == want.len(), "lifted bundle count mismatch")?;
        for (name, w) in actual.iter().zip(want) {
            check(
                if writable {
                    self.write(name)?
                } else {
                    self.read(name)?
                } == *w,
                "lifted call bundle width mismatch",
            )?;
        }
        Ok(())
    }
    fn length(&self, value: &str) -> Result<(), String> {
        if crate::validate::is_int_literal(value) {
            let n = unsigned_literal(value)?;
            check(
                n.bits() <= self.obj.ptr_width,
                "literal lifted length exceeds ptr_width",
            )
        } else {
            self.read(value).map(|_| ())
        }
    }
    fn row(&self, ins: &Instr) -> Result<(), String> {
        let xs = &ins.operands;
        let op = ins.op.strip_prefix("lift.").unwrap_or(&ins.op);
        let (base, suffix) = op.split_once(".i").unwrap_or((op, ""));
        let suffix = || {
            suffix
                .parse::<usize>()
                .map_err(|_| err("3.3", "missing lifted width"))
        };
        match base {
            "const" => {
                let w = suffix()?;
                self.destination(ins, w)?;
                let (negative, text) = xs[0]
                    .strip_prefix('-')
                    .map_or((false, xs[0].as_str()), |t| (true, t));
                let n = BigUint::parse_decimal(text)?;
                check(
                    if negative {
                        n.cmp_big(&BigUint::from_u64(1).shl(w - 1)).is_le()
                    } else {
                        n.bits() <= w
                    },
                    "constant outside signed/unsigned spelling range",
                )
            }
            "add" | "sub" | "mul" | "and" | "or" | "xor" | "shl" | "lshr" | "ashr" | "udiv"
            | "urem" | "sdiv" | "srem" => {
                let w = self.read(&xs[0])?;
                self.width(&xs[1], w)?;
                self.destination(ins, w)
            }
            op if op.starts_with("icmp.") => {
                self.width(&xs[1], self.read(&xs[0])?)?;
                self.destination(ins, 1)
            }
            "select" => {
                self.width(&xs[0], 1)?;
                let w = self.read(&xs[1])?;
                self.width(&xs[2], w)?;
                self.destination(ins, w)
            }
            "zext" | "sext" | "trunc" => {
                let from = self.read(&xs[0])?;
                let to = suffix()?;
                check(
                    if base == "trunc" {
                        to < from
                    } else {
                        to > from
                    },
                    "lifted cast must strictly widen/narrow",
                )?;
                self.destination(ins, to)
            }
            "mov" => self.destination(ins, self.read(&xs[0])?),
            "parmov" => {
                for (dst, src) in ins.lhs.iter().zip(list_operand(&xs[0])?) {
                    check(
                        self.write(dst)? == self.read(&src)?,
                        "parallel move width mismatch",
                    )?;
                }
                Ok(())
            }
            "addr" => self.destination(ins, self.obj.ptr_width),
            "alloc" => {
                self.length(&xs[0])?;
                self.destination(ins, self.obj.ptr_width)
            }
            "boundscheck" => {
                self.width(&xs[0], self.obj.ptr_width)?;
                self.length(&xs[1])
            }
            "free" => self.width(&xs[0], self.obj.ptr_width),
            "load" | "store" => {
                let w = suffix()?;
                check(
                    w != 0 && w <= 128 && w.is_multiple_of(8),
                    "lifted memory width must be 8,16,..,128",
                )?;
                self.width(&xs[0], self.obj.ptr_width)?;
                if base == "load" {
                    self.destination(ins, w)
                } else {
                    self.width(&xs[1], w)
                }
            }
            "faultif" => {
                unsigned_literal(&xs[1])?;
                self.width(&xs[0], 1)
            }
            "qbranch" => self.width(&xs[0], 1),
            "ret" => self.width(&xs[0], self.f.lists[1][0].ty.width().unwrap()),
            "retval" => self.width(&xs[0], oracle_sig(self.obj, &self.f.name)?.tgt),
            "oracle_call" | "oracle_uncall" => {
                let sig = oracle_sig(self.obj, xs[0].trim_start_matches('@'))?;
                check(
                    self.fixed(&xs[1]).is_some_and(|p| p.ty == Type::Token),
                    "lifted oracle call requires immutable token",
                )?;
                self.bundles(&xs[2], &sig.ins, false)?;
                check(
                    self.write(&xs[3])? == sig.tgt && self.write(&xs[4])? == sig.fault,
                    "lifted oracle target/fault width mismatch",
                )?;
                self.regions(&xs[5], &sig.mem)?;
                self.arguments(&xs[6], &sig.args)
            }
            "perm_call" => {
                let sig = action_sig(self.obj, xs[0].trim_start_matches('@'))?;
                self.bundles(&xs[1], &sig.muts, true)?;
                self.bundles(&xs[2], &sig.ins, false)?;
                self.regions(&xs[3], &sig.mem)?;
                self.arguments(&xs[4], &sig.args)
            }
            // Existence/arity checked by validate_function; closure widths checked below.
            "label" | "qjumpr" => {
                if base == "qjumpr" {
                    self.read(&xs[0])?;
                }
                Ok(())
            }
            "fault" => unsigned_literal(&xs[0]).map(|_| ()),
            "call" | "qjump" => Ok(()),
            _ => Err(err("4.2", format!("unhandled lifted schema {}", ins.op))),
        }
    }
}

pub(crate) fn function(obj: &Object, f: &Function) -> Result<(), String> {
    let schema = Schema::new(obj, f);
    check(
        schema.reads.values().all(|w| *w > 0),
        "zero-width lifted bit-vector",
    )?;
    for ins in f.blocks.iter().flat_map(|b| &b.instrs) {
        schema
            .row(ins)
            .map_err(|e| format!("{e} in @{}: {}", f.name, ins.raw))?;
    }
    Ok(())
}

pub(crate) fn closures(obj: &Object) -> Result<(), String> {
    for root in obj.functions.iter().filter(|f| f.kind == FnKind::Oracle) {
        let names = crate::oracle::lproc_closure(obj, root);
        let fs = std::iter::once(root)
            .chain(names.iter().filter_map(|n| obj.function(n)))
            .collect::<Vec<_>>();
        let fault = oracle_sig(obj, &root.name)?.fault;
        // Unresolved imported rows acquire their exact index space at link, when validated again.
        let rows = if names.iter().all(|n| obj.function(n).is_some()) {
            Some(
                crate::instances::for_oracle(obj, &root.name)?
                    .iter()
                    .map(|i| {
                        obj.function(&i.symbol)
                            .unwrap()
                            .blocks
                            .iter()
                            .map(|b| b.instrs.len())
                            .sum::<usize>()
                    })
                    .sum::<usize>(),
            )
        } else {
            None
        };
        for f in fs {
            for ins in f.blocks.iter().flat_map(|b| &b.instrs) {
                let code = match ins.op.as_str() {
                    "fault" => Some(&ins.operands[0]),
                    "lift.faultif" => Some(&ins.operands[1]),
                    _ => None,
                };
                if let Some(c) = code {
                    let c = unsigned_literal(c)?;
                    check(
                        c.bits() <= fault,
                        "fault code exceeds containing oracle's fault width",
                    )?;
                }
                if let Some(n) = rows {
                    let width = match ins.op.as_str() {
                        "lift.label" => Some(
                            crate::oracle::location_widths(f)
                                .into_iter()
                                .find(|(p, _)| p == &ins.lhs[0])
                                .unwrap()
                                .1,
                        ),
                        "qjumpr" => {
                            let schema = Schema::new(obj, f);
                            Some(schema.read(&ins.operands[0])?)
                        }
                        _ => None,
                    };
                    if let Some(w) = width {
                        check(
                            w >= usize::BITS as usize || (1usize << w) > n,
                            "label/jumpr width cannot encode complete oracle index space",
                        )?;
                    }
                }
            }
        }
    }
    Ok(())
}
