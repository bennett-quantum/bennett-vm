//! Opcode identity is fixed when a source row is parsed.
use bennett_vm::object::{Op, SizedOp};

#[test]
fn opcode_families_keep_width_and_lifted_identity() {
    assert_eq!(Op::from("add"), Op::Add);
    assert_eq!(Op::from("lift.add"), Op::LiftAdd);
    assert!(matches!(
        Op::from("lift.const.i32"),
        Op::Sized {
            kind: SizedOp::Const,
            width: 32,
            lifted: true,
            ..
        }
    ));
    assert!(matches!(
        Op::from("load.i16"),
        Op::Sized {
            kind: SizedOp::Load,
            width: 16,
            lifted: false,
            ..
        }
    ));
    assert!(matches!(Op::from("lift.const.iwrong"), Op::Unknown(_)));
    assert!(matches!(Op::from("lift.ad"), Op::Unknown(_)));
    for name in ["lift.const.i32", "qbranch", "lift.icmp.sle", "qreg.from_id"] {
        assert_eq!(Op::from(name).as_str(), name);
    }
}
