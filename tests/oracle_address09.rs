//! K64 annotates the authenticated stream; raw descriptors are not source admission.
#[path = "support/source09.rs"]
mod support;
use bennett_vm::{
    json::{self, Json},
    ref_library::{Bindings, Catalog, Leaf},
};
fn j(s: &str) -> Json {
    json::parse(s).unwrap()
}
fn fixture(steps: usize, addressed: bool, inverse: bool) -> Vec<Leaf> {
    let text = support::source(
        "oracle @o (x:wire[1] in) (t:wire[1]) (f:wire[4]) () ()\n^entry():\nlift.retval x",
    );
    let (obj, _) = bennett_vm::load(&text, false).unwrap();
    let mut env = Bindings::new();
    env.wire("tgt", 1);
    env.wire("fr", 4);
    env.wire("x", 1);
    if addressed {
        env.address_oracle(&obj, "o").unwrap();
    }
    let names = ["pc", "next", "faulted", "code", "result"];
    let mut state = vec![];
    let mut declarations = vec![];
    for (name, width) in names
        .into_iter()
        .map(|n| (n.to_string(), if n == "code" { 4 } else { 1 }))
        .chain((0..steps).map(|i| (format!("frame{i}"), 1)))
    {
        state.push(Json::obj(vec![
            ("width", Json::u64(width)),
            ("initial", j("0")),
            ("I", j("\"\"")),
            ("storage", Json::Str(name.clone())),
        ]));
        declarations.push((name, width));
    }
    let frames = (0..steps)
        .map(|i| {
            Json::obj(vec![
                ("index", Json::u64(i as u64)),
                ("frame", Json::Str(format!("frame{i}"))),
            ])
        })
        .collect();
    let mut body = j(&format!(
        r#"[{{"invoke":"oracle.construct","args":[1,1,1,{steps},4,1,"pc","next","faulted","code","result","tgt","fr",{},{},[{{"parameters":[{{"name":"f","type":"wire","width":1}}],"body":[{{"gate":"x","wires":["x"]}}]}}],[{{"parameters":[{{"name":"f","type":"wire","width":1}},{{"name":"n","type":"wire","width":1}}],"body":[{{"gate":"x","wires":["x"]}}]}}]]}}]"#,
        Json::Arr(state).to_jcs(),
        Json::Arr(frames).to_jcs()
    ));
    if inverse {
        body = Json::Arr(vec![Json::obj(vec![
            ("inverse", Json::Bool(true)),
            ("body", body),
        ])]);
    }
    for (name, width) in declarations.into_iter().rev() {
        body = Json::Arr(vec![Json::obj(vec![
            ("declare", Json::Str(name)),
            ("width", Json::u64(width)),
            ("class", j(r#"{"class":"any"}"#)),
            ("state", j("\"clean\"")),
            ("body", body),
        ])]);
    }
    env.body("B", body).unwrap();
    Catalog::published()
        .unwrap()
        .expand("adj", &[j("\"B\"")], &env)
        .unwrap()
}
#[test]
fn k64_zero_budget_retains_nonstep_support_and_raw_stream() {
    for steps in 0..=2 {
        let addressed = fixture(steps, true, false);
        let raw = fixture(steps, false, false);
        assert_eq!(addressed.len(), raw.len());
        for (a, b) in addressed.iter().zip(raw) {
            let mut a = a.clone();
            a.address = None;
            assert_eq!(a, b, "addressing cannot change any emitted event");
        }
        for point in ["$construct", "$commit", "$release"] {
            let group = addressed
                .iter()
                .filter(|l| l.address.as_ref().unwrap().path[0].point == point)
                .collect::<Vec<_>>();
            assert!(!group.is_empty());
            for l in group {
                let a = l.address.as_ref().unwrap();
                assert_eq!(a.step, None);
                assert_eq!(a.phase, "");
                assert!(a.path[0].iteration.is_empty());
                assert!(a.path[0].instance.is_empty());
            }
        }
        assert_eq!(
            addressed
                .iter()
                .filter(|l| l.address.as_ref().unwrap().path[0].point == "$step")
                .count()
                > 0,
            steps > 0
        );
    }
}
#[test]
fn k64_step_support_excludes_rows_and_rows_next_share_ordinals() {
    let leaves = fixture(2, true, false);
    for t in 1..=2 {
        for phase in ["fwd", "rev"] {
            let group = leaves
                .iter()
                .filter(|l| {
                    let a = l.address.as_ref().unwrap();
                    a.step == Some(t) && a.phase == phase
                })
                .collect::<Vec<_>>();
            let mut ord = group
                .iter()
                .filter_map(|l| {
                    let a = l.address.as_ref().unwrap();
                    (a.path[0].point == "$step").then_some(a.generated)
                })
                .collect::<Vec<_>>();
            ord.sort();
            assert_eq!(ord, (0..ord.len()).collect::<Vec<_>>());
            let row = group
                .iter()
                .filter(|l| l.address.as_ref().unwrap().path[0].point == "entry:0")
                .collect::<Vec<_>>();
            assert_eq!(row.len(), 2);
            assert_eq!(
                row.iter()
                    .map(|l| l.address.as_ref().unwrap().generated)
                    .collect::<Vec<_>>(),
                if phase == "fwd" {
                    vec![0, 1]
                } else {
                    vec![1, 0]
                }
            );
            let zero = group
                .iter()
                .find(|l| {
                    let a = l.address.as_ref().unwrap();
                    a.path[0].point == "$step" && a.generated == 0
                })
                .unwrap();
            assert_eq!(
                zero.operation,
                if phase == "fwd" { "alloc" } else { "release" }
            );
        }
    }
    let reversed = fixture(2, true, true);
    for (f, r) in leaves.iter().rev().zip(reversed) {
        let mut a = f.address.clone().unwrap();
        a.kind = r.address.as_ref().unwrap().kind;
        assert_eq!(Some(a), r.address);
    }
}
