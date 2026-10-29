//! §16 canonical textual form: parser. Validation lives in `validate`,
//! identities in `identity`; this module only gives structure plus the
//! raw text each hash and trace line refers to.

use crate::bigint::BigUint;
use crate::json::{self, Json};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    I(usize),
    Ptr,
    F64,
    Token,
    Wire(usize),
    Reg(usize),
    Region(usize, usize),
    Qreg(usize),
}

impl Type {
    pub fn parse(text: &str) -> Result<Type, String> {
        let t = text;
        if t == "ptr" {
            return Ok(Type::Ptr);
        }
        if t == "f64" {
            return Ok(Type::F64);
        }
        if t == "token" {
            return Ok(Type::Token);
        }
        if t == "wire" {
            return Ok(Type::Wire(1));
        }
        if let Some(rest) = t.strip_prefix('i')
            && !rest.is_empty()
            && rest.bytes().all(|c| c.is_ascii_digit())
        {
            return Ok(Type::I(parse_usize(rest)?));
        }
        for (prefix, make) in [
            ("wire[", Type::Wire as fn(usize) -> Type),
            ("reg[", Type::Reg),
            ("qreg[", Type::Qreg),
        ] {
            if let Some(rest) = t.strip_prefix(prefix) {
                let num = rest
                    .strip_suffix(']')
                    .ok_or_else(|| format!("malformed type {t:?}"))?;
                return Ok(make(parse_usize(num)?));
            }
        }
        if let Some(rest) = t.strip_prefix("region[") {
            let inner = rest
                .strip_suffix(']')
                .ok_or_else(|| format!("malformed type {t:?}"))?;
            let (a, w) = inner
                .split_once(',')
                .ok_or_else(|| format!("malformed type {t:?}"))?;
            return Ok(Type::Region(parse_usize(a)?, parse_usize(w)?));
        }
        Err(format!("unknown type {t:?}"))
    }
    /// Text form used inside §9.1 signature type strings.
    pub fn signature_text(&self) -> String {
        match self {
            Type::I(n) => format!("i{n}"),
            Type::Ptr => "ptr".into(),
            Type::F64 => "f64".into(),
            Type::Token => "token".into(),
            Type::Wire(n) => {
                if *n == 1 {
                    "wire".into()
                } else {
                    format!("wire[{n}]")
                }
            }
            Type::Reg(n) => format!("reg[{n}]"),
            Type::Region(a, w) => format!("region[{a},{w}]"),
            Type::Qreg(n) => format!("qreg[{n}]"),
        }
    }
    pub fn width(&self) -> Option<usize> {
        match self {
            Type::I(n) | Type::Wire(n) | Type::Reg(n) => Some(*n),
            _ => None,
        }
    }
}

pub fn parse_usize(text: &str) -> Result<usize, String> {
    text.parse::<usize>()
        .map_err(|_| format!("invalid integer {text:?}"))
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub role: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Instr {
    pub lhs: Vec<String>,
    pub op: String,
    pub operands: Vec<String>,
    /// Nested instruction lists for `qif`/`qloop` (0, 1 or 2 arms).
    pub nested: Vec<Vec<Instr>>,
    /// 1-based object text line.
    pub line: usize,
    pub raw: String,
}

#[derive(Clone, Debug)]
pub struct Block {
    pub name: String,
    pub params: Vec<Param>,
    pub line: usize,
    pub instrs: Vec<Instr>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FnKind {
    Cc,
    R,
    Oracle,
    Action,
    PermAction,
    Lproc,
}

impl FnKind {
    pub fn kind_str(&self) -> &'static str {
        match self {
            FnKind::Cc => "cc",
            FnKind::R => "r",
            FnKind::Oracle => "oracle",
            FnKind::Action => "action",
            FnKind::PermAction => "perm_action",
            FnKind::Lproc => "lproc",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Function {
    pub kind: FnKind,
    pub name: String,
    /// The five header lists of §16.2, in order.
    pub lists: [Vec<Param>; 5],
    pub regs: Vec<Param>,
    pub bounds: Vec<usize>,
    pub has_regs_line: bool,
    pub blocks: Vec<Block>,
    pub header_line: usize,
    /// Canonical function bytes: header through last line, LF-joined.
    pub canonical: String,
}

impl Function {
    pub fn block(&self, name: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.name == name)
    }
    /// All instructions flattened in text order (nested lines included).
    pub fn flat_instrs<'a>(&'a self, out: &mut Vec<&'a Instr>) {
        fn walk<'a>(instrs: &'a [Instr], out: &mut Vec<&'a Instr>) {
            for ins in instrs {
                out.push(ins);
                for arm in &ins.nested {
                    walk(arm, out);
                }
            }
        }
        for b in &self.blocks {
            walk(&b.instrs, out);
        }
    }
    pub fn instr_count(&self) -> usize {
        let mut v = Vec::new();
        self.flat_instrs(&mut v);
        v.len()
    }
}

#[derive(Clone, Debug)]
pub struct Symbol {
    pub index: usize,
    pub name: String,
    pub kind: String,
    pub signature: Json,
    pub stored_hash: String,
}

#[derive(Clone, Debug)]
pub struct Import {
    pub signature: Json,
    pub pinned_closure_hash: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Global {
    pub base: BigUint,
    pub size: BigUint,
    pub image: String,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub ptr_width: usize,
    pub h: usize,
    pub a_tab: usize,
    pub s_cells: usize,
    pub null_size: BigUint,
    pub heap_base: BigUint,
    pub heap_limit_max: BigUint,
    pub globals: BTreeMap<String, Global>,
    pub interface_regions: BTreeMap<String, BTreeMap<String, BigUint>>,
    pub block_image: Option<String>,
    pub json: Json,
}

#[derive(Clone, Debug)]
pub struct Object {
    pub text: String,
    pub ptr_width: usize,
    /// Symbols by name.
    pub symbols: BTreeMap<String, Symbol>,
    pub imports: BTreeMap<String, Import>,
    pub functions: Vec<Function>,
    /// Data symbol name -> hex text.
    pub data: BTreeMap<String, String>,
    pub devices: Option<Json>,
    pub layout: Option<Layout>,
    pub has_schedule: bool,
    pub has_certificate: bool,
    pub has_coverage: bool,
}

impl Object {
    pub fn function(&self, name: &str) -> Option<&Function> {
        self.functions.iter().find(|f| f.name == name)
    }
    pub fn symbol_kind(&self, name: &str) -> Option<&str> {
        self.symbols.get(name).map(|s| s.kind.as_str())
    }
}

fn err(rule: &str, msg: impl std::fmt::Display) -> String {
    format!("§{rule}: {msg}")
}

/// Split one canonical line into top-level tokens. Parentheses, brackets,
/// braces and string literals keep their inner spaces.
pub fn split_tokens(line: &str) -> Result<Vec<String>, String> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut depth_paren = 0i32;
    let mut depth_brack = 0i32;
    let mut in_str = false;
    let mut start: Option<usize> = None;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' => {
                in_str = true;
                if start.is_none() {
                    start = Some(i);
                }
            }
            b'(' => {
                depth_paren += 1;
                if start.is_none() {
                    start = Some(i);
                }
            }
            b')' => depth_paren -= 1,
            b'[' => {
                depth_brack += 1;
                if start.is_none() {
                    start = Some(i);
                }
            }
            b']' => depth_brack -= 1,
            b' ' if depth_paren == 0 && depth_brack == 0 => {
                if let Some(st) = start.take() {
                    out.push(line[st..i].to_string());
                }
                i += 1;
                continue;
            }
            _ => {
                if start.is_none() {
                    start = Some(i);
                }
            }
        }
        if depth_paren < 0 || depth_brack < 0 {
            return Err(err(
                "16.2",
                format!("unbalanced delimiters in line {line:?}"),
            ));
        }
        i += 1;
    }
    if in_str || depth_paren != 0 || depth_brack != 0 {
        return Err(err("16.2", format!("unterminated token in line {line:?}")));
    }
    if let Some(st) = start {
        out.push(line[st..].to_string());
    }
    Ok(out)
}

/// Split a parenthesised list interior on top-level commas.
pub fn split_list(inner: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let bytes = inner.as_bytes();
    let mut start = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            if c == b'\\' {
                i += 2;
                continue;
            }
            if c == b'"' {
                in_str = false;
            }
        } else {
            match c {
                b'"' => in_str = true,
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                b',' if depth == 0 => {
                    out.push(inner[start..i].trim().to_string());
                    start = i + 1;
                }
                _ => {}
            }
        }
        i += 1;
    }
    let tail = inner[start..].trim();
    if !tail.is_empty() || !out.is_empty() {
        out.push(tail.to_string());
    }
    out.into_iter().filter(|s| !s.is_empty()).collect()
}

/// Contents of a `( ... )` operand, or `None` when it is not one list.
pub fn paren_inner(token: &str) -> Option<&str> {
    let t = token.strip_prefix('(')?.strip_suffix(')')?;
    // Ensure the outer pair spans the whole token.
    let mut depth = 0i32;
    for (idx, c) in token.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 && idx != token.len() - 1 {
                    return None;
                }
            }
            _ => {}
        }
    }
    Some(t)
}

const SECTIONS: [&str; 9] = [
    "symbols",
    "imports",
    "functions",
    "data",
    "devices",
    "layout",
    "schedule",
    "certificate",
    "coverage",
];

pub fn valid_name(text: &str) -> bool {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

pub fn parse(text: &str) -> Result<Object, String> {
    if text.contains('\r') {
        return Err(err("16.1", "CR line endings are not canonical (LF only)"));
    }
    if !text.ends_with('\n') {
        return Err(err("16.1", "object text must end with LF"));
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Err(err("16.2", "malformed object header"));
    }
    for (i, line) in lines.iter().enumerate() {
        if line.ends_with(' ') || line.ends_with('\t') {
            return Err(err(
                "16.1",
                format!("trailing whitespace on line {}", i + 1),
            ));
        }
        if line.contains('\t') {
            return Err(err("16.1", format!("tab character on line {}", i + 1)));
        }
    }
    // Blank lines only immediately before a section line, exactly one.
    for (i, line) in lines.iter().enumerate() {
        if line.is_empty() {
            let next = lines.get(i + 1);
            match next {
                Some(n) if n.starts_with("section ") => {}
                _ => {
                    return Err(err(
                        "16.1",
                        format!("blank line {} does not precede a section", i + 1),
                    ));
                }
            }
            if i > 0 && lines[i - 1].is_empty() {
                return Err(err(
                    "16.1",
                    format!("more than one blank line before line {}", i + 1),
                ));
            }
        }
        if line.starts_with("section ") && (i == 0 || !lines[i - 1].is_empty()) {
            return Err(err(
                "16.1",
                format!(
                    "section on line {} needs exactly one preceding blank line",
                    i + 1
                ),
            ));
        }
    }
    let ptr_width = parse_header(lines[0])?;
    let mut obj = Object {
        text: text.to_string(),
        ptr_width,
        symbols: BTreeMap::new(),
        imports: BTreeMap::new(),
        functions: Vec::new(),
        data: BTreeMap::new(),
        devices: None,
        layout: None,
        has_schedule: false,
        has_certificate: false,
        has_coverage: false,
    };
    // Sections: gather (name, body line indices).
    let mut order: Vec<(&str, usize, usize)> = Vec::new(); // name, header idx, body start
    for (i, line) in lines.iter().enumerate() {
        if let Some(name) = line.strip_prefix("section ") {
            order.push((name, i, i + 1));
        }
    }
    let mut last_rank: Option<usize> = None;
    for (idx, (name, hdr, start)) in order.iter().enumerate() {
        let rank = SECTIONS
            .iter()
            .position(|s| s == name)
            .ok_or_else(|| err("9.1", format!("unknown section {name:?}")))?;
        if let Some(prev) = last_rank
            && rank <= prev
        {
            return Err(err(
                "9.1",
                format!("section {name:?} is duplicated or out of order"),
            ));
        }
        last_rank = Some(rank);
        let end = match order.get(idx + 1) {
            Some((_, next_hdr, _)) => next_hdr - 1, // drop the blank line
            None => lines.len(),
        };
        let body = &lines[*start..end];
        let body_start_line = start + 1; // 1-based line of body[0]
        match *name {
            "symbols" => parse_symbols(&mut obj, body)?,
            "imports" => parse_imports(&mut obj, body)?,
            "functions" => {
                let mut cur = Cursor {
                    lines: body,
                    pos: 0,
                    base_line: body_start_line,
                };
                obj.functions = parse_functions(&mut cur)?;
            }
            "data" => parse_data(&mut obj, body)?,
            "devices" => obj.devices = Some(parse_json_section(name, body)?),
            "layout" => obj.layout = Some(parse_layout(body)?),
            "schedule" => {
                parse_json_section(name, body)?;
                obj.has_schedule = true;
            }
            "certificate" => {
                parse_json_section(name, body)?;
                obj.has_certificate = true;
            }
            "coverage" => {
                parse_json_section(name, body)?;
                obj.has_coverage = true;
            }
            _ => unreachable!(),
        }
        let _ = hdr;
    }
    Ok(obj)
}

fn parse_header(line: &str) -> Result<usize, String> {
    let toks = split_tokens(line).map_err(|_| err("16.2", "malformed object header"))?;
    let keys = [
        "isa", "object", "launch", "sched", "cert", "coverage", "target",
    ];
    if toks.len() != 9 || toks[0] != "bennett-object" {
        return Err(err("16.2", "malformed object header"));
    }
    for (i, k) in keys.iter().enumerate() {
        let want = format!("{k}=0.8");
        if toks[1 + i] != want {
            if toks[1 + i].starts_with(&format!("{k}=")) {
                return Err(err(
                    "1",
                    format!(
                        "unsupported {k} version in header {:?}; this implementation is 0.8",
                        toks[1 + i]
                    ),
                ));
            }
            return Err(err("16.2", "malformed object header"));
        }
    }
    let pw = toks[8]
        .strip_prefix("ptr_width=")
        .ok_or_else(|| err("16.2", "malformed object header"))?;
    parse_usize(pw).map_err(|_| err("16.2", "malformed object header"))
}

fn parse_json_section(name: &str, body: &[&str]) -> Result<Json, String> {
    if body.len() != 1 {
        return Err(err(
            "9.1",
            format!("section {name} must be exactly one JCS line"),
        ));
    }
    let v = json::parse(body[0]).map_err(|e| err("9.1", format!("section {name}: {e}")))?;
    if !matches!(v, Json::Obj(_)) {
        return Err(err("9.1", format!("section {name} must be a JSON object")));
    }
    if v.to_jcs() != body[0] {
        return Err(err(
            "16.1",
            format!("section {name} line is not RFC 8785 canonical JSON"),
        ));
    }
    Ok(v)
}

fn parse_symbols(obj: &mut Object, body: &[&str]) -> Result<(), String> {
    let v = parse_json_section("symbols", body)?;
    let map = v.as_obj().unwrap();
    for (index_text, entry) in map {
        let index = parse_usize(index_text).map_err(|_| {
            err(
                "9.1",
                format!("symbol index {index_text:?} is not an integer"),
            )
        })?;
        let e = entry
            .as_obj()
            .ok_or_else(|| err("9.1", "symbol entry must be an object"))?;
        for k in e.keys() {
            if !["name", "kind", "signature", "interface_hash"].contains(&k.as_str()) {
                return Err(err("9.1", format!("unknown field {k:?} in symbol entry")));
            }
        }
        let name = entry
            .get("name")
            .and_then(|x| x.as_str())
            .ok_or_else(|| err("9.1", "symbol entry needs a string name"))?
            .to_string();
        let kind = entry
            .get("kind")
            .and_then(|x| x.as_str())
            .ok_or_else(|| err("9.1", "symbol entry needs a string kind"))?
            .to_string();
        let signature = entry
            .get("signature")
            .cloned()
            .ok_or_else(|| err("9.1", "symbol entry needs a signature"))?;
        let stored_hash = entry
            .get("interface_hash")
            .and_then(|x| x.as_str())
            .ok_or_else(|| err("9.1", "symbol entry needs a string interface_hash"))?
            .to_string();
        if !valid_name(&name) {
            return Err(err("16.1", format!("invalid symbol name {name:?}")));
        }
        if obj
            .symbols
            .insert(
                name.clone(),
                Symbol {
                    index,
                    name: name.clone(),
                    kind,
                    signature,
                    stored_hash,
                },
            )
            .is_some()
        {
            return Err(err("9.1", format!("duplicate symbol name {name:?}")));
        }
    }
    Ok(())
}

fn parse_imports(obj: &mut Object, body: &[&str]) -> Result<(), String> {
    let v = parse_json_section("imports", body)?;
    for (name, entry) in v.as_obj().unwrap() {
        let e = entry
            .as_obj()
            .ok_or_else(|| err("9.1", "import entry must be an object"))?;
        for k in e.keys() {
            if !["signature", "pinned_closure_hash"].contains(&k.as_str()) {
                return Err(err("9.1", format!("unknown field {k:?} in import entry")));
            }
        }
        let signature = entry
            .get("signature")
            .cloned()
            .ok_or_else(|| err("9.1", "import entry needs a signature"))?;
        let pinned = match entry.get("pinned_closure_hash") {
            Some(p) => Some(
                p.as_str()
                    .ok_or_else(|| err("9.1", "pinned_closure_hash must be a string"))?
                    .to_string(),
            ),
            None => None,
        };
        if obj
            .imports
            .insert(
                name.clone(),
                Import {
                    signature,
                    pinned_closure_hash: pinned,
                },
            )
            .is_some()
        {
            return Err(err("9.1", format!("duplicate import {name:?}")));
        }
    }
    Ok(())
}

fn parse_data(obj: &mut Object, body: &[&str]) -> Result<(), String> {
    for line in body {
        let (name, hex) = line
            .split_once(' ')
            .ok_or_else(|| err("16.2", format!("malformed data line {line:?}")))?;
        if !valid_name(name) {
            return Err(err("16.1", format!("invalid data name {name:?}")));
        }
        if hex.len() % 2 != 0
            || !hex
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return Err(err(
                "16.1",
                format!("data line for {name:?} is not lowercase even hex"),
            ));
        }
        if obj.data.insert(name.to_string(), hex.to_string()).is_some() {
            return Err(err("9.1", format!("duplicate data entry {name:?}")));
        }
    }
    Ok(())
}

fn layout_big(v: &Json, key: &str) -> Result<BigUint, String> {
    v.get(key).and_then(|x| x.as_big()).ok_or_else(|| {
        err(
            "9.1",
            format!("layout field {key:?} must be a nonnegative integer"),
        )
    })
}
fn layout_usize(v: &Json, key: &str) -> Result<usize, String> {
    v.get(key).and_then(|x| x.as_usize()).ok_or_else(|| {
        err(
            "9.1",
            format!("layout field {key:?} must be a nonnegative integer that fits the host"),
        )
    })
}

fn parse_layout(body: &[&str]) -> Result<Layout, String> {
    let v = parse_json_section("layout", body)?;
    let map = v.as_obj().unwrap();
    for k in map.keys() {
        if ![
            "ptr_width",
            "H",
            "A_tab",
            "S_cells",
            "null_size",
            "heap_base",
            "heap_limit_max",
            "globals",
            "interface_regions",
            "block_image",
        ]
        .contains(&k.as_str())
        {
            return Err(err("9.1", format!("unknown field {k:?} in layout")));
        }
    }
    let mut globals = BTreeMap::new();
    let gobj = v
        .get("globals")
        .and_then(|x| x.as_obj())
        .ok_or_else(|| err("9.1", "layout.globals must be an object"))?;
    for (name, g) in gobj {
        let gm = g
            .as_obj()
            .ok_or_else(|| err("9.1", "layout global must be an object"))?;
        for k in gm.keys() {
            if !["base", "size", "image"].contains(&k.as_str()) {
                return Err(err("9.1", format!("unknown field {k:?} in layout global")));
            }
        }
        globals.insert(
            name.clone(),
            Global {
                base: layout_big(g, "base")?,
                size: layout_big(g, "size")?,
                image: g
                    .get("image")
                    .and_then(|x| x.as_str())
                    .ok_or_else(|| err("9.1", "layout global needs a string image"))?
                    .to_string(),
            },
        );
    }
    let mut interface_regions = BTreeMap::new();
    let iobj = v
        .get("interface_regions")
        .and_then(|x| x.as_obj())
        .ok_or_else(|| err("9.1", "layout.interface_regions must be an object"))?;
    for (oracle, regions) in iobj {
        let rm = regions
            .as_obj()
            .ok_or_else(|| err("9.1", "interface_regions entry must be an object"))?;
        let mut inner = BTreeMap::new();
        for (region, rv) in rm {
            let rvm = rv
                .as_obj()
                .ok_or_else(|| err("9.1", "interface region entry must be an object"))?;
            for k in rvm.keys() {
                if k != "base" {
                    return Err(err(
                        "9.1",
                        format!("unknown field {k:?} in interface region entry"),
                    ));
                }
            }
            inner.insert(region.clone(), layout_big(rv, "base")?);
        }
        interface_regions.insert(oracle.clone(), inner);
    }
    Ok(Layout {
        ptr_width: layout_usize(&v, "ptr_width")?,
        h: layout_usize(&v, "H")?,
        a_tab: layout_usize(&v, "A_tab")?,
        s_cells: layout_usize(&v, "S_cells")?,
        null_size: layout_big(&v, "null_size")?,
        heap_base: layout_big(&v, "heap_base")?,
        heap_limit_max: layout_big(&v, "heap_limit_max")?,
        globals,
        interface_regions,
        block_image: v
            .get("block_image")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
        json: v,
    })
}

struct Cursor<'a> {
    lines: &'a [&'a str],
    pos: usize,
    /// 1-based object line number of `lines[0]`.
    base_line: usize,
}

impl<'a> Cursor<'a> {
    fn line_no(&self) -> usize {
        self.base_line + self.pos
    }
    fn peek(&self) -> Option<&'a str> {
        self.lines.get(self.pos).copied()
    }
    fn next(&mut self) -> Option<(usize, &'a str)> {
        let line = self.peek()?;
        let no = self.line_no();
        self.pos += 1;
        Some((no, line))
    }
}

fn is_function_header(line: &str) -> bool {
    line.starts_with("fn cc @")
        || line.starts_with("fn r @")
        || line.starts_with("oracle @")
        || line.starts_with("action @")
        || line.starts_with("perm_action @")
        || line.starts_with("lproc @")
}

fn parse_params(token: &str) -> Result<Vec<Param>, String> {
    let inner = paren_inner(token).ok_or_else(|| {
        err(
            "16.2",
            format!("expected a parenthesised list, got {token:?}"),
        )
    })?;
    let mut out = Vec::new();
    for item in split_list(inner) {
        // NAME ":" type (SP role)?
        let mut parts = item.splitn(2, ' ');
        let head = parts.next().unwrap_or("");
        let role = parts.next().map(|s| s.to_string());
        if let Some(r) = &role
            && r != "in"
            && r != "mut"
        {
            return Err(err(
                "16.2",
                format!("invalid role {r:?} in parameter {item:?}"),
            ));
        }
        let (name, ty) = head
            .split_once(':')
            .ok_or_else(|| err("16.2", format!("malformed parameter {item:?}")))?;
        if !valid_name(name) {
            return Err(err("16.1", format!("invalid parameter name {name:?}")));
        }
        out.push(Param {
            name: name.to_string(),
            ty: Type::parse(ty).map_err(|e| err("16.2", e))?,
            role,
        });
    }
    Ok(out)
}

fn parse_functions(cur: &mut Cursor) -> Result<Vec<Function>, String> {
    let mut functions = Vec::new();
    while cur.peek().is_some() {
        functions.push(parse_function(cur)?);
    }
    Ok(functions)
}

fn parse_function(cur: &mut Cursor) -> Result<Function, String> {
    let start_pos = cur.pos;
    let (header_line, header) = cur
        .next()
        .ok_or_else(|| err("16.2", "expected a function"))?;
    if !is_function_header(header) {
        return Err(err(
            "16.2",
            format!("expected a function header, got {header:?}"),
        ));
    }
    let toks = split_tokens(header)?;
    let (kind, name_tok, list_toks): (FnKind, &str, &[String]) = if toks[0] == "fn" {
        let kind = match toks.get(1).map(|s| s.as_str()) {
            Some("cc") => FnKind::Cc,
            Some("r") => FnKind::R,
            _ => return Err(err("16.2", format!("malformed function header {header:?}"))),
        };
        (
            kind,
            toks.get(2)
                .ok_or_else(|| err("16.2", "malformed function header"))?,
            &toks[3.min(toks.len())..],
        )
    } else {
        let kind = match toks[0].as_str() {
            "oracle" => FnKind::Oracle,
            "action" => FnKind::Action,
            "perm_action" => FnKind::PermAction,
            "lproc" => FnKind::Lproc,
            _ => unreachable!(),
        };
        (
            kind,
            toks.get(1)
                .ok_or_else(|| err("16.2", "malformed function header"))?,
            &toks[2.min(toks.len())..],
        )
    };
    let name = name_tok
        .strip_prefix('@')
        .ok_or_else(|| {
            err(
                "16.2",
                format!("function name {name_tok:?} must start with @"),
            )
        })?
        .to_string();
    if !valid_name(&name) {
        return Err(err("16.1", format!("invalid function name {name:?}")));
    }
    if list_toks.len() != 5 {
        return Err(err(
            "16.2",
            format!(
                "function header for @{name} must have five lists, got {}",
                list_toks.len()
            ),
        ));
    }
    let mut lists: [Vec<Param>; 5] = Default::default();
    for i in 0..5 {
        lists[i] = parse_params(&list_toks[i])?;
    }
    // Optional regs line.
    let mut regs = Vec::new();
    let mut bounds = Vec::new();
    let mut has_regs_line = false;
    if let Some(line) = cur.peek()
        && (line == "regs" || line.starts_with("regs "))
    {
        let (_, regs_line) = cur.next().unwrap();
        has_regs_line = true;
        let (r, b) = parse_regs_line(regs_line)?;
        regs = r;
        bounds = b;
    }
    // Blocks until the next function header or end of section.
    let mut blocks = Vec::new();
    while let Some(line) = cur.peek() {
        if is_function_header(line) {
            break;
        }
        if !line.starts_with('^') {
            return Err(err(
                "16.2",
                format!("expected a block header in @{name}, got {line:?}"),
            ));
        }
        blocks.push(parse_block(cur)?);
    }
    if blocks.is_empty() {
        return Err(err("16.2", format!("function @{name} has no blocks")));
    }
    let canonical = cur.lines[start_pos..cur.pos].join("\n");
    Ok(Function {
        kind,
        name,
        lists,
        regs,
        bounds,
        has_regs_line,
        blocks,
        header_line,
        canonical,
    })
}

fn parse_regs_line(line: &str) -> Result<(Vec<Param>, Vec<usize>), String> {
    // regs { a:reg[8], b:reg[1] } [bounds {3, 4}]
    let after = line
        .strip_prefix("regs")
        .ok_or_else(|| err("16.2", format!("malformed regs line {line:?}")))?
        .trim();
    let open = after
        .strip_prefix('{')
        .ok_or_else(|| err("16.2", format!("malformed regs line {line:?}")))?;
    let close = open
        .find('}')
        .ok_or_else(|| err("16.2", format!("malformed regs line {line:?}")))?;
    let inner = &open[..close];
    let rest = open[close + 1..].trim();
    let mut regs = Vec::new();
    for item in split_list(inner) {
        let (name, ty) = item
            .split_once(':')
            .ok_or_else(|| err("16.2", format!("malformed regs entry {item:?}")))?;
        regs.push(Param {
            name: name.to_string(),
            ty: Type::parse(ty).map_err(|e| err("16.2", e))?,
            role: None,
        });
    }
    let mut bounds = Vec::new();
    if !rest.is_empty() {
        let b = rest
            .strip_prefix("bounds")
            .ok_or_else(|| err("16.2", format!("malformed regs line {line:?}")))?
            .trim();
        let b = b
            .strip_prefix('{')
            .and_then(|x| x.strip_suffix('}'))
            .ok_or_else(|| err("16.2", format!("malformed bounds in {line:?}")))?;
        for item in split_list(b) {
            bounds.push(parse_usize(&item).map_err(|e| err("16.2", e))?);
        }
    }
    Ok((regs, bounds))
}

fn parse_block(cur: &mut Cursor) -> Result<Block, String> {
    let (line_no, header) = cur.next().ok_or_else(|| err("16.2", "expected a block"))?;
    let h = header
        .strip_prefix('^')
        .ok_or_else(|| err("16.2", format!("malformed block header {header:?}")))?;
    let h = h
        .strip_suffix(':')
        .ok_or_else(|| err("16.2", format!("block header {header:?} must end with ':'")))?;
    let paren = h
        .find('(')
        .ok_or_else(|| err("16.2", format!("malformed block header {header:?}")))?;
    let name = &h[..paren];
    if !valid_name(name) {
        return Err(err("16.1", format!("invalid block name {name:?}")));
    }
    let params = parse_params(&h[paren..])?;
    let mut instrs = Vec::new();
    while let Some(line) = cur.peek() {
        if line.starts_with('^') || is_function_header(line) {
            break;
        }
        instrs.push(parse_instr(cur)?);
    }
    if instrs.is_empty() {
        return Err(err("16.2", format!("block ^{name} has no instructions")));
    }
    Ok(Block {
        name: name.to_string(),
        params,
        line: line_no,
        instrs,
    })
}

fn parse_instr(cur: &mut Cursor) -> Result<Instr, String> {
    let (line_no, line) = cur
        .next()
        .ok_or_else(|| err("16.2", "expected an instruction"))?;
    let line = line.to_string();
    let mut toks = split_tokens(&line)?;
    let opens = toks.last().map(|t| t.as_str()) == Some("{");
    if opens {
        toks.pop();
    }
    let (lhs, rest): (Vec<String>, Vec<String>) = match toks.iter().position(|t| t == "=") {
        Some(eq) => {
            if eq != 1 {
                return Err(err("16.2", format!("malformed instruction {line:?}")));
            }
            let lhs_tok = toks[0].clone();
            let lhs = if let Some(inner) = paren_inner(&lhs_tok) {
                split_list(inner)
            } else {
                vec![lhs_tok]
            };
            (lhs, toks[2..].to_vec())
        }
        None => (Vec::new(), toks),
    };
    if rest.is_empty() {
        return Err(err("16.2", format!("malformed instruction {line:?}")));
    }
    let op = rest[0].clone();
    let operands = rest[1..].to_vec();
    let mut nested = Vec::new();
    if opens {
        loop {
            let mut arm = Vec::new();
            loop {
                let next = cur.peek().ok_or_else(|| {
                    err("16.2", format!("unterminated nested block after {line:?}"))
                })?;
                if next == "}" || next == "} {" {
                    break;
                }
                arm.push(parse_instr(cur)?);
            }
            let (_, closer) = cur.next().unwrap();
            nested.push(arm);
            if closer == "}" {
                break;
            }
        }
    }
    Ok(Instr {
        lhs,
        op,
        operands,
        nested,
        line: line_no,
        raw: line,
    })
}
