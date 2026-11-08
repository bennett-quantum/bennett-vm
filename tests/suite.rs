//! VM integration suites, linked once. Slow cases remain mandatory in the gate.
mod action_support09;
mod address09;
mod admission09;
mod allocation_bindings09;
mod bound_calls09;
mod certificates;
mod classical09;
mod classical09_launch;
mod cleanup09;
mod cleanup_fresh09;
mod cleanup_multirow09;
mod cleanup_partition09;
mod coherent;
mod coherent_typing09;
mod conformance;
mod devices;
mod differential;
mod eps_decade;
mod estimate09;
mod frame_domain;
mod interval;
mod isa08_reference;
mod jit;
mod json_numbers;
mod launch_c;
mod link09;
mod measurement09;
mod measurement_exact09;
mod memory_address09;
mod memory_files;
mod object09;
mod opcodes;
mod oracle_address09;
mod oracle_cleanup09;
#[path = "support/pins.rs"]
mod pins;
mod ptr_icmp;
mod qif_adjoint09;
mod ref09;
mod region_alias09;
mod replay09;
mod reports;
mod saved_tape;
mod schedules;
mod segment_instances09;
mod source_address09;
mod target_execution;
mod targets;
mod token_partitions09;
mod validation_order;

#[path = "support/device.rs"]
mod device_fixture;
#[path = "support/source09.rs"]
mod source09;
