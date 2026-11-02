//! Source occurrence matching precedes cleanup-carrier ownership. This does
//! not discharge §6 admission obligations or instantiate source loop visits.
use super::*;

struct Occurrence {
    index: usize,
    outputs: BTreeSet<String>,
    previous: BTreeMap<String, usize>,
    inputs: BTreeMap<String, usize>,
    matched: bool,
}
pub(super) struct OccurrenceMatches {
    pub inverse: BTreeMap<usize, usize>,
    pub entries: BTreeMap<usize, usize>,
}
impl Source {
    pub(super) fn locations(&self, name: &str) -> Result<BTreeSet<String>, String> {
        let (base, subscript) = name
            .split_once('[')
            .map_or((name, None), |(n, i)| (n, Some(i)));
        let width = if let Some(width) = self.widths.get(base) {
            *width
        } else if let Some(region) = self.regions.get(base) {
            self.region_width(region)?
        } else {
            return Ok(BTreeSet::new());
        };
        if let Some(index) = subscript {
            let index = index
                .strip_suffix(']')
                .ok_or_else(|| err("invalid source version subscript"))?
                .parse::<usize>()
                .map_err(err)?;
            if index >= width {
                return Err(err("source version subscript outside bundle"));
            }
            Ok(BTreeSet::from([format!("{base}[{index}]")]))
        } else {
            Ok((0..width).map(|i| format!("{base}[{i}]")).collect())
        }
    }
    pub(super) fn read_locations(&self, row: &Instr) -> Result<BTreeSet<String>, String> {
        let mut locations = BTreeSet::new();
        for operand in &row.operands {
            for name in
                crate::validate::list_operand(operand).unwrap_or_else(|_| vec![operand.clone()])
            {
                locations.extend(self.locations(&name)?);
            }
        }
        for child in row.nested.iter().flatten() {
            locations.extend(self.read_locations(child)?);
        }
        Ok(locations)
    }
    fn written_locations(&self, row: &Instr, env: &Bindings) -> Result<BTreeSet<String>, String> {
        let mut names = Self::outputs(row, env);
        match row.op.as_str() {
            "qalloc" | "qborrow" | "qcopy" | "qload" => names.extend(row.lhs.iter().cloned()),
            "qfree" | "qreturn" => names.push(row.operands[0].clone()),
            "quncopy" => names.push(row.operands[1].clone()),
            "qunload" => names.push(row.operands[2].clone()),
            _ => {}
        }
        let mut locations = BTreeSet::new();
        for name in names {
            locations.extend(self.locations(&name)?);
        }
        Ok(locations)
    }
    pub(super) fn inverse_occurrences(
        &self,
        rows: &[Instr],
        env: &Bindings,
        adjoint: bool,
    ) -> Result<OccurrenceMatches, String> {
        let mut current = BTreeMap::new();
        let mut occurrences: Vec<Occurrence> = vec![];
        let mut matches = BTreeMap::new();
        let mut entries = BTreeMap::new();
        let mut closed_allocations = BTreeSet::new();
        let mut depth = 0i32;
        let mut scope_outputs = BTreeSet::new();
        let order: Vec<_> = if adjoint {
            (0..rows.len()).rev().collect()
        } else {
            (0..rows.len()).collect()
        };
        for index in order {
            let row = &rows[index];
            let outputs = if matches!(row.op.as_str(), "qpred" | "qunpred") {
                if (row.op == "qpred") != adjoint {
                    depth += 1;
                } else {
                    depth -= 1;
                }
                if depth < 0 {
                    return Err(err("unmatched version-history predicate"));
                }
                if depth != 0 {
                    continue;
                }
                std::mem::take(&mut scope_outputs)
            } else {
                if row.op == "qloop" && row.operands.first().is_some_and(|n| n == "0") {
                    continue;
                }
                let outputs = self.written_locations(row, env)?;
                if depth != 0 {
                    scope_outputs.extend(outputs);
                    continue;
                }
                outputs
            };
            let release_kind = match (adjoint, row.op.as_str()) {
                (false, "qfree") => Some(("qalloc", row.operands[0].as_str())),
                (false, "qreturn") => Some(("qborrow", row.operands[0].as_str())),
                (true, "qalloc") => Some(("qfree", row.lhs[0].as_str())),
                (true, "qborrow") => Some(("qreturn", row.lhs[0].as_str())),
                _ => None,
            };
            if let Some((kind, name)) = release_kind {
                let entry = if let Some(first) = outputs.first() {
                    current
                        .get(first)
                        .copied()
                        .filter(|id| *id != 0)
                        .map(|id| id - 1)
                } else {
                    occurrences.iter().rposition(|prior| {
                        let allocation = &rows[prior.index];
                        allocation.op == kind
                            && !closed_allocations.contains(&prior.index)
                            && if adjoint {
                                allocation.operands.first().is_some_and(|n| n == name)
                            } else {
                                allocation.lhs.first().is_some_and(|n| n == name)
                            }
                    })
                };
                if let Some(id) = entry {
                    let prior = &occurrences[id];
                    if rows[prior.index].op == kind
                        && prior.outputs == outputs
                        && outputs.iter().all(|n| current.get(n) == Some(&(id + 1)))
                    {
                        entries.insert(index, prior.index);
                        closed_allocations.insert(prior.index);
                    }
                }
            }
            let inputs = self
                .read_locations(row)?
                .difference(&outputs)
                .map(|n| (n.clone(), *current.get(n).unwrap_or(&0)))
                .collect::<BTreeMap<_, _>>();
            let previous = outputs
                .iter()
                .map(|n| (n.clone(), *current.get(n).unwrap_or(&0)))
                .collect::<BTreeMap<_, _>>();
            let candidate = if let Some(first) = outputs.first() {
                current
                    .get(first)
                    .copied()
                    .filter(|id| *id != 0)
                    .map(|id| id - 1)
            } else {
                occurrences.iter().rposition(|prior| {
                    !prior.matched
                        && prior.outputs.is_empty()
                        && prior.inputs == inputs
                        && Self::inverse_rows(&rows[prior.index], row)
                })
            };
            let candidate = candidate.filter(|id| {
                let prior = &occurrences[*id];
                !prior.matched
                    && prior.outputs == outputs
                    && prior.inputs == inputs
                    && outputs.iter().all(|n| current.get(n) == Some(&(id + 1)))
                    && Self::inverse_rows(&rows[prior.index], row)
            });
            let next_version = occurrences.len() + 1;
            if let Some(id) = candidate {
                let prior = &mut occurrences[id];
                matches.insert(prior.index, index);
                matches.insert(index, prior.index);
                prior.matched = true;
                for (name, version) in &prior.previous {
                    // A cell write always advances its version (§5.3/§6.1),
                    // including a write restoring the same basis value.
                    let cell = name.split('[').next().unwrap();
                    current.insert(
                        name.clone(),
                        if self.regions.contains_key(cell) {
                            next_version
                        } else {
                            *version
                        },
                    );
                }
            } else {
                for name in &outputs {
                    current.insert(name.clone(), next_version);
                }
            }
            occurrences.push(Occurrence {
                index,
                outputs,
                previous,
                inputs,
                matched: candidate.is_some(),
            });
        }
        if depth != 0 {
            return Err(err("unclosed version-history predicate"));
        }
        Ok(OccurrenceMatches {
            inverse: matches,
            entries,
        })
    }
}
