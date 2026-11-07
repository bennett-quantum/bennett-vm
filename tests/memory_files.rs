//! Binary region input avoids the operating system's per-argument hex limit.
use crate::source09 as support;
use bennett_vm::{json::{self, Json}, sha256::to_hex};
use std::{path::{Path, PathBuf}, process::{Command, Output}, sync::atomic::{AtomicUsize, Ordering}};

struct Fixture { dir: PathBuf, object: PathBuf, image: PathBuf }
impl Fixture {
    fn new(version: &str, address_width: usize) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("memory-file-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        let object = dir.join("read.bisa");
        let image = dir.join("region = data.bin");
        let functions = format!("oracle @read (x:wire[{address_width}] in) (t:wire[8]) (f:wire[4]) (m:region[{address_width},8] in) ()\nregs {{ base:reg[32], offset:reg[32], p:reg[32], v:reg[8] }}\n^entry():\nbase = lift.addr @m\noffset = lift.zext.i32 x\np = lift.add base offset\nv = lift.load.i8 p\nlift.retval v");
        let mut source = support::source_at(version, &functions);
        if address_width > 2 {
            let (prefix, layout) = source.split_once("section layout\n").unwrap();
            let Json::Obj(mut layout) = json::parse(layout.trim()).unwrap() else { panic!("layout"); };
            layout.insert("H".into(), Json::u64(18));
            layout.insert("heap_base".into(), Json::u64(131072));
            layout.insert("heap_limit_max".into(), Json::u64(131072));
            source = support::sign(&format!("{prefix}section layout\n{}\n", Json::Obj(layout).to_jcs()));
        }
        std::fs::write(&object, source).unwrap();
        Self { dir, object, image }
    }
    fn run(&self, engine: &str, command: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_bennett-vm")).args([command, self.object.to_str().unwrap(), "read"])
            .args(args).env("BENNETT_VM_ENGINE", engine).output().unwrap()
    }
}
impl Drop for Fixture { fn drop(&mut self) { std::fs::remove_dir_all(&self.dir).unwrap(); } }
fn success(output: Output) -> Json {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    json::parse(std::str::from_utf8(&output.stdout).unwrap()).unwrap()
}
fn binding(path: &Path) -> String { format!("m={}", path.display()) }

#[test]
fn binary_regions_match_hex_for_oracle_steps_and_reversal_on_both_engines_and_versions() {
    let bytes = [0x17, 0x53, 0x9a, 0xed];
    for version in ["0.8", "0.9"] {
        let fixture = Fixture::new(version, 2);
        std::fs::write(&fixture.image, bytes).unwrap();
        let file = binding(&fixture.image);
        let hex = format!("m={}", to_hex(&bytes));
        for engine in ["interpreter", "jit"] {
            for (index, expected) in bytes.into_iter().enumerate() {
                let index = index.to_string();
                let result = success(fixture.run(engine, "run-oracle", &["--input", &index, "--budget", "8", "--mem-file", &file]));
                assert_eq!(result.get("code").and_then(Json::as_u64), Some(0));
                assert_eq!(result.get("payload").and_then(Json::as_u64), Some(expected as u64));
                assert_eq!(result, success(fixture.run(engine, "run-oracle", &["--input", &index, "--budget", "8", "--mem", &hex])));
            }
            for (command, args) in [("step-check", vec!["--exhaustive", "--max-bits", "2"]), ("reverse", vec!["--input", "3"])] {
                let mut file_args = args.clone(); file_args.extend(["--budget", "8", "--mem-file", &file]);
                let mut hex_args = args; hex_args.extend(["--budget", "8", "--mem", &hex]);
                let result = success(fixture.run(engine, command, &file_args));
                assert_eq!(result, success(fixture.run(engine, command, &hex_args)));
                if command == "step-check" {
                    assert_eq!(result.get("cases").and_then(Json::as_u64), Some(4));
                    assert_eq!(result.get("fixed_regions"), Some(&Json::Arr(vec![Json::Str("m".into())])));
                } else { assert_eq!(result.get("equals_entry"), Some(&Json::Bool(true))); }
            }
        }
    }
}

#[test]
fn binary_region_validation_rejects_bad_bindings_without_execution() {
    let fixture = Fixture::new("0.9", 2);
    std::fs::write(&fixture.image, [1, 2, 3, 4]).unwrap();
    let good = binding(&fixture.image);
    let missing = binding(&fixture.dir.join("missing"));
    let unknown = format!("other={}", fixture.image.display());
    for command in ["run-oracle", "step-check", "reverse"] {
        let base = if command == "step-check" { vec!["--exhaustive", "--max-bits", "2"] } else { vec!["--input", "0"] };
        for suffix in [vec!["--mem-file", "malformed"], vec!["--mem-file", &missing], vec!["--mem-file", &unknown],
            vec!["--mem-file", &good, "--mem-file", &good], vec!["--mem", "m=01020304", "--mem-file", &good]] {
            let mut args = base.clone(); args.extend(suffix);
            let out = fixture.run("interpreter", command, &args);
            assert!(!out.status.success(), "{command}/{args:?}"); assert!(out.stdout.is_empty()); assert!(!out.stderr.is_empty());
        }
        for bytes in [vec![], vec![0u8;3], vec![0u8;5]] {
            std::fs::write(&fixture.image, bytes).unwrap();
            let mut args = base.clone(); args.extend(["--mem-file", &good]);
            let out = fixture.run("interpreter", command, &args);
            assert!(!out.status.success()); assert!(out.stdout.is_empty());
        }
        std::fs::write(&fixture.image, [1, 2, 3, 4]).unwrap();
    }
}

#[test]
fn binary_region_larger_than_one_hex_argument_loads_the_actual_last_byte() {
    let fixture = Fixture::new("0.9", 16);
    let image = (0..65536).map(|i| ((i * 37 + 11) & 255) as u8).collect::<Vec<_>>();
    std::fs::write(&fixture.image, &image).unwrap();
    let file = binding(&fixture.image);
    assert!(2 * image.len() + 2 >= 131072);
    for engine in ["interpreter", "jit"] {
        let result = success(fixture.run(engine, "run-oracle", &["--input", "65535", "--budget", "8", "--mem-file", &file]));
        assert_eq!(result.get("code").and_then(Json::as_u64), Some(0));
        assert_eq!(result.get("payload").and_then(Json::as_u64), Some(image[65535] as u64));
    }
}
