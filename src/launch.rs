//! One-shot §10 launch. The stateful C ABI follows in milestone 2.
pub fn cmd_run_program(args: &[String]) -> Result<(), String> {
    crate::cc::cmd_program(args)
}
