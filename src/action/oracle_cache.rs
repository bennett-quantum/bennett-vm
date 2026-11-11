//! Bounded pure F_B values, admitted only after literal private-state restoration.
use super::*;
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    object: usize,
    callee: String,
    inputs: Vec<BigUint>,
    budget: [u64; 4],
    controls: usize,
}
#[derive(Debug)]
pub(super) struct VerifiedValues {
    values: HashMap<Key, (BigUint, BigUint)>,
    capacity: usize,
}
impl VerifiedValues {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            values: HashMap::new(),
            capacity,
        }
    }
    pub(super) fn outcome(
        &mut self,
        object: &Object,
        callee: &str,
        inputs: &[BigUint],
        budget: Budget,
        controls: usize,
    ) -> Result<(BigUint, BigUint), String> {
        // These invocation checks also run on hits; only pure value calculation is reused.
        crate::certificate::verify_embedded(object, callee, &[], &budget, None)?;
        crate::cliargs::validate_budget(object, &budget)?;
        let key = Key {
            object: object as *const Object as usize,
            callee: callee.into(),
            inputs: inputs.to_vec(),
            budget: [budget.steps, budget.heap, budget.stack, budget.depth],
            controls,
        };
        if let Some(value) = self.values.get(&key) {
            return Ok(value.clone());
        }
        let ordinary = crate::oracle::run_oracle(object, callee, inputs, &mut [], &[], budget)?;
        let mut literal = crate::step::Machine::new(object, callee, inputs, &[], &[], budget)?;
        let entry = literal.state.clone();
        for _ in 0..budget.steps {
            literal.forward_step()?;
        }
        let outcome = literal.outcome();
        for _ in 0..budget.steps {
            literal.inverse_step()?;
        }
        if literal.state != entry || !literal.frames.is_empty() {
            return Err(
                "§5.2: batch oracle cache miss did not restore literal private state/frames".into(),
            );
        }
        if outcome.payload != ordinary.payload || outcome.code != ordinary.code {
            return Err("T6: batch oracle literal and ordinary outcomes differ".into());
        }
        let value = (ordinary.payload, ordinary.code);
        if self.values.len() < self.capacity {
            self.values.insert(key, value.clone());
        }
        Ok(value)
    }
}
