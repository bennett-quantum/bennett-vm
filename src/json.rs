//! Minimal JSON with RFC 8785 (JCS) canonical emission.
//!
//! Numbers keep their source text; JCS emission re-serialises integers
//! exactly and floats through `f64` shortest round-trip (the object
//! schemas of §9.1 carry only integers and decimal strings).

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    /// Raw number text, validated.
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(BTreeMap<String, Json>),
}

impl Json {
    pub fn obj(pairs: Vec<(&str, Json)>) -> Json {
        let mut m = BTreeMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v);
        }
        Json::Obj(m)
    }
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.get(key),
            _ => None,
        }
    }
    pub fn as_obj(&self) -> Option<&BTreeMap<String, Json>> {
        match self {
            Json::Obj(m) => Some(m),
            _ => None,
        }
    }
    pub fn as_arr(&self) -> Option<&Vec<Json>> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    /// Integer value: accepts a JSON integer or a decimal string (§9.1).
    pub fn as_big(&self) -> Option<crate::bigint::BigUint> {
        match self {
            Json::Num(n) => {
                if n.starts_with('-') || n.contains(['.', 'e', 'E']) {
                    None
                } else {
                    crate::bigint::BigUint::parse_decimal(n).ok()
                }
            }
            Json::Str(s) => crate::bigint::BigUint::parse_decimal(s).ok(),
            _ => None,
        }
    }
    pub fn as_u64(&self) -> Option<u64> {
        self.as_big().and_then(|b| b.to_u64())
    }
    pub fn as_usize(&self) -> Option<usize> {
        self.as_big().and_then(|b| b.to_usize())
    }
    /// JCS object emission needs no width knowledge: emit a nonnegative
    /// integer as a JSON number through 2^53-1, else as a decimal string
    /// (§9.1 convention used by the reference CLI output).
    pub fn u(v: &crate::bigint::BigUint) -> Json {
        const MAX_SAFE: u64 = (1u64 << 53) - 1;
        match v.to_u64() {
            Some(n) if n <= MAX_SAFE => Json::Num(n.to_string()),
            _ => Json::Str(v.to_decimal()),
        }
    }
    pub fn u64(v: u64) -> Json {
        Json::u(&crate::bigint::BigUint::from_u64(v))
    }
    pub fn to_jcs(&self) -> String {
        let mut out = String::new();
        self.write_jcs(&mut out);
        out
    }
    fn write_jcs(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Num(n) => out.push_str(&canonical_number(n)),
            Json::Str(s) => write_jcs_string(s, out),
            Json::Arr(a) => {
                out.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write_jcs(out);
                }
                out.push(']');
            }
            Json::Obj(m) => {
                out.push('{');
                // JCS sorts by UTF-16 code units; our keys are ASCII in
                // every §9 schema, where that equals byte order.
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_jcs_string(k, out);
                    out.push(':');
                    v.write_jcs(out);
                }
                out.push('}');
            }
        }
    }
}

fn canonical_number(raw: &str) -> String {
    if raw.bytes().all(|c| c.is_ascii_digit())
        || (raw.starts_with('-') && raw.len() > 1 && raw[1..].bytes().all(|c| c.is_ascii_digit()))
    {
        return raw.to_string();
    }
    match raw.parse::<f64>() {
        Ok(v) if v.is_finite() => format_f64(v),
        _ => raw.to_string(),
    }
}

/// Shortest round-trip decimal (Rust `Display` for f64 is shortest).
pub fn format_f64(v: f64) -> String {
    if v == 0.0 {
        return "0".to_string();
    }
    let s = format!("{v}");
    s
}

fn write_jcs_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

pub struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

pub fn parse(text: &str) -> Result<Json, String> {
    let mut p = Parser {
        b: text.as_bytes(),
        i: 0,
    };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.i != p.b.len() {
        return Err(format!("trailing characters at byte {} of JSON", p.i));
    }
    Ok(v)
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }
    fn peek(&self) -> Result<u8, String> {
        self.b
            .get(self.i)
            .copied()
            .ok_or_else(|| "unexpected end of JSON".to_string())
    }
    fn value(&mut self) -> Result<Json, String> {
        match self.peek()? {
            b'{' => {
                self.i += 1;
                let mut m = BTreeMap::new();
                self.ws();
                if self.peek()? == b'}' {
                    self.i += 1;
                    return Ok(Json::Obj(m));
                }
                loop {
                    self.ws();
                    let k = match self.value()? {
                        Json::Str(s) => s,
                        _ => return Err("JSON object key is not a string".into()),
                    };
                    self.ws();
                    if self.peek()? != b':' {
                        return Err("expected ':' in JSON object".into());
                    }
                    self.i += 1;
                    self.ws();
                    let v = self.value()?;
                    if m.insert(k, v).is_some() {
                        return Err("duplicate key in JSON object".into());
                    }
                    self.ws();
                    match self.peek()? {
                        b',' => self.i += 1,
                        b'}' => {
                            self.i += 1;
                            return Ok(Json::Obj(m));
                        }
                        c => {
                            return Err(format!(
                                "expected ',' or '}}' in JSON object, got {:?}",
                                c as char
                            ));
                        }
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.peek()? == b']' {
                    self.i += 1;
                    return Ok(Json::Arr(a));
                }
                loop {
                    self.ws();
                    a.push(self.value()?);
                    self.ws();
                    match self.peek()? {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return Ok(Json::Arr(a));
                        }
                        c => {
                            return Err(format!(
                                "expected ',' or ']' in JSON array, got {:?}",
                                c as char
                            ));
                        }
                    }
                }
            }
            b'"' => Ok(Json::Str(self.string()?)),
            b't' => self.lit("true", Json::Bool(true)),
            b'f' => self.lit("false", Json::Bool(false)),
            b'n' => self.lit("null", Json::Null),
            _ => self.number(),
        }
    }
    fn lit(&mut self, word: &str, v: Json) -> Result<Json, String> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(format!("invalid JSON literal at byte {}", self.i))
        }
    }
    fn string(&mut self) -> Result<String, String> {
        debug_assert_eq!(self.b[self.i], b'"');
        self.i += 1;
        let mut out = String::new();
        loop {
            let c = self.peek()?;
            self.i += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let e = self.peek()?;
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let cp = self.hex4()?;
                            if (0xd800..0xdc00).contains(&cp) {
                                if self.peek()? == b'\\' {
                                    self.i += 1;
                                    if self.peek()? == b'u' {
                                        self.i += 1;
                                        let lo = self.hex4()?;
                                        if !(0xdc00..0xe000).contains(&lo) {
                                            return Err("invalid surrogate pair".into());
                                        }
                                        let combined = 0x10000
                                            + (((cp - 0xd800) as u32) << 10)
                                            + (lo as u32 - 0xdc00);
                                        out.push(
                                            char::from_u32(combined)
                                                .ok_or("invalid surrogate pair")?,
                                        );
                                    } else {
                                        return Err("lone surrogate in JSON string".into());
                                    }
                                } else {
                                    return Err("lone surrogate in JSON string".into());
                                }
                            } else {
                                out.push(char::from_u32(cp as u32).ok_or("invalid \\u escape")?);
                            }
                        }
                        _ => return Err("invalid escape in JSON string".into()),
                    }
                }
                _ => {
                    // Copy the full UTF-8 sequence starting at the byte just consumed.
                    let start = self.i - 1;
                    let len = utf8_len(c)?;
                    if start + len > self.b.len() {
                        return Err("invalid UTF-8 in JSON string".into());
                    }
                    let s = std::str::from_utf8(&self.b[start..start + len])
                        .map_err(|_| "invalid UTF-8 in JSON string")?;
                    out.push_str(s);
                    self.i = start + len;
                }
            }
        }
    }
    fn hex4(&mut self) -> Result<u16, String> {
        if self.i + 4 > self.b.len() {
            return Err("truncated \\u escape".into());
        }
        let s =
            std::str::from_utf8(&self.b[self.i..self.i + 4]).map_err(|_| "invalid \\u escape")?;
        self.i += 4;
        u16::from_str_radix(s, 16).map_err(|_| "invalid \\u escape".to_string())
    }
    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        if self.peek()? == b'-' {
            self.i += 1;
        }
        while self.i < self.b.len()
            && matches!(
                self.b[self.i],
                b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-'
            )
        {
            self.i += 1;
        }
        if start == self.i {
            return Err(format!("invalid JSON value at byte {start}"));
        }
        let raw = std::str::from_utf8(&self.b[start..self.i])
            .unwrap()
            .to_string();
        raw.parse::<f64>()
            .map_err(|_| format!("invalid JSON number {raw:?}"))?;
        Ok(Json::Num(raw))
    }
}

fn utf8_len(first: u8) -> Result<usize, String> {
    if first < 0x80 {
        Ok(1)
    } else if first >> 5 == 0b110 {
        Ok(2)
    } else if first >> 4 == 0b1110 {
        Ok(3)
    } else if first >> 3 == 0b11110 {
        Ok(4)
    } else {
        Err("invalid UTF-8 lead byte".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_jcs() {
        let v = parse(r#"{"b":1,"a":[true,null,"x"]}"#).unwrap();
        assert_eq!(v.to_jcs(), r#"{"a":[true,null,"x"],"b":1}"#);
        let big = parse(r#"{"0":{"kind":"oracle"}}"#).unwrap();
        assert_eq!(big.to_jcs(), r#"{"0":{"kind":"oracle"}}"#);
    }
}
