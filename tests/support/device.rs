use bennett_vm::json::{self, Json};
use std::path::Path;
pub fn block_object(full: bool) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let raw = std::fs::read_to_string(
        root.join("bennett-isa-spec/reference-interpreter/tests/fixtures/classical.bisa"),
    )
    .unwrap();
    let from = raw.find("fn r @effects ").unwrap();
    let to = raw[from..].find("fn r @impure ").unwrap() + from;
    let raw = format!(
        "{}fn r @effects () (r:i64) () () ()\n^entry():\nn = const.i16 1024\np = alloc n\nsentinel = const.i16 65535\nstore.i16 p sentinel\nsector = const.i64 0\neffect block.read sector p\nr = load.i64 p\nret r\n{}",
        &raw[..from],
        &raw[to..]
    );
    let (obj, ids) = bennett_vm::load(&raw, true).unwrap();
    let mut symbols = Json::Obj(
        obj.symbols
            .values()
            .map(|s| {
                (
                    s.index.to_string(),
                    Json::obj(vec![
                        ("interface_hash", Json::Str(ids.interface[&s.name].clone())),
                        ("kind", Json::Str(s.kind.clone())),
                        ("name", Json::Str(s.name.clone())),
                        ("signature", s.signature.clone()),
                    ]),
                )
            })
            .collect(),
    );
    if let Json::Obj(m) = &mut symbols {
        m.insert(
            obj.symbols.len().to_string(),
            Json::obj(vec![
                ("interface_hash", Json::Str("0".repeat(64))),
                ("kind", Json::Str("data".into())),
                ("name", Json::Str("sector_image".into())),
                (
                    "signature",
                    json::parse(r#"{"abi":"0.8","kind":"data","size":512,"tier":"-"}"#).unwrap(),
                ),
            ]),
        );
    }
    let mut lines = raw.lines().map(String::from).collect::<Vec<_>>();
    let i = lines.iter().position(|l| l == "section symbols").unwrap() + 1;
    lines[i] = symbols.to_jcs();
    let i = lines.iter().position(|l| l == "section layout").unwrap() + 1;
    let mut layout = json::parse(&lines[i]).unwrap();
    if let Json::Obj(m) = &mut layout {
        m.insert("block_image".into(), Json::Str("sector_image".into()));
    }
    lines[i] = layout.to_jcs();
    let devices = lines.iter().position(|l| l == "section devices").unwrap();
    lines.splice(
        devices..devices,
        [
            "section data".into(),
            format!(
                "sector_image a5{}",
                if full {
                    "00".repeat(511)
                } else {
                    String::new()
                }
            ),
            "".into(),
        ],
    );
    // Data is part of the semantic envelope: rehash once after inserting the image.
    let raw = lines.join("\n") + "\n";
    let raw = if full {
        raw
    } else {
        raw.replace("\"size\":512", "\"size\":1")
    };
    let obj = bennett_vm::object::parse(&raw).unwrap();
    let ids = bennett_vm::identity::compute(&obj).unwrap();
    let mut lines = raw.lines().map(String::from).collect::<Vec<_>>();
    let i = lines.iter().position(|l| l == "section symbols").unwrap() + 1;
    lines[i] = Json::Obj(
        obj.symbols
            .values()
            .map(|s| {
                (
                    s.index.to_string(),
                    Json::obj(vec![
                        ("interface_hash", Json::Str(ids.interface[&s.name].clone())),
                        ("kind", Json::Str(s.kind.clone())),
                        ("name", Json::Str(s.name.clone())),
                        ("signature", s.signature.clone()),
                    ]),
                )
            })
            .collect(),
    )
    .to_jcs();
    lines.join("\n") + "\n"
}
