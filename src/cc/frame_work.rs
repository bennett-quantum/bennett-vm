use super::Frame;

impl Clone for Frame {
    fn clone(&self) -> Self {
        crate::execution_work::record(crate::execution_work::Operation::PayloadClone, 1);
        Self {
            classes_before: self.classes_before.clone(),
            classes_after: self.classes_after.clone(),
            wire_boundary: self.wire_boundary,
            region: self.region.clone(),
            definitions: self.definitions.clone(),
            next_activation: self.next_activation,
            memory_keys: self.memory_keys.clone(),
            data: self.data.clone(),
            after: self.after.clone(),
            control: self.control.clone(),
            pure: self.pure,
            reversed: self.reversed,
            marker: self.marker,
            measured: self.measured,
            tokens_created: self.tokens_created.clone(),
        }
    }
}
