//! Batch transport mirrors the pinned CLI contract, not interpreter implementation.
use bennett_vm::{
    bigint::BigUint as U,
    json::{self, Json},
};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};
fn fixture_at(version: &str, body: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = PathBuf::from(std::env::var_os("TMPDIR").unwrap_or_else(|| ".tmp".into()))
        .join("batch-reverse-tests");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join(format!(
        "{}-{}.bisa",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut raw = crate::source09::source_at(version, body);
    if body.contains("region[") && !raw.contains("section layout") {
        raw.push_str("\nsection layout\n{\"A_tab\":2,\"H\":6,\"S_cells\":4,\"globals\":{},\"heap_base\":32,\"heap_limit_max\":32,\"interface_regions\":{},\"null_size\":16,\"ptr_width\":32}\n");
        raw = crate::source09::sign(&raw);
    }
    std::fs::write(&path, raw).unwrap();
    path
}
fn fixture(body: &str) -> PathBuf {
    fixture_at("0.8", body)
}
fn arithmetic(op: &str) -> PathBuf {
    fixture(&format!(
        "oracle @f (x:wire[8] in, y:wire[8] in) (t:wire[8]) (fr:wire[4]) () ()\nregs {{ z:reg[8] }}\n^entry():\nz = lift.{op} x y\nlift.retval z\naction @full (t:wire[8] mut, f:wire[4] mut) (x:wire[8] in, y:wire[8] in) () (tk:token) ()\n^entry():\noracle_call @f tk (x, y) t f () ()"
    ))
}
fn vm() -> PathBuf {
    env!("CARGO_BIN_EXE_bennett-vm").into()
}
fn reference() -> PathBuf {
    let config = include_str!("../gate.toml");
    let section = config
        .split("[siblings.\"bennett-batch-reference\"]")
        .nth(1)
        .unwrap();
    let sha = section
        .lines()
        .find(|s| s.starts_with("sha ="))
        .unwrap()
        .split('"')
        .nth(1)
        .unwrap();
    let root = std::env::var_os("BENNETT_BATCH_ISA_SPEC")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                ".tmp/gate/cache/bennett-batch-reference@{}/bennett-batch-reference",
                &sha[..12]
            ))
        });
    let head = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(head.status.success());
    assert_eq!(String::from_utf8(head.stdout).unwrap().trim(), sha);
    root.join("reference-interpreter/target/release/bennett-isa-ref")
}
fn run(
    bin: &Path,
    object: &Path,
    action: &str,
    options: &[&str],
    input: &str,
    engine: &str,
) -> Output {
    let mut child = Command::new(bin)
        .arg("reverse-action-batch")
        .arg(object)
        .arg(action)
        .args(options)
        .env("BENNETT_VM_ENGINE", engine)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}
#[derive(Debug, PartialEq, Eq)]
struct Case {
    index: String,
    flags: u8,
    forward: Option<Vec<u8>>,
    reversed: Option<Vec<u8>>,
}
fn decode(output: &[u8]) -> (Json, Vec<Case>, Json) {
    let end = output.iter().position(|b| *b == b'\n').expect("header LF");
    let header = json::parse(std::str::from_utf8(&output[..end]).unwrap()).unwrap();
    let mut cases = vec![];
    if header.get("encoding").unwrap().as_str() == Some("jsonl") {
        let mut footer = None;
        for line in std::str::from_utf8(&output[end + 1..]).unwrap().lines() {
            let value = json::parse(line).unwrap();
            if let Json::Arr(a) = value {
                assert_eq!(a.len(), 5);
                assert_eq!(a[0].as_str(), Some("case"));
                let bytes = |v: &Json| v.as_str().map(|s| bennett_vm::sha256::from_hex(s).unwrap());
                cases.push(Case {
                    index: a[1].as_str().unwrap().into(),
                    flags: a[2].as_u64().unwrap() as u8,
                    forward: bytes(&a[3]),
                    reversed: bytes(&a[4]),
                });
            } else {
                assert!(footer.is_none());
                footer = Some(value);
            }
        }
        return (header, cases, footer.expect("complete footer"));
    }
    let bytes = header
        .get("domain_bits")
        .unwrap()
        .as_usize()
        .unwrap()
        .div_ceil(8);
    assert_eq!(
        header.get("record_bytes").unwrap().as_usize(),
        Some(2 + 2 * bytes)
    );
    let mut rank = header
        .get("lo")
        .and_then(Json::as_str)
        .map(|s| U::parse_decimal(s).unwrap())
        .unwrap_or_default();
    let mut cursor = end + 1;
    while output.get(cursor) == Some(&1) {
        let flags = output[cursor + 1];
        let forward = output[cursor + 2..cursor + 2 + bytes].to_vec();
        let reversed = output[cursor + 2 + bytes..cursor + 2 + 2 * bytes].to_vec();
        if flags & 1 == 0 {
            assert!(forward.iter().all(|b| *b == 0));
        }
        if flags & 32 == 0 {
            assert!(reversed.iter().all(|b| *b == 0));
        }
        cases.push(Case {
            index: rank.to_decimal(),
            flags,
            forward: (flags & 1 != 0).then_some(forward),
            reversed: (flags & 32 != 0).then_some(reversed),
        });
        rank = rank.add(&U::from_u64(1));
        cursor += 2 + 2 * bytes;
    }
    assert_eq!(output.get(cursor), Some(&0));
    assert_eq!(output.last(), Some(&b'\n'));
    let footer =
        json::parse(std::str::from_utf8(&output[cursor + 1..output.len() - 1]).unwrap()).unwrap();
    (header, cases, footer)
}
fn single_bits(value: &Json, n: usize) -> Vec<u8> {
    let map = value.as_obj().unwrap();
    assert_eq!(map.len(), 1);
    let (bits, amp) = map.first_key_value().unwrap();
    assert_eq!(amp.as_arr().unwrap()[0].as_u64(), Some(1));
    let mut bytes = vec![0; n.div_ceil(8)];
    for (i, b) in bits.bytes().enumerate() {
        if b == b'1' {
            bytes[i / 8] |= 1 << (i % 8);
        }
    }
    bytes
}
#[test]
fn range_rows_and_single_reverse_match_including_fault_tuples() {
    for op in ["add", "udiv"] {
        let object = arithmetic(op);
        for engine in ["interpreter", "jit"] {
            let text = "{\"mut\":[0,0],\"in\":[1,0],\"mem\":{},\"borrowed\":[]}\n{\"mut\":[255,15],\"in\":[13,3],\"mem\":{},\"borrowed\":[]}\n{\"mut\":[0,0],\"in\":[1,0],\"mem\":{},\"borrowed\":[]}\n";
            let out = run(
                &vm(),
                &object,
                "full",
                &["--arg", "token:4,0,0,0", "--format", "jsonl"],
                text,
                engine,
            );
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let (header, cases, footer) = decode(&out.stdout);
            assert_eq!(header.get("rank_order").unwrap().to_jcs(), "[2,3,0,1]");
            assert_eq!(cases.len(), 3);
            assert_eq!(footer.get("cases").unwrap().as_str(), Some("3"));
            assert!(cases.iter().all(|c| c.flags == 127));
            for (case, (m, i)) in
                cases
                    .iter()
                    .zip([("0,0", "1,0"), ("255,15", "13,3"), ("0,0", "1,0")])
            {
                let single = Command::new(vm())
                    .arg("reverse")
                    .arg(&object)
                    .arg("full")
                    .args(["--mut", m, "--in", i, "--arg", "token:4,0,0,0"])
                    .env("BENNETT_VM_ENGINE", engine)
                    .output()
                    .unwrap();
                assert!(
                    single.status.success(),
                    "{}",
                    String::from_utf8_lossy(&single.stderr)
                );
                let value =
                    json::parse(std::str::from_utf8(&single.stdout).unwrap().trim()).unwrap();
                assert_eq!(
                    case.forward.as_ref().unwrap(),
                    &single_bits(value.get("forward_state").unwrap(), 28)
                );
                assert_eq!(
                    case.reversed.as_ref().unwrap(),
                    &single_bits(value.get("state").unwrap(), 28)
                );
            }
            assert_eq!(cases[0].forward, cases[2].forward);
        }
    }
}
#[test]
fn binary_jsonl_shards_cover_the_same_ordered_range() {
    let object =
        fixture("action @a (t:wire[2] mut) (c:wire[2] in) () () ()\n^entry():\ncx c[0] t[1]");
    let full = run(
        &vm(),
        &object,
        "a",
        &["--range", "0..16"],
        "",
        "interpreter",
    );
    assert!(full.status.success());
    let (_, all, _) = decode(&full.stdout);
    let mut shards = vec![];
    for bounds in ["0..5", "5..5", "5..11", "11..16"] {
        let out = run(
            &vm(),
            &object,
            "a",
            &["--range", bounds, "--format", "jsonl"],
            "",
            "interpreter",
        );
        assert!(out.status.success());
        shards.extend(decode(&out.stdout).1);
    }
    assert_eq!(shards, all);
    assert_eq!(all.len(), 16);
}
#[test]
fn failed_local_checks_complete_but_invalid_input_has_no_complete_footer() {
    let object = fixture("action @a (t:wire mut) () () () ()\n^entry():\nh t");
    let out = run(
        &vm(),
        &object,
        "a",
        &["--range", "0..2", "--format", "jsonl"],
        "",
        "interpreter",
    );
    assert!(!out.status.success());
    let (_, cases, footer) = decode(&out.stdout);
    assert_eq!(cases.len(), 2);
    assert!(cases.iter().all(|c| c.flags != 127 && c.forward.is_none()));
    assert_eq!(footer.get("failed").unwrap().as_str(), Some("2"));
    for invalid in [
        "\n",
        "{}\n",
        "{\"mut\":[2],\"in\":[],\"mem\":{},\"borrowed\":[]}\n",
        "{\"mut\":[0],\"in\":[],\"mem\":{},\"borrowed\":[],\"extra\":0}\n",
    ] {
        let out = run(&vm(), &object, "a", &[], invalid, "interpreter");
        assert!(!out.status.success());
        assert!(!String::from_utf8_lossy(&out.stdout).contains("\"complete\":true"));
        assert!(String::from_utf8_lossy(&out.stderr).contains("0"));
    }
}
#[test]
#[ignore = "pinned reference batch differential; mandatory in full gate"]
fn streams_are_byte_identical_to_pinned_reference() {
    let objects = [
        (
            fixture("action @a (t:wire[2] mut) () () () ()\n^entry():\nx t[0]\ncx t[0] t[1]"),
            "a",
            vec![],
            "0..4",
        ),
        (
            arithmetic("add"),
            "full",
            vec!["--arg", "token:4,0,0,0"],
            "4080..4160",
        ),
        (
            arithmetic("udiv"),
            "full",
            vec!["--arg", "token:4,0,0,0"],
            "4080..4160",
        ),
        (
            fixture("action @a (t:wire mut) () () () ()\n^entry():\ns t\ns t"),
            "a",
            vec![],
            "0..2",
        ),
        (
            fixture("action @a (t:wire mut) () () () ()\n^entry():\nh t"),
            "a",
            vec![],
            "0..2",
        ),
        (
            fixture(
                "action @a (t:wire[2] mut) (addr:wire in) (db:region[1,2] mut) () ()\n^entry():\nqexchm db addr t",
            ),
            "a",
            vec![],
            "0..128",
        ),
        (
            fixture(
                "action @a (t:wire mut) () () () ()\n^entry():\nb = qborrow any 1\ncx t b\ncx t b\nqreturn b",
            ),
            "a",
            vec![],
            "0..4",
        ),
        (
            fixture("action @a (t:wire[128] mut) () () () ()\n^entry():\nx t[127]"),
            "a",
            vec![],
            "340282366920938463463374607431768211453..340282366920938463463374607431768211456",
        ),
    ];
    let mut compared = 0;
    for (object, action, args, bounds) in objects {
        for encoding in ["binary", "jsonl"] {
            for adjoint in [false, true] {
                let mut options = args.clone();
                options.extend(["--range", bounds, "--format", encoding]);
                if adjoint {
                    options.push("--adjoint");
                }
                let expected = run(&reference(), &object, action, &options, "", "interpreter");
                assert!(
                    expected.stdout.contains(&b'\n'),
                    "reference rejected {} {action} {bounds}: {}",
                    object.display(),
                    String::from_utf8_lossy(&expected.stderr)
                );
                let rows = decode(&expected.stdout).1.len();
                for engine in ["interpreter", "jit"] {
                    let actual = run(&vm(), &object, action, &options, "", engine);
                    assert_eq!(
                        actual.status.code(),
                        expected.status.code(),
                        "{}",
                        String::from_utf8_lossy(&actual.stderr)
                    );
                    assert_eq!(
                        actual.stdout, expected.stdout,
                        "{action} {encoding} adjoint={adjoint} engine={engine}"
                    );
                    compared += rows;
                }
            }
        }
        if action == "full" {
            let input = "{\"mut\":[13,15],\"in\":[17,0],\"mem\":{},\"borrowed\":[]}\n{\"mut\":[3,7],\"in\":[21,3],\"mem\":{},\"borrowed\":[]}\n";
            for encoding in ["binary", "jsonl"] {
                let mut options = args.clone();
                options.extend(["--format", encoding]);
                let expected = run(
                    &reference(),
                    &object,
                    action,
                    &options,
                    input,
                    "interpreter",
                );
                for engine in ["interpreter", "jit"] {
                    let actual = run(&vm(), &object, action, &options, input, engine);
                    assert_eq!(actual.status.code(), expected.status.code());
                    assert_eq!(actual.stdout, expected.stdout);
                    compared += 2;
                }
            }
        }
    }
    println!("BATCH_REFERENCE_DIFFERENTIAL_ROWS={compared}");
}

#[test]
fn arbitrary_precision_endpoints_memory_and_borrowed_pool_are_complete() {
    let wide = fixture("action @a (t:wire[128] mut) () () () ()\n^entry():\nx t[127]");
    let out = run(
        &vm(),
        &wide,
        "a",
        &[
            "--range",
            "340282366920938463463374607431768211453..340282366920938463463374607431768211456",
        ],
        "",
        "interpreter",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (header, cases, footer) = decode(&out.stdout);
    assert_eq!(header.get("domain_bits").unwrap().as_usize(), Some(128));
    assert_eq!(cases.len(), 3);
    assert_eq!(cases[2].index, "340282366920938463463374607431768211455");
    assert_eq!(footer.get("cases").unwrap().as_str(), Some("3"));
    let memory = fixture(
        "action @a (t:wire[2] mut) (addr:wire in) (db:region[1,2] mut) () ()\n^entry():\nqexchm db addr t",
    );
    let out = run(
        &vm(),
        &memory,
        "a",
        &["--format", "jsonl"],
        "{\"mut\":[1],\"in\":[0],\"mem\":{\"db\":[2,3]},\"borrowed\":[]}\n",
        "interpreter",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (header, cases, _) = decode(&out.stdout);
    assert_eq!(header.get("rank_order").unwrap().to_jcs(), "[1,0,2,3]");
    assert_eq!(cases[0].flags, 127);
    assert_eq!(cases[0].forward, Some(vec![106]));
    assert_eq!(cases[0].reversed, Some(vec![113]));
    let borrowed = fixture(
        "action @a (t:wire mut) () () () ()\n^entry():\nb = qborrow any 1\ncx t b\ncx t b\nqreturn b",
    );
    let out = run(
        &vm(),
        &borrowed,
        "a",
        &["--format", "jsonl"],
        "{\"mut\":[1],\"in\":[],\"mem\":{},\"borrowed\":[1]}\n",
        "interpreter",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (header, cases, _) = decode(&out.stdout);
    assert_eq!(header.get("domain_bits").unwrap().as_usize(), Some(2));
    assert_eq!(cases[0].flags, 127);
    assert_eq!(cases[0].forward, Some(vec![3]));
    assert_eq!(cases[0].reversed, Some(vec![3]));
}
#[test]
fn interactive_header_and_flush_each_do_not_wait_for_eof() {
    use std::{
        io::{BufRead, BufReader},
        sync::mpsc,
        time::Duration,
    };
    let object = fixture("action @a (t:wire mut) () () () ()\n^entry():\nx t");
    let mut child = Command::new(vm())
        .arg("reverse-action-batch")
        .arg(&object)
        .arg("a")
        .args(["--format", "jsonl", "--flush-each"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (send, receive) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if send.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let next = |child: &mut std::process::Child| match receive.recv_timeout(Duration::from_secs(5))
    {
        Ok(line) => line,
        Err(error) => {
            let _ = child.kill();
            panic!("stream did not flush: {error}")
        }
    };
    assert_eq!(
        json::parse(&next(&mut child))
            .unwrap()
            .get("type")
            .unwrap()
            .as_str(),
        Some("header")
    );
    let mut input = child.stdin.take().unwrap();
    input
        .write_all(b"{\"mut\":[0],\"in\":[],\"mem\":{},\"borrowed\":[]}\n")
        .unwrap();
    input.flush().unwrap();
    let row = json::parse(&next(&mut child)).unwrap();
    assert_eq!(row.as_arr().unwrap()[0].as_str(), Some("case"));
    drop(input);
    assert_eq!(
        json::parse(&next(&mut child))
            .unwrap()
            .get("type")
            .unwrap()
            .as_str(),
        Some("footer")
    );
    assert!(child.wait().unwrap().success());
    reader.join().unwrap();
}

#[test]
fn malformed_ranges_and_noncanonical_uints_abort_without_footer() {
    let object = fixture("action @a (t:wire[64] mut) () () () ()\n^entry():\nx t[0]");
    for bounds in [
        "-1..2",
        "00..2",
        "2..1",
        "0..18446744073709551617",
        "0..1..2",
    ] {
        let out = run(&vm(), &object, "a", &["--range", bounds], "", "interpreter");
        assert!(!out.status.success());
        assert!(out.stdout.is_empty());
    }
    for value in ["-1", "0.0", "1e0", "\"01\"", "9007199254740992", "\"1\""] {
        let input = format!("{{\"mut\":[{value}],\"in\":[],\"mem\":{{}},\"borrowed\":[]}}\n");
        let out = run(&vm(), &object, "a", &[], &input, "interpreter");
        assert!(!out.status.success(), "{value}");
        assert!(!String::from_utf8_lossy(&out.stdout).contains("\"complete\":true"));
    }
}
#[test]
fn closed_output_pipe_is_an_error() {
    let object = fixture("action @a (t:wire[16] mut) () () () ()\n^entry():\nx t[0]");
    let mut child = Command::new(vm())
        .arg("reverse-action-batch")
        .arg(object)
        .arg("a")
        .args(["--range", "0..65536"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    assert!(!out.status.success());
    assert!(!out.stderr.is_empty());
}

#[test]
#[ignore = "pinned reference batch differential; mandatory in full gate"]
fn isa09_fixed_arguments_and_partitions_match_reference_streams() {
    let token = fixture_at(
        "0.9",
        "action @a (t:wire mut) (c:wire in) () (a:token, b:token) ()\n^entry():\ncx c t",
    );
    let rotation = fixture_at(
        "0.9",
        "action @a (t:wire mut) () () (angle:f64) ()\n^entry():\nrx angle t",
    );
    let mut compared = 0;
    for (object, args, bounds, rows) in [
        (
            &token,
            vec![
                "--arg",
                "token:4,0,0,0",
                "--arg",
                "token:4,0,0,0",
                "--token-aliases",
                "[0,0]",
            ],
            "0..4",
            4,
        ),
        (
            &token,
            vec![
                "--arg",
                "token:4,0,0,0",
                "--arg",
                "token:4,0,0,0",
                "--token-aliases",
                "[0,1]",
            ],
            "0..4",
            4,
        ),
        (&rotation, vec!["--arg", "f64:0.5"], "0..2", 2),
    ] {
        for encoding in ["binary", "jsonl"] {
            for adjoint in [false, true] {
                let mut options = args.clone();
                options.extend(["--range", bounds, "--format", encoding]);
                if adjoint {
                    options.push("--adjoint");
                }
                let expected = run(&reference(), object, "a", &options, "", "interpreter");
                assert!(
                    !expected.stdout.is_empty(),
                    "{}",
                    String::from_utf8_lossy(&expected.stderr)
                );
                for engine in ["interpreter", "jit"] {
                    let actual = run(&vm(), object, "a", &options, "", engine);
                    assert_eq!(
                        actual.status.code(),
                        expected.status.code(),
                        "{}",
                        String::from_utf8_lossy(&actual.stderr)
                    );
                    assert_eq!(actual.stdout, expected.stdout);
                    compared += rows;
                }
            }
        }
    }
    println!("BATCH_REFERENCE_09_DIFFERENTIAL_ROWS={compared}");
}

#[test]
fn selected_adjoint_direction_has_its_own_forward_tuple() {
    let object = fixture("action @a (t:wire[2] mut) () () () ()\n^entry():\nx t[0]\ncx t[0] t[1]");
    for (adjoint, expected) in [(false, vec![3, 0, 1, 2]), (true, vec![1, 2, 3, 0])] {
        let mut options = vec!["--range", "0..4"];
        if adjoint {
            options.push("--adjoint");
        }
        let out = run(&vm(), &object, "a", &options, "", "interpreter");
        assert!(out.status.success());
        let (_, cases, _) = decode(&out.stdout);
        assert_eq!(
            cases
                .iter()
                .map(|c| c.forward.as_ref().unwrap()[0])
                .collect::<Vec<_>>(),
            expected
        );
        for (index, case) in cases.iter().enumerate() {
            assert_eq!(case.reversed, Some(vec![index as u8]));
            assert_eq!(case.flags, 127);
        }
    }
}
