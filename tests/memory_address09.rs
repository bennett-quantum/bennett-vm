//! Address-only () and indexed scalar addresses retain their exact wire sets.
#[path = "support/source09.rs"]
mod support;
use bennett_vm::{
    action::{QuantumState, RegionWires, execute_action_inner},
    bigint::BigUint,
    json::Json,
    ref_library::Catalog,
};

fn source(address_bits: usize, address: &str) -> String {
    support::source(&format!(
        "action @root (t:wire[2] mut) (a:wire[2] in) (r:region[{address_bits},2] mut) () ()\n^entry():\nd = qload r {address}\nqxor d t\nqxor d t\nqunload r {address} d\nqstore r {address} t\nqstore r {address} t\nqexchm r {address} t\nqexchm r {address} t"
    ))
}

#[test]
fn zero_and_indexed_addresses_execute_all_memory_rows_in_both_directions() {
    let catalog = Catalog::published().unwrap();
    for (address_bits, address) in [(0, "()"), (1, "a[1]"), (2, "a")] {
        let (object, _) = bennett_vm::load(&source(address_bits, address), false).unwrap();
        for adjoint in [false, true] {
            assert!(
                !catalog
                    .lower_action(&object, "root", &Json::Arr(vec![]), adjoint, 0)
                    .unwrap()
                    .is_empty()
            );
            for bits in 0..4 {
                for target in 0..4 {
                    for memory in 0..4 {
                        for control in [false, true] {
                            let mut state = QuantumState::new();
                            let t = state.alloc_wires(2);
                            let a = state.alloc_wires(2);
                            let cells = (0..1usize << address_bits)
                                .map(|_| state.alloc_wires(2))
                                .collect::<Vec<_>>();
                            let c = state.alloc_wire();
                            state.prepare(&a, &BigUint::from_u64(bits));
                            state.prepare(&t, &BigUint::from_u64(target));
                            for (cell, wires) in cells.iter().enumerate() {
                                state
                                    .prepare(wires, &BigUint::from_u64((memory + cell as u64) % 4));
                            }
                            if control {
                                state.flip(c, &[]);
                            }
                            let before = state.clone();
                            execute_action_inner(
                                &object,
                                &mut state,
                                "root",
                                &[t],
                                &[a],
                                &[RegionWires {
                                    a: address_bits,
                                    w: 2,
                                    cells,
                                }],
                                &[],
                                adjoint,
                                &[c],
                            )
                            .unwrap();
                            assert!(
                                state.exact_equals(&before),
                                "A={address_bits}, address={bits}, target={target}, memory={memory}, control={control}, adjoint={adjoint}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn empty_address_is_only_an_address_of_a_zero_bit_region() {
    for (address_bits, address) in [(0, "a[0]"), (1, "()"), (1, "a"), (1, "a[2]"), (2, "a[1]")] {
        let error = bennett_vm::load(&source(address_bits, address), false).unwrap_err();
        assert!(
            error.contains("§3.3"),
            "A={address_bits}, {address}: {error}"
        );
    }
}
