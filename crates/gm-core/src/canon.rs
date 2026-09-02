//! Canonical JSON serialisation (RFC 8785 JCS) and content addressing.
//!
//! Every object in the store is identified by the SHA-256 of its canonical
//! form. Two implementations that agree on this module agree on every hash in
//! the file, which is what makes clone/push/pull possible between independent
//! tools. The rules are deliberately boring:
//!
//! * UTF-8, no insignificant whitespace.
//! * Object keys sorted by their UTF-16 code units.
//! * Numbers formatted by the ECMAScript `Number::toString` algorithm.
//! * Strings escaped with the shortest legal escape.

use crate::error::{Error, Result};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// Serialise a JSON value to its canonical form.
pub fn canonicalize(value: &Value) -> Result<String> {
    let mut out = String::new();
    write_value(value, &mut out)?;
    Ok(out)
}

/// Canonicalise then hash, yielding a `sha256-<64 hex chars>` object id.
pub fn hash_value(value: &Value) -> Result<String> {
    Ok(hash_bytes(canonicalize(value)?.as_bytes()))
}

/// Hash raw canonical bytes. Used when re-verifying stored objects, where we
/// must hash exactly what is on disk rather than a re-serialisation of it.
pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256-{}", hex::encode(hasher.finalize()))
}

fn write_value(value: &Value, out: &mut String) -> Result<()> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => out.push_str(&write_number(n)?),
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => write_object(map, out)?,
    }
    Ok(())
}

fn write_object(map: &Map<String, Value>, out: &mut String) -> Result<()> {
    // JCS orders keys by UTF-16 code unit, not by Rust's UTF-8 byte order.
    // The two agree for everything in the Basic Multilingual Plane but disagree
    // once supplementary characters (emoji, some CJK extensions) are involved,
    // and key ordering decides the hash.
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort_by_cached_key(|k| utf16_units(k));

    out.push('{');
    for (i, key) in keys.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_string(key, out);
        out.push(':');
        write_value(&map[*key], out)?;
    }
    out.push('}');
    Ok(())
}

fn utf16_units(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_number(n: &serde_json::Number) -> Result<String> {
    // Integers within the exactly-representable range print identically under
    // both the integer and the ECMAScript double rules, so we can shortcut.
    if let Some(i) = n.as_i64() {
        return Ok(i.to_string());
    }
    if let Some(u) = n.as_u64() {
        return Ok(u.to_string());
    }
    let f = n.as_f64().ok_or(Error::NonFiniteNumber)?;
    ecma_number_to_string(f)
}

/// ECMAScript `Number::toString(x, 10)` (ECMA-262 section 6.1.6.1.20), which
/// RFC 8785 adopts wholesale. Rust's own `{}` and `{:e}` are both
/// shortest-round-trip but choose fixed vs exponential notation on different
/// thresholds, so we take the shortest digits from `{:e}` and re-place the
/// decimal point ourselves.
pub fn ecma_number_to_string(f: f64) -> Result<String> {
    if !f.is_finite() {
        return Err(Error::NonFiniteNumber);
    }
    if f == 0.0 {
        // Canonical form has no signed zero: -0.0 and 0.0 must hash alike.
        return Ok("0".to_string());
    }

    let sign = if f < 0.0 { "-" } else { "" };
    let sci = format!("{:e}", f.abs()); // e.g. "1.23456e2"
    let (mantissa, exp) = sci.split_once('e').expect("LowerExp always emits 'e'");
    let exp: i32 = exp
        .parse()
        .expect("LowerExp always emits an integer exponent");
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let digits = digits.trim_end_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };

    let k = digits.len() as i32; // number of significant digits
    let n = exp + 1; // value == 0.<digits> * 10^n

    let body = if k <= n && n <= 21 {
        format!("{}{}", digits, "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{}", "0".repeat((-n) as usize), digits)
    } else {
        let e = n - 1;
        let esign = if e >= 0 { "+" } else { "-" };
        if k == 1 {
            format!("{}e{}{}", digits, esign, e.abs())
        } else {
            format!("{}.{}e{}{}", &digits[..1], &digits[1..], esign, e.abs())
        }
    };
    Ok(format!("{sign}{body}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keys_are_sorted_and_whitespace_stripped() {
        let v = json!({ "b": 1, "a": { "d": 2, "c": 3 } });
        assert_eq!(canonicalize(&v).unwrap(), r#"{"a":{"c":3,"d":2},"b":1}"#);
    }

    #[test]
    fn key_order_uses_utf16_units_not_utf8_bytes() {
        // The two orderings genuinely disagree here, and key order decides the
        // hash, so getting this wrong would make our commit ids incompatible
        // with every other JCS implementation.
        //
        //   U+1F600  UTF-16 D83D DE00   UTF-8 F0 9F 98 80
        //   U+FF3A   UTF-16 FF3A        UTF-8 EF BC BA
        //
        // By UTF-16 the emoji leads (D83D < FF3A); by UTF-8 bytes the
        // fullwidth Z leads (EF < F0). JCS says UTF-16.
        let emoji = "\u{1F600}";
        let fullwidth_z = "\u{FF3A}";
        assert!(emoji > fullwidth_z, "UTF-8 order should put the emoji last");

        let v = json!({ emoji: 1, fullwidth_z: 2 });
        let out = canonicalize(&v).unwrap();
        assert!(
            out.find(emoji).unwrap() < out.find(fullwidth_z).unwrap(),
            "expected the emoji key first under UTF-16 order, got {out}"
        );
    }

    #[test]
    fn numbers_follow_the_ecmascript_rules() {
        let cases: [(f64, &str); 9] = [
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (-2.5, "-2.5"),
            (1e21, "1e+21"),
            (1e-7, "1e-7"),
            (0.000001, "0.000001"),
            (9.81, "9.81"),
            (1e-6 / 10.0, "1e-7"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                ecma_number_to_string(input).unwrap(),
                expected,
                "for {input}"
            );
        }
    }

    #[test]
    fn control_characters_use_short_escapes() {
        let v = json!({ "k": "a\nb\u{1}c" });
        assert_eq!(canonicalize(&v).unwrap(), "{\"k\":\"a\\nb\\u0001c\"}");
    }

    #[test]
    fn hashing_is_insensitive_to_input_key_order() {
        let a: Value = serde_json::from_str(r#"{"x":1,"y":[2,3]}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"y":[2,3],"x":1}"#).unwrap();
        assert_eq!(hash_value(&a).unwrap(), hash_value(&b).unwrap());
    }
}
