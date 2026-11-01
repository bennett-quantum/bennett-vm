//! Memory descriptors are derived only from the admitted source layout.
use super::*;
use crate::bigint::BigUint;

pub(super) struct Memory {
    ptr: usize,
    h: usize,
    atop: usize,
    capacity: usize,
    heap_base: Json,
    heap_limit: Json,
    layout: Json,
    addresses: BTreeMap<String, Json>,
    pub image: String,
}
fn extent_for(exponent: usize) -> Result<usize, String> {
    1usize
        .checked_shl(exponent as u32)
        .ok_or_else(|| err("host limit: oracle memory descriptor"))
}
fn table_view(ptr: usize, capacity: usize, offset: usize, width: usize) -> Json {
    Json::Arr(
        (0..capacity)
            .map(|i| range("$oracle.table", i * (2 * ptr + 2) + offset, width))
            .collect(),
    )
}
pub(super) fn slots(
    object: &Object,
    rows: &[&Instr],
    slots: &mut Vec<usize>,
) -> Result<(), String> {
    for i in rows {
        let widths = match i.op.as_str() {
            "lift.alloc" => {
                let l = object
                    .layout
                    .as_ref()
                    .ok_or_else(|| err("missing allocation layout"))?;
                vec![
                    object.ptr_width,
                    2 * object.ptr_width + 2,
                    l.h + 2,
                    l.a_tab + 1,
                ]
            }
            op if op.starts_with("lift.store.i") => vec![op[12..].parse().map_err(err)?],
            _ => continue,
        };
        slots.resize(slots.len().max(widths.len()), 0);
        for (slot, width) in slots.iter_mut().zip(widths) {
            *slot = (*slot).max(width);
        }
    }
    Ok(())
}
impl Memory {
    pub(super) fn new(
        object: &Object,
        f: &crate::object::Function,
        heap: u64,
        env: &mut Bindings,
        add: &mut impl FnMut(String, usize, Json),
    ) -> Result<Option<Self>, String> {
        let Some(l) = &object.layout else {
            return Ok(None);
        };
        let ptr = object.ptr_width;
        let capacity = extent_for(l.a_tab)?;
        let extent = extent_for(l.h)?;
        let mut entries = vec![];
        let mut segments = vec![];
        let mut addresses = BTreeMap::new();
        for (name, global) in &l.globals {
            let base = global
                .base
                .to_usize()
                .ok_or_else(|| err("host limit: global base"))?;
            let size = global
                .size
                .to_usize()
                .ok_or_else(|| err("host limit: global size"))?;
            addresses.insert(format!("@{name}"), integer(base.to_string(), env)?);
            entries.push((base, size));
        }
        let mut mapped = f.lists[3].iter().collect::<Vec<_>>();
        mapped.sort_by_key(|p| &p.name);
        for p in mapped {
            let Type::Region(a, w) = p.ty else {
                return Err(err("invalid mapped region"));
            };
            let base = l.interface_regions[&f.name][&p.name]
                .to_usize()
                .ok_or_else(|| err("host limit: mapped base"))?;
            let size = extent_for(a)?
                .checked_mul(w / 8)
                .ok_or_else(|| err("host limit: mapped extent"))?;
            addresses.insert(format!("@{}", p.name), integer(base.to_string(), env)?);
            entries.push((base, size));
            segments.push((base, size, s(&p.name)));
        }
        segments.sort_by_key(|x| x.0);
        let mut image = vec![0u8; extent];
        for global in l.globals.values() {
            let base = global
                .base
                .to_usize()
                .ok_or_else(|| err("host limit: global base"))?;
            let hex = &object.data[global.image.trim_start_matches('@')];
            for (i, pair) in hex.as_bytes().chunks(2).enumerate() {
                image[base + i] =
                    u8::from_str_radix(std::str::from_utf8(pair).map_err(err)?, 16).map_err(err)?;
            }
        }
        add("$oracle.memory".into(), 8 * extent, n(0));
        let mut layout = vec![];
        let mut start = 0;
        for (base, size, cells) in segments
            .into_iter()
            .chain(std::iter::once((extent, 0, s(""))))
        {
            if base > start {
                layout.push(record(
                    start,
                    base - start,
                    range("$oracle.memory", 8 * start, 8 * (base - start)),
                ));
            }
            if size > 0 {
                layout.push(record(base, size, cells));
            }
            start = base + size;
        }
        let mut table = BigUint::zero();
        for (i, (base, size)) in entries.iter().enumerate() {
            let value = BigUint::from_u64(*base as u64)
                .bitor(&BigUint::from_u64(*size as u64).shl(ptr))
                .bitor(&BigUint::from_u64(3).shl(2 * ptr));
            table = table.bitor(&value.shl(i * (2 * ptr + 2)));
        }
        add(
            "$oracle.table".into(),
            capacity * (2 * ptr + 2),
            integer(table.to_decimal(), env)?,
        );
        add("$oracle.atop".into(), l.a_tab + 1, n(entries.len()));
        add("$oracle.used".into(), l.h + 2, n(0));
        Ok(Some(Self {
            ptr,
            h: l.h,
            atop: l.a_tab + 1,
            capacity,
            heap_base: integer(l.heap_base.to_decimal(), env)?,
            heap_limit: integer(heap.to_string(), env)?,
            layout: Json::Arr(layout),
            addresses,
            image: image.iter().map(|b| format!("{b:02x}")).collect(),
        }))
    }
    pub(super) fn row(
        &self,
        i: &Instr,
        widths: &BTreeMap<String, usize>,
        slots: &[Json],
        flag: Json,
        fault: usize,
        env: &mut Bindings,
    ) -> Result<Option<Vec<Json>>, String> {
        let slot = |index: usize, width: usize| -> Result<Json, String> {
            Ok(range(
                "$frame",
                field(&slots[index], "from")?.as_usize().unwrap(),
                width,
            ))
        };
        let tail = || vec![flag.clone(), s("$oracle.faulted"), s("$oracle.code")];
        let views = || {
            vec![
                table_view(self.ptr, self.capacity, 0, self.ptr),
                table_view(self.ptr, self.capacity, self.ptr, self.ptr),
                table_view(self.ptr, self.capacity, 2 * self.ptr, 1),
                table_view(self.ptr, self.capacity, 2 * self.ptr + 1, 1),
                s("$oracle.atop"),
            ]
        };
        let op = i.op.as_str();
        let mut args;
        if op == "lift.addr" {
            return Ok(Some(vec![invoke(
                "lift.addr",
                vec![
                    n(self.ptr),
                    self.addresses
                        .get(&i.operands[0])
                        .ok_or_else(|| err("unknown mapped address"))?
                        .clone(),
                    slot(0, self.ptr)?,
                    s(&i.lhs[0]),
                ],
            )]));
        } else if op == "lift.alloc" {
            let w = widths[&i.operands[0]];
            args = vec![
                n(self.ptr),
                n(w),
                n(self.h + 2),
                n(self.atop),
                n((self.h + 2).max(w) + 1),
                n(self.ptr.min(self.h + 2)),
                n(self.ptr.min(w)),
                n(self.capacity),
                self.heap_base.clone(),
                self.heap_limit.clone(),
                n(fault),
                s(&i.operands[0]),
                s("$oracle.used"),
                s("$oracle.atop"),
                s("$oracle.table"),
                s(&i.lhs[0]),
                slot(0, self.ptr)?,
                slot(1, 2 * self.ptr + 2)?,
                slot(2, self.h + 2)?,
                slot(3, self.atop)?,
            ];
            args.extend(tail());
        } else if matches!(op, "lift.boundscheck" | "lift.free") {
            let w = if op == "lift.free" {
                1
            } else {
                widths[&i.operands[1]]
            };
            args = vec![
                n(self.ptr),
                n(w),
                n(self.ptr.max(w) + 1),
                n(self.capacity),
                n(self.atop),
                n(fault),
                s(&i.operands[0]),
            ];
            if op == "lift.free" {
                let name = format!("$oracle.freezero{}", env.values.len());
                // Free's unused n is a clean zero wire; keep its lifetime local.
                args.push(s(&name));
                args.extend(views());
                args.extend(tail());
                return Ok(Some(vec![
                    Json::obj(vec![
                        ("alloc", s(&name)),
                        ("width", n(1)),
                        ("class", s("any")),
                        ("state", s("clean")),
                    ]),
                    invoke(op, args),
                    Json::obj(vec![("release", s(name))]),
                ]));
            }
            args.push(s(&i.operands[1]));
            args.extend(views());
            args.extend(tail());
        } else if op.starts_with("lift.load.i") || op.starts_with("lift.store.i") {
            let width: usize = op.split(".i").nth(1).unwrap().parse().map_err(err)?;
            let bytes = width / 8;
            let len = (usize::BITS - bytes.leading_zeros()) as usize;
            args = vec![
                n(self.ptr),
                n(len),
                n(self.ptr.max(len) + 1),
                n(self.capacity),
                n(self.atop),
                n(fault),
                n(width),
                n(bytes),
                s(&i.operands[0]),
            ];
            args.extend(views());
            args.extend(tail());
            args.push(s(if i.lhs.is_empty() {
                &i.operands[1]
            } else {
                &i.lhs[0]
            }));
            args.push(slot(0, width)?);
            args.push(self.layout.clone());
            return Ok(Some(vec![invoke(
                if i.lhs.is_empty() {
                    "lift.store"
                } else {
                    "lift.load"
                },
                args,
            )]));
        } else {
            return Ok(None);
        }
        Ok(Some(vec![invoke(op, args)]))
    }
}
fn record(base: usize, extent: usize, cells: Json) -> Json {
    Json::obj(vec![
        ("base", n(base)),
        ("extent", n(extent)),
        ("cells", cells),
    ])
}
