//! Fixed tuple layout, strict input values and arbitrary-width rank enumeration.
use super::*;
struct Component {
    group: &'static str,
    name: String,
    cell: Option<usize>,
    offset: usize,
    width: usize,
}
pub(super) struct Layout {
    pub bits: usize,
    pub record_bytes: usize,
    pub order: Vec<usize>,
    components: Vec<Component>,
    pub muts: Vec<Vec<usize>>,
    pub inputs: Vec<Vec<usize>>,
    pub memory: Vec<RegionWires>,
    pub pool: Vec<usize>,
    pub preserved: Vec<usize>,
}
impl Layout {
    pub fn new(
        object: &Object,
        name: &str,
        sig: &crate::validate::ActionSig,
    ) -> Result<Self, String> {
        let f = object.function(name).ok_or("unresolved batch action")?;
        let mut result = Self {
            bits: 0,
            record_bytes: 0,
            order: vec![],
            components: vec![],
            muts: vec![],
            inputs: vec![],
            memory: vec![],
            pool: vec![],
            preserved: vec![],
        };
        let mut mutable = vec![];
        let mut input = vec![];
        let mut memory = vec![];
        for (group, params) in [("mut", &f.lists[0]), ("in", &f.lists[1])] {
            for p in params {
                let width =
                    p.ty.width()
                        .ok_or("batch requires wire interface bundles")?;
                let index = result.components.len();
                let wires = result.add(group, &p.name, None, width)?;
                if group == "mut" {
                    mutable.push(index);
                    result.muts.push(wires);
                } else {
                    input.push(index);
                    result.preserved.extend(&wires);
                    result.inputs.push(wires);
                }
            }
        }
        for (p, (a, w, role)) in f.lists[2].iter().zip(&sig.mem) {
            let count = u32::try_from(*a)
                .ok()
                .and_then(|a| 1usize.checked_shl(a))
                .ok_or("host limit: batch region cells")?;
            let mut cells = vec![];
            cells
                .try_reserve_exact(count)
                .map_err(|e| format!("host limit: batch region: {e}"))?;
            for cell in 0..count {
                memory.push(result.components.len());
                let wires = result.add("mem", &p.name, Some(cell), *w)?;
                if role == "in" {
                    result.preserved.extend(&wires);
                }
                cells.push(wires);
            }
            result.memory.push(RegionWires {
                a: *a,
                w: *w,
                cells,
            });
        }
        let pool = crate::qcli::borrow_capacity(object, name)?;
        let pool_index = result.components.len();
        if pool > 0 {
            result.pool = result.add("borrowed", "$borrowed", None, pool)?;
            result.preserved.extend(&result.pool);
        }
        result.order.extend(input);
        result.order.extend(mutable);
        result.order.extend(memory);
        if pool > 0 {
            result.order.push(pool_index);
        }
        result.record_bytes = result
            .bits
            .div_ceil(8)
            .checked_mul(2)
            .and_then(|n| n.checked_add(2))
            .ok_or("host limit: batch record size")?;
        Ok(result)
    }
    fn add(
        &mut self,
        group: &'static str,
        name: &str,
        cell: Option<usize>,
        width: usize,
    ) -> Result<Vec<usize>, String> {
        let end = self
            .bits
            .checked_add(width)
            .ok_or("host limit: batch tuple width")?;
        let mut wires = vec![];
        wires
            .try_reserve_exact(width)
            .map_err(|e| format!("host limit: batch bundle: {e}"))?;
        wires.extend(self.bits..end);
        self.components.push(Component {
            group,
            name: name.into(),
            cell,
            offset: self.bits,
            width,
        });
        self.bits = end;
        Ok(wires)
    }
    pub fn description(&self) -> Json {
        Json::Arr(
            self.components
                .iter()
                .map(|c| {
                    Json::obj(vec![
                        ("group", text(c.group)),
                        ("name", text(&c.name)),
                        ("cell", c.cell.map_or(Json::Null, size)),
                        ("offset", size(c.offset)),
                        ("width", size(c.width)),
                    ])
                })
                .collect(),
        )
    }
    pub fn rank(&self, index: &BigUint) -> BigUint {
        let mut remaining = index.clone();
        let mut key = BigUint::zero();
        for slot in self.order.iter().rev() {
            let c = &self.components[*slot];
            key = key.add(&remaining.mask(c.width).shl(c.offset));
            remaining = remaining.shr(c.width);
        }
        key
    }
    pub fn parse(&self, line: &str) -> Result<BigUint, String> {
        if line.trim().is_empty() {
            return Err("blank input line".into());
        }
        let value = json::parse(line)?;
        let object = value.as_obj().ok_or("tuple must be an object")?;
        if object.len() != 4
            || object
                .keys()
                .any(|k| !["mut", "in", "mem", "borrowed"].contains(&k.as_str()))
        {
            return Err("tuple requires exactly mut, in, mem, borrowed".into());
        }
        let array = |name| {
            object
                .get(name)
                .and_then(Json::as_arr)
                .ok_or_else(|| format!("{name} must be an array"))
        };
        let mutable = array("mut")?;
        let inputs = array("in")?;
        let pool = array("borrowed")?;
        if mutable.len() != self.muts.len()
            || inputs.len() != self.inputs.len()
            || pool.len() != usize::from(!self.pool.is_empty())
        {
            return Err("tuple component count mismatch".into());
        }
        let memory = object["mem"].as_obj().ok_or("mem must be an object")?;
        let names = self
            .components
            .iter()
            .filter(|c| c.group == "mem")
            .map(|c| c.name.as_str())
            .collect::<BTreeSet<_>>();
        if memory.len() != names.len() || memory.keys().any(|n| !names.contains(n.as_str())) {
            return Err("memory region names differ from interface".into());
        }
        for name in &names {
            let count = self
                .components
                .iter()
                .filter(|c| c.group == "mem" && c.name == *name)
                .count();
            if memory[*name].as_arr().is_none_or(|a| a.len() != count) {
                return Err("memory cell count mismatch".into());
            }
        }
        let mut key = BigUint::zero();
        let (mut m, mut i) = (0, 0);
        for c in &self.components {
            let value = match c.group {
                "mut" => {
                    let v = &mutable[m];
                    m += 1;
                    v
                }
                "in" => {
                    let v = &inputs[i];
                    i += 1;
                    v
                }
                "mem" => &memory[&c.name].as_arr().unwrap()[c.cell.unwrap()],
                "borrowed" => &pool[0],
                _ => unreachable!(),
            };
            let value = uint(value)?;
            if value.bits() > c.width {
                return Err("tuple value exceeds declared width".into());
            }
            key = key.add(&value.shl(c.offset));
        }
        Ok(key)
    }
}
