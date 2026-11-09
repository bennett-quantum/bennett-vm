use super::Machine;
impl Clone for Machine {
    fn clone(&self) -> Self {
        crate::execution_work::record(crate::execution_work::Operation::MachineCheckpoint, 1);
        Self {
            program: self.program.clone(),
            indices: self.indices.clone(),
            quantum: self.quantum.clone(),
            quantum_events: self.quantum_events.clone(),
            pending_region: self.pending_region.clone(),
            obj: self.obj.clone(),
            data: self.data.clone(),
            env: self.env.clone(),
            stack: self.stack.clone(),
            history: self.history.clone(),
            trace: self.trace.clone(),
            next_activation: self.next_activation.clone(),
            steps: self.steps.clone(),
            fault: self.fault.clone(),
            results: self.results.clone(),
            entropy: self.entropy.clone(),
            effects: self.effects.clone(),
            draws: self.draws.clone(),
            draw_pos: self.draw_pos.clone(),
            replies: self.replies.clone(),
            reply_pos: self.reply_pos.clone(),
            definitions: self.definitions.clone(),
            stateful: self.stateful.clone(),
            register_changes: self.register_changes.clone(),
            nan_registers: self.nan_registers.clone(),
            unavailable_handle: self.unavailable_handle.clone(),
        }
    }
}
