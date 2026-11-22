//! Number formatting for the value panel and clipboard.
use crate::expr::{bf16_bits, f16_value, to_signed};
use num_bigint::{BigInt, BigUint};
use num_traits::ToPrimitive;

fn padded(value: &BigUint, radix: u32, digits: usize) -> String {
    let raw = value.to_str_radix(radix).to_ascii_uppercase();
    if raw.len() >= digits { raw } else { "0".repeat(digits - raw.len()) + &raw }
}

/// Insert `sep` every `group` characters, counting from the right.
pub fn group(digits: &str, group: usize, sep: char) -> String {
    let n = digits.chars().count();
    let mut out = String::with_capacity(n + n / group);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (n - i).is_multiple_of(group) {
            out.push(sep);
        }
        out.push(ch);
    }
    out
}

pub fn hex_digits(width: u32) -> usize {
    width.div_ceil(4) as usize
}

pub fn hex(value: &BigUint, width: u32) -> String {
    padded(value, 16, hex_digits(width))
}

pub fn hex_grouped(value: &BigUint, width: u32) -> String {
    format!("0x{}", group(&hex(value, width), 4, '_'))
}

pub fn oct(value: &BigUint, width: u32) -> String {
    padded(value, 8, width.div_ceil(3) as usize)
}

pub fn bin(value: &BigUint, width: u32) -> String {
    padded(value, 2, width as usize)
}

pub fn dec_grouped(value: &BigInt) -> String {
    let raw = value.magnitude().to_str_radix(10);
    let sign = if value.sign() == num_bigint::Sign::Minus { "−" } else { "" };
    format!("{sign}{}", group(&raw, 3, ','))
}

pub fn verilog(value: &BigUint, width: u32) -> String {
    format!("{width}'h{}", group(&hex(value, width).to_ascii_lowercase(), 4, '_'))
}

/// Decimal text of a word under the active interpretation.
pub fn dec_plain(value: &BigUint, width: u32, signed: bool) -> String {
    if signed { to_signed(value, width).to_string() } else { value.to_string() }
}

fn real(x: f64) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x.is_infinite() {
        if x > 0.0 { "+∞".into() } else { "−∞".into() }
    } else if x == 0.0 {
        if x.is_sign_negative() { "−0".into() } else { "0".into() }
    } else {
        let a = x.abs();
        let s = if (1e-5..1e16).contains(&a) { format!("{x}") } else { format!("{x:e}") };
        s.replace('-', "−")
    }
}

/// IEEE 754 decoding for the widths where it applies:
/// (label, value text, field breakdown).
pub fn floats(value: &BigUint, width: u32) -> Vec<(&'static str, String, String)> {
    let bits = value.to_u64().unwrap_or(0);
    let parts = |sign: u64, exp: u64, man: u64, bias: i64, ebits: u32, mdigits: usize| {
        let max = (1 << ebits) - 1;
        let e = if exp == 0 && man == 0 {
            "zero".to_string()
        } else if exp == 0 {
            "subnormal".to_string()
        } else if exp == max {
            if man == 0 { "infinity" } else { "NaN" }.to_string()
        } else {
            format!("2^{}", exp as i64 - bias).replace('-', "−")
        };
        format!("s {sign}  exp 0x{exp:X} ({e})  frac 0x{man:0mdigits$X}")
    };
    match width {
        16 => {
            let h = bits as u16;
            let bf = f32::from_bits((h as u32) << 16) as f64;
            let _ = bf16_bits;
            vec![
                ("F16", real(f16_value(h)), parts((bits >> 15) & 1, (bits >> 10) & 0x1F, bits & 0x3FF, 15, 5, 3)),
                ("BF16", real(bf), parts((bits >> 15) & 1, (bits >> 7) & 0xFF, bits & 0x7F, 127, 8, 2)),
            ]
        }
        32 => vec![(
            "F32",
            real(f32::from_bits(bits as u32) as f64),
            parts((bits >> 31) & 1, (bits >> 23) & 0xFF, bits & 0x7F_FFFF, 127, 8, 6),
        )],
        64 => vec![(
            "F64",
            real(f64::from_bits(bits)),
            parts(bits >> 63, (bits >> 52) & 0x7FF, bits & 0xF_FFFF_FFFF_FFFF, 1023, 11, 13),
        )],
        _ => Vec::new(),
    }
}
