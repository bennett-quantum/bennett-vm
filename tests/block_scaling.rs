use std::cell::Cell;
thread_local! {
    static WORK: Cell<Option<u64>> = const { Cell::new(None) };
}
pub(crate) fn record(n: u64) {
    WORK.with(|w| w.set(w.get().map(|total| total + n)));
}
struct Probe;
impl Probe {
    fn start() -> Self {
        WORK.with(|w| assert!(w.replace(Some(0)).is_none()));
        Self
    }
    fn visits(&self) -> u64 {
        WORK.with(|w| w.get().unwrap())
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        WORK.with(|w| w.set(None));
    }
}
// Branch validation must inspect a bounded number of block names per row.
use crate::{json::Json, object, validate};

fn object(blocks: usize) -> object::Object {
    let mut body = String::from("fn cc @main () () () () ()\n");
    for i in 0..blocks {
        body.push_str(&format!("^b{i}():\n"));
        if i + 1 == blocks {
            body.push_str("ret\n");
        } else {
            body.push_str(&format!("br ^b{}\n", i + 1));
        }
    }
    let mut obj = object::parse(&format!(
        "bennett-object isa=0.9 object=0.9 launch=0.9 sched=0.9 cert=0.9 coverage=0.9 target=0.9 ptr_width=32\n\nsection symbols\n{{}}\n\nsection functions\n{body}"
    )).unwrap();
    obj.symbols.insert(
        "main".into(),
        object::Symbol {
            index: 0,
            name: "main".into(),
            kind: "cc".into(),
            signature: Json::Null,
            stored_hash: "0".repeat(64),
        },
    );
    obj.symbols.get_mut("main").unwrap().signature =
        validate::derived_signature(&obj, "main").unwrap();
    obj
}

#[test]
fn branch_validation_work_is_linear_in_blocks() {
    for n in [128, 1024] {
        let obj = object(n);
        let probe = Probe::start();
        validate::validate(&obj).unwrap();
        let visits = probe.visits();
        assert!(
            visits <= 6 * n as u64,
            "{visits} block names inspected for {n} rows"
        );
    }
}

#[test]
fn indexed_lookup_preserves_first_match_and_rebuilds_after_edits() {
    let mut obj = object(8);
    let f = &mut obj.functions[0];
    let mut duplicate = f.blocks[0].clone();
    duplicate.line += 100;
    f.blocks.push(duplicate);
    {
        let index = object::BlockIndex::new(f);
        for name in ["b0", "b7", "missing"] {
            assert_eq!(
                index.get(name).map(|b| b.line),
                f.block(name).map(|b| b.line)
            );
        }
    }
    f.blocks[0].name = "renamed".into();
    let index = object::BlockIndex::new(f);
    for name in ["b0", "renamed", "missing"] {
        assert_eq!(
            index.get(name).map(|b| b.line),
            f.block(name).map(|b| b.line)
        );
    }
}

#[test]
fn retained_reverse_validates_the_dispatched_object_once() {
    let obj = object(128);
    let ids = crate::identity::compute(&obj).unwrap();
    let symbol = &obj.symbols["main"];
    let symbols = Json::obj(vec![(
        "0",
        Json::obj(vec![
            ("name", Json::Str("main".into())),
            ("kind", Json::Str("cc".into())),
            ("signature", symbol.signature.clone()),
            ("interface_hash", Json::Str(ids.interface["main"].clone())),
        ]),
    )]);
    let text = obj
        .text
        .replacen("\n{}\n", &format!("\n{}\n", symbols.to_jcs()), 1);
    let path = std::env::temp_dir().join(format!("reverse-load-{}.bisa", std::process::id()));
    std::fs::write(&path, text).unwrap();
    let probe = Probe::start();
    let result = crate::cc::cmd_reverse(&[
        path.to_str().unwrap().into(),
        "main".into(),
        "--steps".into(),
        "0".into(),
    ]);
    std::fs::remove_file(path).unwrap();
    result.unwrap();
    let visits = probe.visits();
    assert!(
        visits <= 5 * 128,
        "{visits} block visits: dispatched object was validated twice"
    );
}
