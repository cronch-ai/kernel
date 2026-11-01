//! Deterministic CBOR (RFC 8949 §4.2.1): the one representation of every object Cronch stores, exchanges or hashes.
//!
//! Integers take their shortest form, map keys are sorted by their encoded bytes, and a number with no fractional part
//! that fits in 64 bits is encoded as an integer, so `3` and `3.0` have one encoding. Other numbers are float64.
//!
//! The data model is that of JSON (null, booleans, numbers, text, arrays, maps with text keys); no tags, byte strings
//! or indefinite lengths. Decoding is strict: input is accepted only if it is exactly the encoding of what it decodes
//! to, so every value has one byte representation and one content key.

use serde::Serialize;
use serde_json::{Map, Number, Value};

fn head(out: &mut Vec<u8>, major: u8, n: u64) {
    let m = major << 5;
    match n {
        0..=23 => out.push(m | n as u8),
        24..=0xff => out.extend_from_slice(&[m | 24, n as u8]),
        0x100..=0xffff => {
            out.push(m | 25);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(m | 26);
            out.extend_from_slice(&(n as u32).to_be_bytes());
        }
        _ => {
            out.push(m | 27);
            out.extend_from_slice(&n.to_be_bytes());
        }
    }
}

fn int(out: &mut Vec<u8>, i: i128) {
    if i >= 0 {
        head(out, 0, i as u64)
    } else {
        head(out, 1, (-1 - i) as u64)
    }
}

pub fn encode(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    write(&mut out, value);
    out
}

fn write(out: &mut Vec<u8>, value: &Value) {
    match value {
        Value::Null => out.push(0xf6),
        Value::Bool(false) => out.push(0xf4),
        Value::Bool(true) => out.push(0xf5),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                int(out, i as i128)
            } else if let Some(u) = n.as_u64() {
                int(out, u as i128)
            } else {
                let f = n.as_f64().unwrap_or(f64::NAN);
                if f.fract() == 0.0 && f.abs() < 9.2e18 {
                    int(out, f as i128)
                } else {
                    out.push(0xfb);
                    out.extend_from_slice(&f.to_be_bytes());
                }
            }
        }
        Value::String(s) => {
            head(out, 3, s.len() as u64);
            out.extend_from_slice(s.as_bytes());
        }
        Value::Array(items) => {
            head(out, 4, items.len() as u64);
            for item in items {
                write(out, item);
            }
        }
        Value::Object(map) => {
            let mut entries: Vec<(Vec<u8>, &Value)> = map
                .iter()
                .map(|(k, v)| (encode(&Value::String(k.clone())), v))
                .collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            head(out, 5, entries.len() as u64);
            for (k, v) in entries {
                out.extend_from_slice(&k);
                write(out, v);
            }
        }
    }
}

/// Why bytes are not a deterministic encoding of a value.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DecodeError {
    Truncated {
        offset: usize,
    },
    /// A major type, tag or simple value outside the JSON data model (byte strings, tags, undefined, indefinite lengths).
    Unsupported {
        offset: usize,
        initial: u8,
    },
    InvalidUtf8 {
        offset: usize,
    },
    NonTextKey {
        offset: usize,
    },
    DuplicateKey {
        offset: usize,
    },
    NotFinite {
        offset: usize,
    },
    /// A negative integer below the 64-bit range.
    OutOfRange {
        offset: usize,
    },
    TooDeep {
        offset: usize,
    },
    TrailingBytes {
        offset: usize,
    },
    /// Well-formed, but not the deterministic encoding (lengths not shortest, keys out of order, an integral float).
    NonCanonical,
}

const MAX_DEPTH: usize = 256;

struct Decoder<'b> {
    bytes: &'b [u8],
    offset: usize,
}

impl Decoder<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], DecodeError> {
        let end = self
            .offset
            .checked_add(n)
            .filter(|&e| e <= self.bytes.len());
        let end = end.ok_or(DecodeError::Truncated {
            offset: self.offset,
        })?;
        let slice = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(slice)
    }

    fn argument(&mut self, info: u8, initial_at: usize) -> Result<u64, DecodeError> {
        let width = match info {
            0..=23 => return Ok(info as u64),
            24 => 1,
            25 => 2,
            26 => 4,
            27 => 8,
            _ => {
                return Err(DecodeError::Unsupported {
                    offset: initial_at,
                    initial: self.bytes[initial_at],
                });
            }
        };
        Ok(self
            .take(width)?
            .iter()
            .fold(0u64, |n, b| (n << 8) | *b as u64))
    }

    fn length(&mut self, info: u8, initial_at: usize) -> Result<usize, DecodeError> {
        let n = self.argument(info, initial_at)?;
        usize::try_from(n).map_err(|_| DecodeError::Truncated { offset: initial_at })
    }

    fn value(&mut self, depth: usize) -> Result<Value, DecodeError> {
        let at = self.offset;
        if depth > MAX_DEPTH {
            return Err(DecodeError::TooDeep { offset: at });
        }
        let initial = self.take(1)?[0];
        let (major, info) = (initial >> 5, initial & 0x1f);
        match major {
            0 => Ok(Value::Number(self.argument(info, at)?.into())),
            1 => {
                let n = self.argument(info, at)?;
                let i = i64::try_from(n).map_err(|_| DecodeError::OutOfRange { offset: at })?;
                Ok(Value::Number((-1 - i).into()))
            }
            3 => {
                let len = self.length(info, at)?;
                let text = std::str::from_utf8(self.take(len)?)
                    .map_err(|_| DecodeError::InvalidUtf8 { offset: at })?;
                Ok(Value::String(text.to_string()))
            }
            4 => {
                let len = self.length(info, at)?;
                (0..len)
                    .map(|_| self.value(depth + 1))
                    .collect::<Result<_, _>>()
                    .map(Value::Array)
            }
            5 => {
                let len = self.length(info, at)?;
                let mut map = Map::new();
                for _ in 0..len {
                    let key_at = self.offset;
                    let Value::String(key) = self.value(depth + 1)? else {
                        return Err(DecodeError::NonTextKey { offset: key_at });
                    };
                    let value = self.value(depth + 1)?;
                    if map.insert(key, value).is_some() {
                        return Err(DecodeError::DuplicateKey { offset: key_at });
                    }
                }
                Ok(Value::Object(map))
            }
            7 => match info {
                20 => Ok(Value::Bool(false)),
                21 => Ok(Value::Bool(true)),
                22 => Ok(Value::Null),
                27 => {
                    let bits = self.argument(info, at)?;
                    Number::from_f64(f64::from_bits(bits))
                        .map(Value::Number)
                        .ok_or(DecodeError::NotFinite { offset: at })
                }
                _ => Err(DecodeError::Unsupported {
                    offset: at,
                    initial,
                }),
            },
            _ => Err(DecodeError::Unsupported {
                offset: at,
                initial,
            }),
        }
    }
}

/// Decode a deterministic encoding, rejecting any other.
pub fn decode(bytes: &[u8]) -> Result<Value, DecodeError> {
    let mut decoder = Decoder { bytes, offset: 0 };
    let value = decoder.value(0)?;
    if decoder.offset != bytes.len() {
        return Err(DecodeError::TrailingBytes {
            offset: decoder.offset,
        });
    }
    if encode(&value) != bytes {
        return Err(DecodeError::NonCanonical);
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rfc_vectors() {
        assert_eq!(encode(&json!(0)), [0x00]);
        assert_eq!(encode(&json!(23)), [0x17]);
        assert_eq!(encode(&json!(24)), [0x18, 0x18]);
        assert_eq!(encode(&json!(1000)), [0x19, 0x03, 0xe8]);
        assert_eq!(encode(&json!(-1)), [0x20]);
        assert_eq!(encode(&json!(-1000)), [0x39, 0x03, 0xe7]);
        assert_eq!(encode(&json!("a")), [0x61, 0x61]);
        assert_eq!(encode(&json!([1, 2])), [0x82, 0x01, 0x02]);
        assert_eq!(encode(&json!(1.5)), [0xfb, 0x3f, 0xf8, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn decoding_round_trips() {
        let v = json!({"spec": {"name": "Square", "output": "Int"}, "args": {"n": -3}, "x": [1.5, true, null, "é"]});
        assert_eq!(decode(&encode(&v)), Ok(v));
    }

    #[test]
    fn decoding_rejects_other_encodings() {
        // 1 with a needlessly long argument.
        assert_eq!(decode(&[0x18, 0x01]), Err(DecodeError::NonCanonical));
        // Keys out of order: "aa" before "b".
        assert_eq!(
            decode(&[0xa2, 0x62, b'a', b'a', 0x01, 0x61, b'b', 0x02]),
            Err(DecodeError::NonCanonical)
        );
        // 3.0 as a float.
        assert_eq!(
            decode(&[0xfb, 0x40, 0x08, 0, 0, 0, 0, 0, 0]),
            Err(DecodeError::NonCanonical)
        );
        assert_eq!(
            decode(&[0x01, 0x01]),
            Err(DecodeError::TrailingBytes { offset: 1 })
        );
        assert_eq!(
            decode(&[0x40]),
            Err(DecodeError::Unsupported {
                offset: 0,
                initial: 0x40
            })
        );
        assert_eq!(
            decode(&[0x62, b'a']),
            Err(DecodeError::Truncated { offset: 1 })
        );
    }

    #[test]
    fn integral_floats_are_integers() {
        assert_eq!(encode(&json!(3.0)), encode(&json!(3)));
    }

    #[test]
    fn map_keys_sort_by_encoding() {
        // Shorter keys sort first: "b" (0x61 0x62) before "aa" (0x62 0x61 0x61).
        assert_eq!(
            encode(&json!({"aa": 1, "b": 2})),
            [0xa2, 0x61, b'b', 0x02, 0x62, b'a', b'a', 0x01]
        );
    }
}
