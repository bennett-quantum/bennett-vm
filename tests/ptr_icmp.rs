//! §3.3/§4.8: pointer comparisons retain the classical operand type contract.
fn witness(body: &str) -> String {
    format!(
        "bennett-object isa=0.8 object=0.8 launch=0.8 sched=0.8 cert=0.8 coverage=0.8 target=0.8 ptr_width=64\n\nsection symbols\n{{\"0\":{{\"interface_hash\":\"c4ae142003000705977d4309b9f634bac009bc6ee09ee3a31504c140c5dce457\",\"kind\":\"cc\",\"name\":\"ht_demo_basic\",\"signature\":{{\"abi\":\"0.8\",\"kind\":\"cc\",\"params\":[\"i64\"],\"results\":[\"i64\"],\"tier\":\"cc\"}}}}}}\n\nsection functions\nfn cc @ht_demo_basic (a0:i64) (result:i64) () () ()\n^b0():\n{body}\nret a0\n\nsection layout\n{{\"A_tab\":20,\"H\":21,\"S_cells\":16,\"globals\":{{}},\"heap_base\":4096,\"heap_limit_max\":1048576,\"interface_regions\":{{}},\"null_size\":4096,\"ptr_width\":64}}\n"
    )
}

#[test]
fn pointer_and_integer_comparisons_are_admitted() {
    for pred in ["eq", "ne", "ult", "ule", "slt", "sle"] {
        for operand in ["v1", "v2"] {
            let body =
                format!("v1 = const.i64 0\nv2 = inttoptr v1\nv3 = icmp.{pred} {operand} {operand}");
            bennett_vm::load(&witness(&body), true).unwrap();
        }
    }
}

#[test]
fn classical_type_errors_identify_the_instruction() {
    for row in ["v3 = add v2 v2", "v3 = icmp.eq v1 v2", "v3 = inttoptr v2"] {
        let text = witness(&format!("v1 = const.i64 0\nv2 = inttoptr v1\n{row}"));
        let error = bennett_vm::load(&text, false).unwrap_err();
        let line = text.lines().position(|l| l == row).unwrap() + 1;
        assert!(error.contains("§3.3"), "{error}");
        assert!(error.contains("@ht_demo_basic ^b0"), "{error}");
        assert!(error.contains(&format!("line {line}")), "{error}");
        assert!(error.contains(row), "{error}");
    }
    let error = bennett_vm::load(
        &witness("v1 = const.i8 0\nv2 = const.i64 0\nv3 = icmp.eq v1 v2"),
        false,
    )
    .unwrap_err();
    assert!(error.contains("classical operand types differ"), "{error}");
}
