//! Reusable immutable action setup; execution always starts from caller state.
use super::*;
use std::borrow::Cow;

pub(crate) struct PreparedAction<'o> {
    object: &'o Object,
    name: String,
    function: Cow<'o, Function>,
    bundles: HashMap<String, Vec<usize>>,
    regions: HashMap<String, RegionWires>,
    interface: BTreeSet<String>,
    active: BTreeSet<usize>,
    idle: Vec<usize>,
    controls: Vec<usize>,
    forward: Vec<Instr>,
    inverse: Vec<Instr>,
    declarations: Vec<Instr>,
    releases: allocation_bindings::ReleasePlan,
}
impl<'o> PreparedAction<'o> {
    pub(crate) fn new(
        object: &'o Object,
        name: &str,
        muts: &[Vec<usize>],
        inputs: &[Vec<usize>],
        memory: &[RegionWires],
        controls: &[usize],
        idle: &[usize],
    ) -> Result<Self, String> {
        let f = object.function(name).ok_or_else(|| {
            diag(
                "5.3",
                format!("action @{name} is imported; executing an unresolved import needs linking"),
            )
        })?;
        let function = if object.isa == "0.9" {
            Cow::Owned(bind_allocations(f))
        } else {
            Cow::Borrowed(f)
        };
        if !matches!(
            function.kind,
            crate::object::FnKind::Action | crate::object::FnKind::PermAction
        ) {
            return Err(diag("5.3", format!("@{name} is not an action")));
        }
        let sig = crate::validate::action_sig(object, name)?;
        if !action_binding_valid(&sig, muts, inputs, memory, controls) {
            return Err(diag(
                "3.3",
                "invalid action wire binding (width, shape or overlap)",
            ));
        }
        let mut bundles = HashMap::new();
        for (p, wires) in function.lists[0]
            .iter()
            .zip(muts)
            .chain(function.lists[1].iter().zip(inputs))
        {
            bundles.insert(p.name.clone(), wires.clone());
        }
        let regions = function.lists[2]
            .iter()
            .zip(memory)
            .map(|(p, r)| (p.name.clone(), r.clone()))
            .collect();
        let interface = bundles.keys().cloned().collect();
        let active = muts
            .iter()
            .chain(inputs)
            .flatten()
            .copied()
            .chain(
                memory
                    .iter()
                    .flat_map(|r| r.cells.iter().flatten().copied()),
            )
            .chain(controls.iter().copied())
            .collect();
        let forward: Vec<Instr> = function
            .blocks
            .iter()
            .flat_map(|b| b.instrs.clone())
            .collect();
        let widths = bundle_widths(object, &function);
        let inverse = adjoint_list(&forward, &widths, &object.isa);
        let mut declarations = vec![];
        for instruction in &forward {
            collect_qregions(instruction, &mut |q| declarations.push(q.clone()));
        }
        let releases = if object.isa == "0.9" {
            allocation_bindings::release_metadata(&function)
        } else {
            BTreeMap::new()
        };
        Ok(Self {
            object,
            name: name.into(),
            function,
            bundles,
            regions,
            interface,
            active,
            idle: idle.to_vec(),
            controls: controls.to_vec(),
            forward,
            inverse,
            declarations,
            releases,
        })
    }
    pub(crate) fn execute(
        &self,
        state: &mut QuantumState,
        args: &[Value],
        adjoint: bool,
    ) -> Result<(), String> {
        let budget = args
            .iter()
            .find_map(|v| match v {
                Value::Token(b) | Value::TokenId(_, b) => Some(*b),
                _ => None,
            })
            .unwrap_or_default();
        crate::certificate::verify_embedded(
            self.object,
            &self.name,
            args,
            &budget,
            state.target.as_deref(),
        )?;
        let unique = self.idle.iter().copied().collect::<BTreeSet<_>>();
        if unique.len() != self.idle.len()
            || self.idle.iter().any(|w| {
                *w >= state.next_wire
                    || self.active.contains(w)
                    || (state.target.is_some() && !state.wire_classes.contains_key(w))
            })
        {
            return Err(
                "§9.4: invalid designated idle set (duplicates, nonlive or active interface)"
                    .into(),
            );
        }
        if let Some(target) = &state.target {
            target.validate_function(&self.function)?;
        }
        let args = self.function.lists[3]
            .iter()
            .zip(args)
            .map(|(p, v)| (p.name.clone(), v.clone()))
            .collect();
        let mut executor = Exec {
            obj: self.object,
            state,
            bundles: self.bundles.clone(),
            regions: self.regions.clone(),
            local_images: BTreeMap::new(),
            args,
            pred: self.controls.clone(),
            borrowed: vec![],
            idle: self.idle.clone(),
            func: self.name.clone(),
            exact_required: self.function.kind == crate::object::FnKind::PermAction,
            releases: if adjoint {
                self.releases.clone()
            } else {
                BTreeMap::new()
            },
            iterations: vec![],
        };
        for declaration in &self.declarations {
            executor.create_region(declaration)?;
        }
        executor.exec_list(if adjoint {
            &self.inverse
        } else {
            &self.forward
        })?;
        executor.release_local_regions()?;
        if executor.pred.len() != self.controls.len() {
            return Err(diag(
                "3.5",
                format!("predicate stack of @{} is not restored at exit", self.name),
            ));
        }
        let leaked: Vec<_> = executor
            .bundles
            .keys()
            .filter(|k| !self.interface.contains(*k))
            .collect();
        if !leaked.is_empty() {
            return Err(diag(
                "5.1",
                format!(
                    "action @{} exits with unmatched allocations {leaked:?}",
                    self.name
                ),
            ));
        }
        Ok(())
    }
}
