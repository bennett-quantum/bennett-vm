use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = run(&args);
    bennett_vm::jit::audit();
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::from(1)
        }
    }
}

const USAGE: &str = "\
bennett-vm check <object>
bennett-vm hash <object>
bennett-vm run-oracle <object> <oracle> --input <x[,y...]> [--budget B[,heap,stack,depth]] [--mem name=hex] [--arg value]
bennett-vm step-check <object> <oracle> (--exhaustive [--max-bits N] | --suite | --samples N) [--seed a,b,c,d] [--budget B[,heap,stack,depth]] [--mem name=hex] [--arg value]
bennett-vm run-cc <object> <entry> [--args <a[,b...]> | --input <a[,b...]> | --arg value...] [--entropy hex] [--replies file] [--draws <d[,e...]>] [--budget B[,heap,stack,depth]] [--mem global=hex]
bennett-vm reverse <object> <entry> [run-cc options] [--steps N]
bennett-vm reverse <object> <oracle> --input <x[,y...]> [--budget B[,heap,stack,depth]] [--steps N] [--mem name=hex] [--arg value]
bennett-vm reverse <object> <action> (--mut <v[,...]> [--in <v[,...]>] [--mem name=hex] | --basis <bits> | --state <bits=re[,im];...>) [--args <a[,...]>] [--adjoint]
bennett-vm run-action <object> <action> (--mut <v[,...]> [--in <v[,...]>] [--mem name=hex] | --basis <bits> | --state <bits=re[,im];...>) [--args <a[,...]> | --arg value...] [--adjoint]
bennett-vm run-program <object> <cc-entry> [--args <a[,b...]> | --input <a[,b...]> | --arg value...] [--entropy hex] [--replies file] [--draws <d[,e...]>] [--budget B[,heap,stack,depth]] [--mem global=hex] [--reverse] [--target file]";

fn read_object(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))
}

fn run(args: &[String]) -> Result<(), String> {
    bennett_vm::jit::selection()?;
    let Some(cmd) = args.first() else {
        println!("{USAGE}");
        return Ok(());
    };
    match cmd.as_str() {
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            Ok(())
        }
        "check" | "hash" => {
            let path = args
                .get(1)
                .ok_or_else(|| format!("{cmd} needs an object path\n{USAGE}"))?;
            if args.len() > 2 {
                return Err(format!("unexpected argument {:?}\n{USAGE}", args[2]));
            }
            let text = read_object(path)?;
            let (obj, ids) = bennett_vm::load(&text, cmd == "hash")?;
            println!("{}", bennett_vm::identity::check_json(&obj, &ids).to_jcs());
            Ok(())
        }
        "run-oracle" => bennett_vm::oracle::cmd_run_oracle(&args[1..]),
        "run-cc" => bennett_vm::cc::cmd_run_cc(&args[1..], false),
        "reverse" => bennett_vm::cc::cmd_reverse(&args[1..]),
        "run-program" => bennett_vm::launch::cmd_run_program(&args[1..]),
        "step-check" => bennett_vm::stepcli::check(&args[1..]),
        "run-action" => bennett_vm::qcli::command(&args[1..], false),
        other => Err(format!("unknown subcommand {other}\n{USAGE}")),
    }
}
