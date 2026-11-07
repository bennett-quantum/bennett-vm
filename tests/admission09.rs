use crate::source09 as support;
use support::source;
#[test]
fn cc_result_and_block_types_obey_the_source_type_bounds() {
    for fun in [
        "fn cc @f () () () () ()\n^entry():\nx = const.i129 1\nret",
        "fn cc @f (x:i8) () () () ()\n^entry():\ny = zext.i129 x\nret",
        "fn cc @f (q:qreg[0]) () () () ()\n^entry():\nret",
        "fn cc @f () () () () ()\n^entry():\nbr ^b()\n^b(x:i129):\nret",
        "fn cc @f (q:qreg[2]) () () () ()\n^entry():\nx = measure.i1 q\nret",
    ] {
        let text = source(fun);
        assert!(bennett_vm::load(&text, true).is_err(), "admitted {fun}");
    }
}
#[test]
fn cc_names_have_one_type_independent_of_block_order() {
    let bad = source(
        "fn cc @f (c:i1) (result:i8) () () ()\n^entry():\nbr c ^left() ^right()\n^left():\nx = const.i8 1\nbr ^right()\n^right():\nret x",
    );
    // A missing dynamic instance is a runtime fault, not an admission error (§3.4).
    bennett_vm::load(&bad, true).unwrap();
    assert!(bennett_vm::load(&bad.replace("ret x", "ret missing"), true).is_err());
    // Block order need not be a topological order of definitions.
    let good = source(
        "fn cc @f () (result:i8) () () ()\n^entry():\nbr ^define()\n^use():\nret x\n^define():\nx = const.i8 1\nbr ^use()",
    );
    bennett_vm::load(&good, true).unwrap();
}
#[test]
fn qreg_views_are_consumed_on_every_path_without_parent_use() {
    let prelude = "action @a (x:wire mut) () () () ()\n^entry():\nx x\nfn cc @f (q:qreg[2], c:i1) () () () ()\n^entry():\nv = qreg.slice q 0 1\n";
    for tail in [
        "ret",
        "w = qreg.width v\nret",
        "w = qreg.width q\naction_call @a (v) () () ()\nret",
        "action_call @a (v) () () ()\naction_call @a (v) () () ()\nret",
        "br c ^use() ^skip()\n^use():\naction_call @a (v) () () ()\nret\n^skip():\nret",
    ] {
        let text = source(&format!("{prelude}{tail}"));
        assert!(bennett_vm::load(&text, true).is_err(), "admitted {tail}");
    }
    let good = source(&format!(
        "{prelude}br c ^left() ^right()\n^left():\naction_call @a (v) () () ()\nret\n^right():\naction_call @a (v) () () ()\nret"
    ));
    bennett_vm::load(&good, true).unwrap();
}
#[test]
fn same_qreg_name_cannot_fill_two_slots_or_concat_with_itself() {
    let text = source("fn cc @f (q:qreg[2]) () () () ()\n^entry():\nr = qreg.concat q q\nret");
    assert!(bennett_vm::load(&text, true).is_err());
    let text = source(
        "action @a (x:wire mut) (y:wire in) () () ()\n^entry():\ncx y x\nfn cc @f (q:qreg[1]) () () () ()\n^entry():\naction_call @a (q) (q) () ()\nret",
    );
    assert!(bennett_vm::load(&text, true).is_err());
}
#[test]
fn general_header_groups_and_import_types_are_not_coerced() {
    let extra = source("action @a (x:wire mut) () () () ()\n^entry():\nx x")
        .replace("() () () ()\n", "() () () (n:i8)\n");
    assert!(bennett_vm::load(&extra, true).is_err());
    for functions in [
        "action @a (x:wire mut) () () (n:wire) ()\n^entry():\nx x",
        "oracle @o (x:wire in) (t:wire) (f:wire[4]) () (n:reg[1])\n^entry():\nlift.retval x",
        "lproc @f (x:reg[1]) (r:reg[1]) () () ()\nregs { a:reg[0] }\n^entry():\nlift.ret x",
    ] {
        assert!(
            bennett_vm::load(&source(functions), true).is_err(),
            "admitted {functions}"
        );
    }
}
#[test]
fn qreg_transfers_preserve_dynamic_types_and_runtime_width_checks() {
    for functions in [
        "fn cc @f (q:qreg[2]) (r:qreg[*]) () () ()\n^entry():\nret q",
        "fn cc @f (q:qreg[*]) (r:qreg[2]) () () ()\n^entry():\nret q",
        "fn cc @f (q:qreg[*]) () () () ()\n^entry():\nbr ^b(q)\n^b(p:qreg[2]):\nret",
    ] {
        bennett_vm::load(&source(functions), true).unwrap();
    }
    let incompatible = source("fn cc @f (q:qreg[3]) (r:qreg[2]) () () ()\n^entry():\nret q");
    assert!(bennett_vm::load(&incompatible, true).is_err());
}
#[test]
fn all_integer_tokens_use_the_canonical_int_grammar() {
    let good = source(
        "action @a (x:wire[2] mut) () () () ()\nregs { } bounds {2}\n^entry():\nqloop 2 {\nx x[0]\n}",
    );
    for (from, to) in [
        ("ptr_width=32", "ptr_width=032"),
        ("wire[2]", "wire[02]"),
        ("bounds {2}", "bounds {02}"),
        ("qloop 2", "qloop 02"),
        ("x[0]", "x[00]"),
    ] {
        assert!(
            bennett_vm::load(&good.replace(from, to), true).is_err(),
            "admitted {to}"
        );
    }
    let good = source("fn cc @f () () () () ()\n^entry():\nx = const.i8 1\nret");
    for (from, to) in [
        ("const.i8", "const.i08"),
        ("i8 1", "i8 01"),
        ("i8 1", "i8 +1"),
    ] {
        assert!(
            bennett_vm::load(&good.replace(from, to), true).is_err(),
            "admitted {to}"
        );
    }
}
#[test]
fn canonical_empty_data_images_remain_admissible() {
    let text = "bennett-object isa=0.9 object=0.9 launch=0.9 sched=0.9 cert=0.9 coverage=0.9 target=0.9 ptr_width=32\n\nsection symbols\n{\"0\":{\"interface_hash\":\"0000000000000000000000000000000000000000000000000000000000000000\",\"kind\":\"data\",\"name\":\"empty\",\"signature\":{\"abi\":\"0.9\",\"kind\":\"data\",\"size\":0,\"tier\":\"-\"}}}\n\nsection data\nempty \n";
    bennett_vm::load(text, true).unwrap();
}
#[test]
fn modelled_device_rows_require_their_complete_operand_schema() {
    let functions = "fn r @f (p:ptr, n:i64) () () () ()\n^entry():\neffect clock.read p\nret";
    let text = source(functions)
        + "\nsection devices\n{\"models\":{\"clock\":{\"version\":\"0.9\"}},\"services\":{}}\n";
    bennett_vm::load(&text, true).unwrap();
    for (from, to) in [
        ("clock.read p", "clock.read"),
        ("clock.read p", "clock.read p n"),
        ("clock.read p", "clock.read n"),
    ] {
        assert!(
            bennett_vm::load(&text.replace(from, to), true).is_err(),
            "admitted {to}"
        );
    }
}
#[test]
fn qreg_view_names_are_not_confused_with_class_or_opcode_literals() {
    let text = source(
        "action @a (x:wire mut) () () () ()\n^entry():\nx x\nfn cc @f (q:qreg[2]) () () () ()\n^entry():\nany = qreg.slice q 0 1\nr = qreg.new any 1\naction_call @a (any) () () ()\nqreg.free r\nret",
    );
    bennett_vm::load(&text, true).unwrap();
}
