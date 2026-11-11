//! Streamed action reversal using the ordinary executor and exact shadow.
use super::*;
use crate::{
    cliargs,
    identity::Identities,
    json::{self, Json},
};
use std::io::{self, BufRead, Write};
mod layout;
use layout::Layout;
const FORMAT: &str = "bennett.reverse-action-batch/1";
fn text(value: impl Into<String>) -> Json {
    Json::Str(value.into())
}
fn size(value: usize) -> Json {
    Json::u64(value as u64)
}
fn decimal(raw: &str) -> Result<BigUint, String> {
    if raw.is_empty()
        || (raw.len() > 1 && raw.starts_with('0'))
        || !raw.bytes().all(|b| b.is_ascii_digit())
    {
        return Err("expected canonical unsigned decimal".into());
    }
    BigUint::parse_decimal(raw)
}
pub(super) fn uint(value: &Json) -> Result<BigUint, String> {
    let (raw, string) = match value {
        Json::Str(s) => (s, true),
        Json::Num(s) => (s, false),
        _ => return Err("expected canonical unsigned integer".into()),
    };
    let result = decimal(raw)?;
    if string != (result >= BigUint::from_u64(1).shl(53)) {
        return Err(
            "uint encoding must use numbers below 2^53 and decimal strings otherwise".into(),
        );
    }
    Ok(result)
}
struct Record {
    flags: u8,
    forward: Option<BigUint>,
    reversed: Option<BigUint>,
}
fn basis(state: &QuantumState) -> Option<BigUint> {
    (state.exact.len() == 1).then(|| state.exact.first_key_value().unwrap().0.clone())
}
fn positive_unit(state: &QuantumState) -> bool {
    if state.exact.len() != 1 {
        return false;
    }
    let value = state.exact.first_key_value().unwrap().1;
    value == &value.conj() && value.real_sign() > 0 && value.mul(value) == state.normalization
}
struct Batch<'o> {
    object: &'o Object,
    name: String,
    layout: Layout,
    action: prepared::PreparedAction<'o>,
    args: Vec<Value>,
    aliases: Option<String>,
    adjoint: bool,
    empty: QuantumState,
}
impl<'o> Batch<'o> {
    fn new(
        object: &'o Object,
        name: &str,
        raw: &[String],
        aliases: Option<&str>,
        adjoint: bool,
    ) -> Result<Self, String> {
        let sig = crate::validate::action_sig(object, name)?;
        let layout = Layout::new(object, name, &sig)?;
        let mut empty = QuantumState::new();
        empty.try_alloc_wires(layout.bits)?;
        if std::env::var("BENNETT_VM_BATCH_CACHE").as_deref() != Ok("0") {
            empty.oracle_values = Some(std::rc::Rc::new(std::cell::RefCell::new(
                oracle_cache::VerifiedValues::new(65_536),
            )));
        }
        let args = crate::qcli::bind_args(
            object,
            &sig.args,
            raw,
            &cliargs::default_budget(object, 1_000_000),
        )?;
        crate::token_aliases::bind_cli(object, &mut args.clone(), None, aliases)?;
        let action = prepared::PreparedAction::new(
            object,
            name,
            &layout.muts,
            &layout.inputs,
            &layout.memory,
            &[],
            &layout.pool,
        )?;
        Ok(Self {
            object,
            name: name.into(),
            layout,
            action,
            args,
            aliases: aliases.map(str::to_string),
            adjoint,
            empty,
        })
    }
    fn execute(&self, input: &BigUint) -> Result<Record, String> {
        let mut state = self.empty.clone();
        let wires: Vec<_> = (0..self.layout.bits).collect();
        state.prepare(&wires, input);
        let entry = state.clone();
        let mut args = self.args.clone();
        crate::token_aliases::bind_cli(self.object, &mut args, None, self.aliases.as_deref())?;
        self.action.execute(&mut state, &args, self.adjoint)?;
        state.rewind_clean_wires(self.layout.bits)?;
        let forward = basis(&state);
        let mut flags = u8::from(forward.is_some())
            | (u8::from(positive_unit(&state)) << 1)
            | (u8::from(state.wire_count() == self.layout.bits) << 2);
        let preserved = state.exact.keys().all(|key| {
            self.layout
                .preserved
                .iter()
                .all(|wire| key.bit(*wire) == input.bit(*wire))
        });
        flags |= u8::from(preserved) << 3;
        self.action.execute(&mut state, &args, !self.adjoint)?;
        state.rewind_clean_wires(self.layout.bits)?;
        flags |=
            u8::from(state.exact_equals(&entry) && state.wire_count() == entry.wire_count()) << 4;
        let reversed = basis(&state);
        flags |= u8::from(reversed.is_some()) << 5;
        flags |= u8::from(positive_unit(&state)) << 6;
        Ok(Record {
            flags,
            forward,
            reversed,
        })
    }
    fn header(
        &self,
        ids: &Identities,
        raw: &[String],
        encoding: &str,
        range: Option<&(BigUint, BigUint)>,
    ) -> Json {
        Json::obj(vec![
            ("type", text("header")),
            ("format", text(FORMAT)),
            ("encoding", text(encoding)),
            ("action", text(&self.name)),
            ("adjoint", Json::Bool(self.adjoint)),
            ("object_hash", text(&ids.object_hash)),
            ("interface_hash", text(&ids.interface[&self.name])),
            (
                "closure_hash",
                ids.closure[&self.name].as_ref().map_or(Json::Null, text),
            ),
            ("args", Json::Arr(raw.iter().map(text).collect())),
            (
                "token_aliases",
                self.aliases
                    .as_ref()
                    .map(|s| json::parse(s))
                    .transpose()
                    .expect("validated partition")
                    .unwrap_or(Json::Null),
            ),
            ("domain_bits", size(self.layout.bits)),
            ("record_bytes", size(self.layout.record_bytes)),
            (
                "mode",
                text(if range.is_some() { "range" } else { "jsonl" }),
            ),
            (
                "lo",
                range.map_or(Json::Null, |(lo, _)| text(lo.to_decimal())),
            ),
            (
                "hi",
                range.map_or(Json::Null, |(_, hi)| text(hi.to_decimal())),
            ),
            (
                "rank_order",
                Json::Arr(self.layout.order.iter().map(|n| size(*n)).collect()),
            ),
            ("components", self.layout.description()),
        ])
    }
}
fn payload(value: Option<&BigUint>, bytes: usize) -> Vec<u8> {
    let mut out = vec![0; bytes];
    if let Some(value) = value {
        for (index, byte) in out.iter_mut().enumerate() {
            *byte = value
                .limbs()
                .get(index / 8)
                .map_or(0, |limb| (limb >> (8 * (index % 8))) as u8);
        }
    }
    out
}
fn write_case(
    out: &mut impl Write,
    index: &BigUint,
    record: &Record,
    bytes: usize,
    binary: bool,
) -> Result<(), String> {
    if binary {
        out.write_all(&[1, record.flags])
            .map_err(|e| e.to_string())?;
        out.write_all(&payload(record.forward.as_ref(), bytes))
            .map_err(|e| e.to_string())?;
        out.write_all(&payload(record.reversed.as_ref(), bytes))
            .map_err(|e| e.to_string())?;
    } else {
        let hex = |v: Option<&BigUint>| {
            v.map_or(Json::Null, |v| {
                text(crate::sha256::to_hex(&payload(Some(v), bytes)))
            })
        };
        writeln!(
            out,
            "{}",
            Json::Arr(vec![
                text("case"),
                text(index.to_decimal()),
                Json::u64(record.flags as u64),
                hex(record.forward.as_ref()),
                hex(record.reversed.as_ref())
            ])
            .to_jcs()
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub fn command(arguments: &[String]) -> Result<(), String> {
    let options = cliargs::scan_opts(
        arguments,
        &["range", "format", "arg", "args", "token-aliases"],
        &["adjoint", "flush-each"],
    )?;
    if options.positional.len() != 2 {
        return Err("reverse-action-batch needs OBJECT ACTION".into());
    }
    let encoding = options.one("format")?.unwrap_or("binary");
    if !["binary", "jsonl"].contains(&encoding) {
        return Err("format must be binary or jsonl".into());
    }
    let (object, ids) = crate::load(&cliargs::read_object(&options.positional[0])?, false)?;
    let name = cliargs::entry_name(&options.positional[1]);
    let raw = if let Some(raw) = options.one("args")? {
        if !options.all("arg").is_empty() {
            return Err("--args and --arg are exclusive".into());
        }
        if raw.is_empty() {
            vec![]
        } else {
            raw.split(',').map(str::to_string).collect()
        }
    } else {
        options.all("arg")
    };
    let batch = Batch::new(
        &object,
        name,
        &raw,
        options.one("token-aliases")?,
        options.flags.contains("adjoint"),
    )?;
    let range = options
        .one("range")?
        .map(|s| -> Result<_, String> {
            let (lo, hi) = s.split_once("..").ok_or("range must be LO..HI")?;
            let (lo, hi) = (decimal(lo)?, decimal(hi)?);
            if lo > hi || hi > BigUint::from_u64(1).shl(batch.layout.bits) {
                return Err("range outside complete tuple domain".into());
            }
            Ok((lo, hi))
        })
        .transpose()?;
    let stdout = io::stdout();
    let mut out = io::BufWriter::with_capacity(1 << 20, stdout.lock());
    writeln!(
        out,
        "{}",
        batch.header(&ids, &raw, encoding, range.as_ref()).to_jcs()
    )
    .map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())?;
    let mut count = BigUint::zero();
    let mut failed = BigUint::zero();
    let mut first_failed = None;
    let mut case = |index: &BigUint, input: BigUint| -> Result<(), String> {
        let record = batch
            .execute(&input)
            .map_err(|e| format!("batch tuple {}: {e}", index.to_decimal()))?;
        write_case(
            &mut out,
            index,
            &record,
            batch.layout.bits.div_ceil(8),
            encoding == "binary",
        )
        .map_err(|e| format!("batch tuple {} output: {e}", index.to_decimal()))?;
        if options.flags.contains("flush-each") {
            out.flush()
                .map_err(|e| format!("batch tuple {} output: {e}", index.to_decimal()))?;
        }
        count = count.add(&BigUint::from_u64(1));
        if record.flags != 127 {
            failed = failed.add(&BigUint::from_u64(1));
            if first_failed.is_none() {
                first_failed = Some(index.to_decimal());
            }
        }
        Ok(())
    };
    if let Some((lo, hi)) = &range {
        let mut index = lo.clone();
        while &index < hi {
            case(&index, batch.layout.rank(&index))?;
            index = index.add(&BigUint::from_u64(1));
        }
    } else {
        let stdin = io::stdin();
        let mut reader = stdin.lock();
        let mut line = String::new();
        let mut index = BigUint::zero();
        loop {
            line.clear();
            let len = reader
                .read_line(&mut line)
                .map_err(|e| format!("batch tuple {} input: {e}", index.to_decimal()))?;
            if len == 0 {
                break;
            }
            let input = batch
                .layout
                .parse(&line)
                .map_err(|e| format!("batch tuple {} input: {e}", index.to_decimal()))?;
            case(&index, input)?;
            index = index.add(&BigUint::from_u64(1));
        }
    }
    let footer = Json::obj(vec![
        ("type", text("footer")),
        ("format", text(FORMAT)),
        ("complete", Json::Bool(true)),
        ("cases", text(count.to_decimal())),
        ("failed", text(failed.to_decimal())),
        ("first_failed", first_failed.map_or(Json::Null, text)),
    ]);
    if encoding == "binary" {
        out.write_all(&[0]).map_err(|e| e.to_string())?;
    }
    writeln!(out, "{}", footer.to_jcs()).map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())?;
    if failed.is_zero() {
        Ok(())
    } else {
        Err(format!(
            "{} batch cases failed local checks",
            failed.to_decimal()
        ))
    }
}
