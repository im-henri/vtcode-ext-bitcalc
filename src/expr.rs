//! Expression language: C operators plus Verilog literals, bit/part selects
//! and concatenation. Evaluation is exact (unbounded two's complement); the
//! caller truncates the final result to the active word width.
use num_bigint::{BigInt, BigUint};
use num_integer::Integer;
use num_traits::{One, Signed, ToPrimitive, Zero};
use std::collections::BTreeMap;

/// Largest supported word width.
pub const MAX_WIDTH: u32 = 8192;
/// Upper bound on intermediate magnitudes, shift amounts and bit indices.
const MAX_BITS: u64 = 1 << 20;

#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub message: String,
    /// Byte offset in the source the error refers to.
    pub pos: Option<usize>,
}

impl Error {
    fn at(message: impl Into<String>, pos: usize) -> Self {
        Self { message: message.into(), pos: Some(pos) }
    }
    fn new(message: impl Into<String>) -> Self {
        Self { message: message.into(), pos: None }
    }
}

type Result<T> = std::result::Result<T, Error>;

/// A named value. `value` is stored truncated to `width` bits.
#[derive(Clone, Debug, PartialEq)]
pub struct Var {
    pub value: BigUint,
    pub width: u32,
}

/// A named bit range of the current value (register field).
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub name: String,
    pub hi: u32,
    pub lo: u32,
}

pub struct Env<'a> {
    pub width: u32,
    pub signed: bool,
    pub ans: &'a BigUint,
    pub vars: &'a BTreeMap<String, Var>,
    pub fields: &'a [Field],
    /// Set during evaluation when the current value (or a field of it) is read.
    pub used_ans: std::cell::Cell<bool>,
}

impl Env<'_> {
    fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }
}

/// Parse `name[hi:lo] name[i], …` into fields, rejecting overlaps.
pub fn parse_fields(spec: &str) -> std::result::Result<Vec<Field>, String> {
    let mut out: Vec<Field> = Vec::new();
    for item in spec.split([',', ' ', ';']).filter(|s| !s.trim().is_empty()) {
        let item = item.trim();
        let (name, rest) = item
            .split_once('[')
            .ok_or_else(|| format!("Write fields as name[hi:lo], not '{item}'"))?;
        let range = rest.strip_suffix(']').ok_or_else(|| format!("Missing ']' in '{item}'"))?;
        let valid_name = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid_name || name == "ans" {
            return Err(format!("'{name}' is not a usable field name"));
        }
        if FUNCTIONS.iter().any(|f| f.name == name) {
            return Err(format!("'{name}' is a function name"));
        }
        let num = |t: &str| t.trim().parse::<u32>().map_err(|_| format!("Bad bit index in '{item}'"));
        let (hi, lo) = match range.split_once(':') {
            Some((h, l)) => (num(h)?, num(l)?),
            None => (num(range)?, num(range)?),
        };
        if hi < lo {
            return Err(format!("Write {name} high bit first: [{lo}:{hi}]"));
        }
        if hi >= MAX_WIDTH {
            return Err(format!("{name} is beyond bit {}", MAX_WIDTH - 1));
        }
        if let Some(other) = out.iter().find(|f| f.lo <= hi && lo <= f.hi) {
            return Err(format!("{name} overlaps {}", other.name));
        }
        if out.iter().any(|f| f.name == name) {
            return Err(format!("{name} is defined twice"));
        }
        out.push(Field { name: name.to_string(), hi, lo });
    }
    out.sort_by_key(|f| std::cmp::Reverse(f.hi));
    Ok(out)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Ans,
    Var(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// Exact result. For a field write this is the whole updated word.
    pub value: BigInt,
    /// Set for `name = …`, `name[hi:lo] = …` and `[hi:lo] = …`.
    pub assign: Option<Target>,
    /// Field written by an assignment, as (hi, lo).
    pub field: Option<(u32, u32)>,
    /// Advisory note, e.g. a field value that did not fit.
    pub note: Option<String>,
}

pub fn mask_bits(width: u64) -> BigInt {
    (BigInt::one() << width) - 1
}

/// Truncate to `width` bits, two's complement.
pub fn truncate(value: &BigInt, width: u32) -> BigUint {
    (value & mask_bits(width as u64)).magnitude().clone()
}

/// Interpret a `width`-bit word as two's complement.
pub fn to_signed(value: &BigUint, width: u32) -> BigInt {
    if width > 0 && value.bit(width as u64 - 1) {
        BigInt::from(value.clone()) - (BigInt::one() << width)
    } else {
        BigInt::from(value.clone())
    }
}

/// Minimum width able to hold `value` (two's complement when negative or
/// when `signed`).
pub fn bits_needed(value: &BigInt, signed: bool) -> u64 {
    if value.is_negative() {
        // -(2^(n-1)) needs n bits
        let m = (-value - 1u32).bits();
        m + 1
    } else if signed {
        value.bits() + 1
    } else {
        value.bits().max(1)
    }
}

pub fn fits(value: &BigInt, width: u32, signed: bool) -> bool {
    if signed {
        bits_needed(value, true) <= width as u64
    } else {
        !value.is_negative() && value.bits() <= width as u64
    }
}

// ───────────────────────────── lexer ─────────────────────────────

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(BigInt, Option<u32>),
    Float(f64),
    Ident(String),
    Op(&'static str),
    End,
}

#[derive(Clone, Debug)]
struct Token {
    tok: Tok,
    pos: usize,
}

const OPS: [&str; 33] = [
    ">>>", "<<<", "**", "<<", ">>", "<=", ">=", "==", "!=", "&&", "||", "+:", "-:", "(", ")",
    "[", "]", "{", "}", ",", ":", "=", "<", ">", "+", "-", "*", "/", "%", "&", "|", "^", "~",
];

fn lex(src: &str) -> Result<Vec<Token>> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if c.is_ascii_digit() || c == b'\'' || (c == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let (tok, end) = lex_number(src, i)?;
            out.push(Token { tok, pos: start });
            i = end;
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' || c == b'$' {
            let mut end = i + 1;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            let name = src[i..end].trim_start_matches('$').to_string();
            out.push(Token { tok: Tok::Ident(name), pos: start });
            i = end;
            continue;
        }
        if c == b'!' && bytes.get(i + 1) != Some(&b'=') {
            out.push(Token { tok: Tok::Op("!"), pos: start });
            i += 1;
            continue;
        }
        match OPS.iter().find(|op| src[i..].starts_with(**op)) {
            Some(op) => {
                let op: &'static str = if *op == "<<<" { "<<" } else { op };
                out.push(Token { tok: Tok::Op(op), pos: start });
                i += if op == "<<" && src[i..].starts_with("<<<") { 3 } else { op.len() };
            }
            None => {
                let ch = src[i..].chars().next().unwrap_or('?');
                return Err(Error::at(format!("Unexpected character '{ch}'"), i));
            }
        }
    }
    out.push(Token { tok: Tok::End, pos: src.len() });
    Ok(out)
}

fn digits_value(digits: &str, radix: u32, pos: usize) -> Result<BigInt> {
    let clean: String = digits.chars().filter(|c| *c != '_').collect();
    if clean.is_empty() {
        return Err(Error::at("Missing digits", pos));
    }
    if let Some(bad) = clean.chars().find(|c| !c.is_digit(radix)) {
        let name = match radix {
            2 => "binary",
            8 => "octal",
            16 => "hex",
            _ => "decimal",
        };
        if matches!(bad, 'x' | 'X' | 'z' | 'Z' | '?') {
            return Err(Error::at("x/z digits have no numeric value here", pos));
        }
        return Err(Error::at(format!("'{bad}' is not a {name} digit"), pos));
    }
    BigInt::parse_bytes(clean.as_bytes(), radix).ok_or_else(|| Error::at("Invalid number", pos))
}

fn lex_number(src: &str, start: usize) -> Result<(Tok, usize)> {
    let bytes = src.as_bytes();
    let take = |mut i: usize, ok: &dyn Fn(u8) -> bool| {
        while i < bytes.len() && (ok(bytes[i]) || bytes[i] == b'_') {
            i += 1;
        }
        i
    };
    let alnum = |b: u8| b.is_ascii_alphanumeric();
    let mut i = start;
    let mut size: Option<u64> = None;
    if bytes[i] != b'\'' {
        // 0x / 0b / 0o prefixes
        if bytes[i] == b'0' && i + 1 < bytes.len() {
            let radix = match bytes[i + 1] {
                b'x' | b'X' => Some(16),
                b'b' | b'B' => Some(2),
                b'o' | b'O' => Some(8),
                _ => None,
            };
            if let Some(radix) = radix {
                let end = take(i + 2, &alnum);
                let value = digits_value(&src[i + 2..end], radix, start)?;
                return Ok((Tok::Num(value, None), end));
            }
        }
        let end = take(i, &|b| b.is_ascii_digit());
        // float literal
        let mut fend = end;
        if bytes.get(fend) == Some(&b'.') && bytes.get(fend + 1).is_some_and(u8::is_ascii_digit) {
            fend = take(fend + 1, &|b| b.is_ascii_digit());
        }
        if matches!(bytes.get(fend), Some(b'e' | b'E'))
            && (bytes.get(fend + 1).is_some_and(u8::is_ascii_digit)
                || (matches!(bytes.get(fend + 1), Some(b'+' | b'-'))
                    && bytes.get(fend + 2).is_some_and(u8::is_ascii_digit)))
        {
            fend = take(fend + 2, &|b| b.is_ascii_digit());
        }
        if fend != end || src[i..end].starts_with('.') {
            let text: String = src[i..fend].chars().filter(|c| *c != '_').collect();
            let value: f64 = text.parse().map_err(|_| Error::at("Invalid float", start))?;
            return Ok((Tok::Float(value), fend));
        }
        let value = digits_value(&src[i..end], 10, start)?;
        if bytes.get(end) != Some(&b'\'') {
            // K/M/G/T binary multipliers: 4K = 4096
            if let Some(&suffix) = bytes.get(end)
                && !bytes.get(end + 1).is_some_and(|b| alnum(*b) || *b == b'_')
            {
                let power = match suffix.to_ascii_uppercase() {
                    b'K' => Some(10),
                    b'M' => Some(20),
                    b'G' => Some(30),
                    b'T' => Some(40),
                    b'P' => Some(50),
                    _ => None,
                };
                if let Some(power) = power {
                    return Ok((Tok::Num(value << power, None), end + 1));
                }
            }
            if bytes.get(end).is_some_and(|b| alnum(*b)) {
                return Err(Error::at(
                    "Unexpected letter after number (hex needs 0x or 'h)",
                    end,
                ));
            }
            return Ok((Tok::Num(value, None), end));
        }
        let size_value = value.to_u64().filter(|s| *s >= 1 && *s <= MAX_BITS);
        size = Some(size_value.ok_or_else(|| Error::at("Literal size must be at least 1", start))?);
        i = end;
    }
    // Verilog based literal: [size]'[s]<base><digits>
    i += 1;
    let mut signed = false;
    if matches!(bytes.get(i), Some(b's' | b'S')) {
        signed = true;
        i += 1;
    }
    let radix = match bytes.get(i).map(u8::to_ascii_lowercase) {
        Some(b'h') => 16,
        Some(b'b') => 2,
        Some(b'o') => 8,
        Some(b'd') => 10,
        _ => return Err(Error::at("Expected h, b, o or d after '", i)),
    };
    i += 1;
    while matches!(bytes.get(i), Some(b' ')) {
        i += 1;
    }
    let end = take(i, &|b| b.is_ascii_alphanumeric() || b == b'?');
    let value = digits_value(&src[i..end], radix, i)?;
    match size {
        Some(size) => {
            let size = size as u32;
            let word = truncate(&value, size);
            let value = if signed { to_signed(&word, size) } else { BigInt::from(word) };
            Ok((Tok::Num(value, Some(size)), end))
        }
        None => Ok((Tok::Num(value, None), end)),
    }
}

// ───────────────────────────── parser ─────────────────────────────

#[derive(Clone, Debug)]
enum Expr {
    Num(BigInt, Option<u32>),
    Float(f64, usize),
    Var(String, usize),
    Ans,
    Unary(&'static str, Box<Expr>),
    Binary(&'static str, Box<Expr>, Box<Expr>, usize),
    Select(Box<Expr>, Sel, usize),
    Concat(Vec<Expr>),
    Repeat(Box<Expr>, Vec<Expr>, usize),
    Call(String, Vec<Expr>, usize),
}

#[derive(Clone, Debug)]
enum Sel {
    Bit(Box<Expr>),
    Range(Box<Expr>, Box<Expr>),
    Up(Box<Expr>, Box<Expr>),
    Down(Box<Expr>, Box<Expr>),
}

struct Parser {
    toks: Vec<Token>,
    i: usize,
}

fn binary_prec(op: &str) -> Option<(u8, bool)> {
    // (precedence, right associative)
    Some(match op {
        "||" => (1, false),
        "&&" => (2, false),
        "|" => (3, false),
        "^" => (4, false),
        "&" => (5, false),
        "==" | "!=" => (6, false),
        "<" | "<=" | ">" | ">=" => (7, false),
        "<<" | ">>" | ">>>" => (8, false),
        "+" | "-" => (9, false),
        "*" | "/" | "%" => (10, false),
        "**" => (12, true),
        _ => return None,
    })
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.i].tok
    }
    fn pos(&self) -> usize {
        self.toks[self.i].pos
    }
    fn next(&mut self) -> Token {
        let t = self.toks[self.i].clone();
        if self.i + 1 < self.toks.len() {
            self.i += 1;
        }
        t
    }
    fn eat(&mut self, op: &str) -> bool {
        if matches!(self.peek(), Tok::Op(o) if *o == op) {
            self.next();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, op: &str) -> Result<()> {
        if self.eat(op) {
            Ok(())
        } else {
            Err(Error::at(format!("Expected '{op}'"), self.pos()))
        }
    }

    fn expr(&mut self, min: u8) -> Result<Expr> {
        let mut left = self.unary()?;
        while let Tok::Op(op) = self.peek() {
            let (op, pos) = (*op, self.pos());
            let Some((prec, right)) = binary_prec(op) else { break };
            if prec < min {
                break;
            }
            self.next();
            let rhs = self.expr(if right { prec } else { prec + 1 })?;
            left = Expr::Binary(op, Box::new(left), Box::new(rhs), pos);
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr> {
        if let Tok::Op(op @ ("-" | "+" | "~" | "!")) = self.peek().clone() {
            self.next();
            // Unary binds tighter than everything but **: -2**2 = -(2**2)
            let operand = self.expr(11)?;
            return Ok(Expr::Unary(op, Box::new(operand)));
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<Expr> {
        let mut e = self.primary()?;
        while *self.peek() == Tok::Op("[") {
            let pos = self.pos();
            self.next();
            let sel = self.selector()?;
            e = Expr::Select(Box::new(e), sel, pos);
        }
        Ok(e)
    }

    /// After '[': parse `i]`, `hi:lo]`, `lo+:n]` or `hi-:n]`.
    fn selector(&mut self) -> Result<Sel> {
        let a = self.expr(0)?;
        let sel = if self.eat(":") {
            Sel::Range(Box::new(a), Box::new(self.expr(0)?))
        } else if self.eat("+:") {
            Sel::Up(Box::new(a), Box::new(self.expr(0)?))
        } else if self.eat("-:") {
            Sel::Down(Box::new(a), Box::new(self.expr(0)?))
        } else {
            Sel::Bit(Box::new(a))
        };
        self.expect("]")?;
        Ok(sel)
    }

    fn primary(&mut self) -> Result<Expr> {
        let t = self.next();
        match t.tok {
            Tok::Num(v, w) => Ok(Expr::Num(v, w)),
            Tok::Float(f) => Ok(Expr::Float(f, t.pos)),
            Tok::Ident(name) => {
                if *self.peek() == Tok::Op("(") {
                    self.next();
                    let mut args = Vec::new();
                    if !self.eat(")") {
                        loop {
                            args.push(self.expr(0)?);
                            if self.eat(")") {
                                break;
                            }
                            self.expect(",")?;
                        }
                    }
                    return Ok(Expr::Call(name.to_ascii_lowercase(), args, t.pos));
                }
                if name == "ans" || name == "_" {
                    return Ok(Expr::Ans);
                }
                Ok(Expr::Var(name, t.pos))
            }
            Tok::Op("(") => {
                let e = self.expr(0)?;
                self.expect(")")?;
                Ok(e)
            }
            Tok::Op("[") => {
                // [hi:lo] alone selects from ans
                let sel = self.selector()?;
                Ok(Expr::Select(Box::new(Expr::Ans), sel, t.pos))
            }
            Tok::Op("{") => {
                let first = self.expr(0)?;
                if *self.peek() == Tok::Op("{") {
                    // replication {n{a, b}}
                    self.next();
                    let mut items = vec![self.expr(0)?];
                    while self.eat(",") {
                        items.push(self.expr(0)?);
                    }
                    self.expect("}")?;
                    self.expect("}")?;
                    return Ok(Expr::Repeat(Box::new(first), items, t.pos));
                }
                let mut items = vec![first];
                while self.eat(",") {
                    items.push(self.expr(0)?);
                }
                self.expect("}")?;
                Ok(Expr::Concat(items))
            }
            Tok::End => Err(Error::at("Expression is incomplete", t.pos)),
            Tok::Op(op) => Err(Error::at(format!("Unexpected '{op}'"), t.pos)),
        }
    }
}

// ─────────────────────────── evaluation ───────────────────────────

#[derive(Clone, Debug)]
struct V {
    v: BigInt,
    w: Option<u32>,
}

impl V {
    fn new(v: BigInt, w: Option<u32>) -> Self {
        Self { v, w }
    }
    fn bit(b: bool) -> Self {
        Self::new(BigInt::from(b as u8), Some(1))
    }
}

fn combine(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        _ => None,
    }
}

fn check_size(v: &BigInt, pos: usize) -> Result<()> {
    if v.bits() > MAX_BITS {
        Err(Error::at("Intermediate result exceeds 2^20 bits", pos))
    } else {
        Ok(())
    }
}

pub fn evaluate(src: &str, env: &Env) -> Result<Outcome> {
    let toks = lex(src)?;
    let mut p = Parser { toks, i: 0 };
    if *p.peek() == Tok::End {
        return Err(Error::new("Empty expression"));
    }
    let lhs = p.expr(0)?;
    if *p.peek() == Tok::Op("=") {
        let eq = p.pos();
        p.next();
        let rhs = p.expr(0)?;
        if *p.peek() != Tok::End {
            return Err(Error::at("Unexpected input after expression", p.pos()));
        }
        return assign(lhs, rhs, eq, env);
    }
    if *p.peek() != Tok::End {
        let msg = match p.peek() {
            Tok::Op(")") => "Unmatched ')'".to_string(),
            Tok::Op(op) => format!("Unexpected '{op}'"),
            _ => "Missing operator".to_string(),
        };
        return Err(Error::at(msg, p.pos()));
    }
    let v = eval(&lhs, env)?;
    Ok(Outcome { value: v.v, assign: None, field: None, note: None })
}

fn assign(lhs: Expr, rhs: Expr, eq: usize, env: &Env) -> Result<Outcome> {
    let value = eval(&rhs, env)?.v;
    let target = |e: &Expr| -> Result<Target> {
        match e {
            Expr::Ans => Ok(Target::Ans),
            Expr::Var(name, pos) => {
                if FUNCTIONS.iter().any(|f| f.name == name) {
                    Err(Error::at(format!("'{name}' is a function name"), *pos))
                } else {
                    Ok(Target::Var(name.clone()))
                }
            }
            _ => Err(Error::at("Only a name or a bit range can be assigned", eq)),
        }
    };
    // A field name stands for its range of ans: `rd = 5`, `rd[1:0] = 3`.
    let lhs = match lhs {
        Expr::Var(name, pos) if env.field(&name).is_some() => {
            let f = env.field(&name).unwrap();
            Expr::Select(
                Box::new(Expr::Ans),
                Sel::Range(Box::new(Expr::Num(f.hi.into(), None)), Box::new(Expr::Num(f.lo.into(), None))),
                pos,
            )
        }
        Expr::Select(base, sel, pos) if matches!(&*base, Expr::Var(n, _) if env.field(n).is_some()) => {
            let Expr::Var(name, _) = *base else { unreachable!() };
            let f = env.field(&name).unwrap().clone();
            let (hi, lo) = sel_range(&sel, env, pos)?;
            if hi > f.hi - f.lo {
                return Err(Error::at(format!("{name} has only {} bits", f.hi - f.lo + 1), pos));
            }
            Expr::Select(
                Box::new(Expr::Ans),
                Sel::Range(Box::new(Expr::Num((f.lo + hi).into(), None)), Box::new(Expr::Num((f.lo + lo).into(), None))),
                pos,
            )
        }
        other => other,
    };
    match lhs {
        Expr::Select(base, sel, pos) => {
            let target = target(&base)?;
            let (word, width) = match &target {
                Target::Ans => (env.ans.clone(), env.width),
                Target::Var(name) => {
                    let var = env.vars.get(name).ok_or_else(|| {
                        Error::at(format!("Unknown name '{name}'; assign it first"), pos)
                    })?;
                    (var.value.clone(), var.width)
                }
            };
            let (hi, lo) = sel_range(&sel, env, pos)?;
            if hi >= width {
                return Err(Error::at(
                    format!("Bit {hi} is outside the {width}-bit value"),
                    pos,
                ));
            }
            let n = hi - lo + 1;
            let field = truncate(&value, n);
            let note = (!fits(&value, n, value.is_negative())).then(|| {
                format!("Value needs {} bits; kept the low {n}", bits_needed(&value, false))
            });
            let mask = mask_bits(n as u64) << lo;
            let word = BigInt::from(word);
            let updated = (word & !mask.clone()) | (BigInt::from(field) << lo);
            Ok(Outcome { value: updated, assign: Some(target), field: Some((hi, lo)), note })
        }
        other => Ok(Outcome { value, assign: Some(target(&other)?), field: None, note: None }),
    }
}

fn read_word(value: &BigUint, width: u32, signed: bool) -> BigInt {
    if signed { to_signed(value, width) } else { BigInt::from(value.clone()) }
}

fn index(v: &V, pos: usize, what: &str) -> Result<u32> {
    match v.v.to_u64() {
        Some(i) if i <= MAX_BITS && !v.v.is_negative() => Ok(i as u32),
        _ => Err(Error::at(format!("{what} must be between 0 and 2^20"), pos)),
    }
}

fn sel_range(sel: &Sel, env: &Env, pos: usize) -> Result<(u32, u32)> {
    let ev = |e: &Expr, what| -> Result<u32> { index(&eval(e, env)?, pos, what) };
    Ok(match sel {
        Sel::Bit(i) => {
            let i = ev(i, "Bit index")?;
            (i, i)
        }
        Sel::Range(hi, lo) => {
            let (hi, lo) = (ev(hi, "Bit index")?, ev(lo, "Bit index")?);
            if hi < lo {
                return Err(Error::at(
                    format!("Write ranges high bit first: [{lo}:{hi}]"),
                    pos,
                ));
            }
            (hi, lo)
        }
        Sel::Up(lo, n) => {
            let (lo, n) = (ev(lo, "Bit index")?, ev(n, "Width")?);
            if n == 0 {
                return Err(Error::at("Width must be at least 1", pos));
            }
            (lo + n - 1, lo)
        }
        Sel::Down(hi, n) => {
            let (hi, n) = (ev(hi, "Bit index")?, ev(n, "Width")?);
            if n == 0 || n > hi + 1 {
                return Err(Error::at("Part select runs below bit 0", pos));
            }
            (hi, hi + 1 - n)
        }
    })
}

fn eval(e: &Expr, env: &Env) -> Result<V> {
    match e {
        Expr::Num(v, w) => Ok(V::new(v.clone(), *w)),
        Expr::Float(_, pos) => Err(Error::at(
            "Fractions are only accepted by f16(), bf16(), f32() and f64()",
            *pos,
        )),
        Expr::Ans => Ok({
            env.used_ans.set(true);
            V::new(read_word(env.ans, env.width, env.signed), Some(env.width))
        }),
        Expr::Var(name, _) if env.field(name).is_some() => {
            let f = env.field(name).unwrap();
            env.used_ans.set(true);
            let n = f.hi - f.lo + 1;
            Ok(V::new(BigInt::from(truncate(&BigInt::from(env.ans >> f.lo), n)), Some(n)))
        }
        Expr::Var(name, pos) => match env.vars.get(name) {
            Some(var) => Ok(V::new(read_word(&var.value, var.width, env.signed), Some(var.width))),
            None => match name.as_str() {
                "pi" | "e" | "inf" | "nan" => Err(Error::at(
                    format!("'{name}' is only meaningful inside f32()/f64()"),
                    *pos,
                )),
                _ => Err(Error::at(format!("Unknown name '{name}'"), *pos)),
            },
        },
        Expr::Unary(op, a) => {
            let a = eval(a, env)?;
            Ok(match *op {
                "-" => V::new(-a.v, a.w),
                "+" => a,
                "~" => V::new(!a.v, a.w),
                _ => V::bit(a.v.is_zero()),
            })
        }
        Expr::Binary(op, a, b, pos) => {
            let a = eval(a, env)?;
            if *op == "&&" && a.v.is_zero() {
                return Ok(V::bit(false));
            }
            if *op == "||" && !a.v.is_zero() {
                return Ok(V::bit(true));
            }
            let b = eval(b, env)?;
            binary(op, a, b, *pos, env)
        }
        Expr::Select(base, sel, pos) => {
            let base = eval(base, env)?;
            let (hi, lo) = sel_range(sel, env, *pos)?;
            let n = hi - lo + 1;
            Ok(V::new(BigInt::from(truncate(&(base.v >> lo), n)), Some(n)))
        }
        Expr::Concat(items) => {
            let mut acc = BigInt::zero();
            let mut total: u64 = 0;
            for item in items {
                let v = eval(item, env)?;
                let w = v.w.unwrap_or(env.width);
                total += w as u64;
                if total > MAX_BITS {
                    return Err(Error::new("Concatenation exceeds 2^20 bits"));
                }
                acc = (acc << w) | BigInt::from(truncate(&v.v, w));
            }
            Ok(V::new(acc, Some(total as u32)))
        }
        Expr::Repeat(count, items, pos) => {
            let n = index(&eval(count, env)?, *pos, "Repeat count")?;
            if n == 0 {
                return Err(Error::at("Repeat count must be at least 1", *pos));
            }
            let inner = eval(&Expr::Concat(items.clone()), env)?;
            let w = inner.w.unwrap_or(env.width) as u64;
            if w * n as u64 > MAX_BITS {
                return Err(Error::at("Replication exceeds 2^20 bits", *pos));
            }
            let mut acc = BigInt::zero();
            for _ in 0..n {
                acc = (acc << w) | &inner.v;
            }
            Ok(V::new(acc, Some((w * n as u64) as u32)))
        }
        Expr::Call(name, args, pos) => call(name, args, *pos, env),
    }
}

fn binary(op: &str, a: V, b: V, pos: usize, env: &Env) -> Result<V> {
    let w = combine(a.w, b.w);
    let shift = |b: &V| index(b, pos, "Shift amount");
    let v = match op {
        "+" => V::new(a.v + b.v, w),
        "-" => V::new(a.v - b.v, w),
        "*" => {
            if a.v.bits() + b.v.bits() > MAX_BITS {
                return Err(Error::at("Product exceeds 2^20 bits", pos));
            }
            V::new(a.v * b.v, w)
        }
        "/" | "%" => {
            if b.v.is_zero() {
                return Err(Error::at("Division by zero", pos));
            }
            if op == "/" { V::new(a.v / b.v, w) } else { V::new(a.v % b.v, w) }
        }
        "**" => {
            if b.v.is_negative() {
                return Err(Error::at("Negative exponent has no integer result", pos));
            }
            let e = b.v.to_u64().unwrap_or(u64::MAX);
            let base_bits = a.v.bits();
            if base_bits > 1 && (base_bits - 1).saturating_mul(e) > MAX_BITS {
                return Err(Error::at("Power exceeds 2^20 bits", pos));
            }
            V::new(num_traits::pow(a.v, e as usize), a.w)
        }
        "<<" => {
            let n = shift(&b)?;
            let r = a.v << n;
            check_size(&r, pos)?;
            V::new(r, a.w)
        }
        ">>" => {
            let wa = a.w.unwrap_or(env.width);
            let word = if a.v.is_negative() { BigInt::from(truncate(&a.v, wa)) } else { a.v };
            V::new(word >> shift(&b)?, a.w)
        }
        ">>>" => {
            let wa = a.w.unwrap_or(env.width);
            let word = to_signed(&truncate(&a.v, wa), wa);
            V::new(word >> shift(&b)?, a.w)
        }
        "&" => V::new(a.v & b.v, w),
        "|" => V::new(a.v | b.v, w),
        "^" => V::new(a.v ^ b.v, w),
        "==" => V::bit(a.v == b.v),
        "!=" => V::bit(a.v != b.v),
        "<" => V::bit(a.v < b.v),
        "<=" => V::bit(a.v <= b.v),
        ">" => V::bit(a.v > b.v),
        ">=" => V::bit(a.v >= b.v),
        "&&" => V::bit(!a.v.is_zero() && !b.v.is_zero()),
        "||" => V::bit(!a.v.is_zero() || !b.v.is_zero()),
        _ => return Err(Error::at(format!("Unknown operator {op}"), pos)),
    };
    Ok(v)
}

// ─────────────────────────── functions ───────────────────────────

pub struct Function {
    pub name: &'static str,
    pub args: &'static str,
    pub help: &'static str,
}

macro_rules! functions {
    ($($name:literal $args:literal $help:literal,)*) => {
        pub const FUNCTIONS: &[Function] = &[$(Function { name: $name, args: $args, help: $help },)*];
    };
}

functions! {
    "mask" "n | hi, lo" "n ones, or ones in bits hi..lo",
    "bit" "n" "1 << n",
    "set" "x, i [, lo]" "set bit i (or bits i..lo)",
    "clr" "x, i [, lo]" "clear bit i (or bits i..lo)",
    "flip" "x, i [, lo]" "invert bit i (or bits i..lo)",
    "ins" "x, hi, lo, v" "write v into bits hi..lo of x",
    "sext" "x, w" "sign-extend the low w bits",
    "zext" "x, w" "keep the low w bits",
    "popcount" "x" "number of 1 bits",
    "parity" "x" "1 if the number of 1 bits is odd",
    "clz" "x [, w]" "leading zeros in a w-bit word",
    "ctz" "x" "trailing zeros",
    "msb" "x" "index of the highest 1 (−1 if none)",
    "lsb" "x" "index of the lowest 1 (−1 if none)",
    "clog2" "x" "bits needed to index x items",
    "log2" "x" "floor(log2 x)",
    "rev" "x [, w]" "reverse bit order",
    "bswap" "x [, w]" "reverse byte order",
    "rol" "x, n [, w]" "rotate left within w bits",
    "ror" "x, n [, w]" "rotate right within w bits",
    "gray" "x" "binary → Gray code",
    "ungray" "x" "Gray code → binary",
    "alignup" "x, a" "round up to a multiple of a",
    "aligndown" "x, a" "round down to a multiple of a",
    "cdiv" "a, b" "division rounding up",
    "abs" "x" "absolute value",
    "min" "a, b, …" "smallest argument",
    "max" "a, b, …" "largest argument",
    "gcd" "a, b" "greatest common divisor",
    "f16" "real" "IEEE half-precision bits",
    "bf16" "real" "bfloat16 bits",
    "f32" "real" "IEEE single-precision bits",
    "f64" "real" "IEEE double-precision bits",
}

fn float_arg(e: &Expr, env: &Env) -> Result<f64> {
    match e {
        Expr::Float(f, _) => Ok(*f),
        Expr::Unary("-", inner) => Ok(-float_arg(inner, env)?),
        Expr::Unary("+", inner) => float_arg(inner, env),
        Expr::Var(name, _) if !env.vars.contains_key(name) && matches!(name.as_str(), "pi" | "e" | "inf" | "nan") => {
            Ok(match name.as_str() {
                "pi" => std::f64::consts::PI,
                "e" => std::f64::consts::E,
                "inf" => f64::INFINITY,
                _ => f64::NAN,
            })
        }
        Expr::Binary(op @ ("+" | "-" | "*" | "/" | "**"), a, b, _) => {
            let (a, b) = (float_arg(a, env)?, float_arg(b, env)?);
            Ok(match *op {
                "+" => a + b,
                "-" => a - b,
                "*" => a * b,
                "/" => a / b,
                _ => a.powf(b),
            })
        }
        other => {
            let v = eval(other, env)?.v;
            Ok(v.to_f64().unwrap_or(f64::INFINITY))
        }
    }
}

pub fn f16_bits(x: f64) -> u16 {
    // Round-to-nearest-even conversion via f32.
    let f = x as f32;
    let bits = f.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xFF) as i32;
    let man = bits & 0x7F_FFFF;
    if exp == 0xFF {
        return sign | 0x7C00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1F {
        return sign | 0x7C00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        let m = man | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = 1u32 << (shift - 1);
        let mut r = m >> shift;
        let rem = m & ((1 << shift) - 1);
        if rem > half || (rem == half && r & 1 == 1) {
            r += 1;
        }
        return sign | r as u16;
    }
    let mut r = ((e as u32) << 10) | (man >> 13);
    let rem = man & 0x1FFF;
    if rem > 0x1000 || (rem == 0x1000 && r & 1 == 1) {
        r += 1;
    }
    sign | r as u16
}

pub fn f16_value(bits: u16) -> f64 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = ((bits >> 10) & 0x1F) as i32;
    let man = (bits & 0x3FF) as f64;
    sign * match exp {
        0 => man * 2f64.powi(-24),
        0x1F if man == 0.0 => f64::INFINITY,
        0x1F => f64::NAN,
        _ => (1.0 + man / 1024.0) * 2f64.powi(exp - 15),
    }
}

pub fn bf16_bits(x: f64) -> u16 {
    let bits = (x as f32).to_bits();
    if (x as f32).is_nan() {
        return ((bits >> 16) | 0x40) as u16;
    }
    let round = 0x7FFF + ((bits >> 16) & 1);
    (bits.wrapping_add(round) >> 16) as u16
}

fn call(name: &str, args: &[Expr], pos: usize, env: &Env) -> Result<V> {
    let Some(f) = FUNCTIONS.iter().find(|f| f.name == name) else {
        return Err(Error::at(format!("Unknown function '{name}'"), pos));
    };
    let arity_error = || Error::at(format!("Usage: {}({})", f.name, f.args), pos);
    if matches!(name, "f16" | "bf16" | "f32" | "f64") {
        let [arg] = args else { return Err(arity_error()) };
        let x = float_arg(arg, env)?;
        return Ok(match name {
            "f16" => V::new(BigInt::from(f16_bits(x)), Some(16)),
            "bf16" => V::new(BigInt::from(bf16_bits(x)), Some(16)),
            "f32" => V::new(BigInt::from((x as f32).to_bits()), Some(32)),
            _ => V::new(BigInt::from(x.to_bits()), Some(64)),
        });
    }
    let vals = args.iter().map(|a| eval(a, env)).collect::<Result<Vec<_>>>()?;
    let n = vals.len();
    let idx = |i: usize, what: &str| index(&vals[i], pos, what);
    // Width of x's word: explicit argument, its own width, or the active width.
    let word_width = |i: usize| -> Result<u32> {
        if n > i {
            let w = idx(i, "Width")?;
            if w == 0 {
                return Err(Error::at("Width must be at least 1", pos));
            }
            Ok(w)
        } else {
            Ok(vals[0].w.unwrap_or(env.width))
        }
    };
    let x = || vals[0].v.clone();
    let word = |w: u32| truncate(&vals[0].v, w);
    let int = |v: BigInt| V::new(v, None);
    let range_mask = |hi: u32, lo: u32| -> Result<BigInt> {
        if hi < lo {
            return Err(Error::at("Write ranges high bit first", pos));
        }
        Ok(mask_bits((hi - lo + 1) as u64) << lo)
    };
    let need = |lo: usize, hi: usize| if n < lo || n > hi { Err(arity_error()) } else { Ok(()) };
    let v = match name {
        "mask" => {
            need(1, 2)?;
            if n == 1 { V::new(mask_bits(idx(0, "Width")? as u64), None) } else { int(range_mask(idx(0, "Bit")?, idx(1, "Bit")?)?) }
        }
        "bit" => {
            need(1, 1)?;
            int(BigInt::one() << idx(0, "Bit index")?)
        }
        "set" | "clr" | "flip" => {
            need(2, 3)?;
            let hi = idx(1, "Bit")?;
            let lo = if n == 3 { idx(2, "Bit")? } else { hi };
            let m = range_mask(hi, lo)?;
            V::new(
                match name {
                    "set" => x() | m,
                    "clr" => x() & !m,
                    _ => x() ^ m,
                },
                vals[0].w,
            )
        }
        "ins" => {
            need(4, 4)?;
            let (hi, lo) = (idx(1, "Bit")?, idx(2, "Bit")?);
            let m = range_mask(hi, lo)?;
            let field = BigInt::from(truncate(&vals[3].v, hi - lo + 1)) << lo;
            V::new((x() & !m) | field, vals[0].w)
        }
        "sext" | "zext" => {
            need(2, 2)?;
            let w = idx(1, "Width")?;
            if w == 0 {
                return Err(Error::at("Width must be at least 1", pos));
            }
            let t = word(w);
            int(if name == "sext" { to_signed(&t, w) } else { BigInt::from(t) })
        }
        "popcount" | "parity" => {
            need(1, 1)?;
            let ones = word(word_width(9)?).count_ones();
            int(BigInt::from(if name == "parity" { ones & 1 } else { ones }))
        }
        "clz" => {
            need(1, 2)?;
            let w = word_width(1)?;
            int(BigInt::from(w as u64 - word(w).bits()))
        }
        "ctz" => {
            need(1, 1)?;
            let w = word_width(9)?;
            int(BigInt::from(word(w).trailing_zeros().unwrap_or(w as u64)))
        }
        "msb" | "lsb" => {
            need(1, 1)?;
            let t = word(word_width(9)?);
            let r = if t.is_zero() {
                -1
            } else if name == "msb" {
                t.bits() as i64 - 1
            } else {
                t.trailing_zeros().unwrap_or(0) as i64
            };
            int(BigInt::from(r))
        }
        "clog2" | "log2" => {
            need(1, 1)?;
            let v = x();
            if v.is_negative() || (name == "log2" && v.is_zero()) {
                return Err(Error::at(format!("{name} needs a positive value"), pos));
            }
            let r = if name == "log2" {
                v.bits() - 1
            } else if v <= BigInt::one() {
                0
            } else {
                (v - 1u32).bits()
            };
            int(BigInt::from(r))
        }
        "rev" => {
            need(1, 2)?;
            let w = word_width(1)?;
            let t = word(w);
            let mut r = BigUint::zero();
            for i in 0..w as u64 {
                if t.bit(i) {
                    r.set_bit(w as u64 - 1 - i, true);
                }
            }
            V::new(BigInt::from(r), Some(w))
        }
        "bswap" => {
            need(1, 2)?;
            let w = word_width(1)?;
            if w % 8 != 0 {
                return Err(Error::at(format!("bswap needs a multiple of 8 bits, not {w}"), pos));
            }
            let mut bytes = word(w).to_bytes_le();
            bytes.resize(w as usize / 8, 0);
            V::new(BigInt::from(BigUint::from_bytes_be(&bytes)), Some(w))
        }
        "rol" | "ror" => {
            need(2, 3)?;
            let w = word_width(2)?;
            let s = (idx(1, "Rotate amount")? % w) as u64;
            let s = if name == "ror" { (w as u64 - s) % w as u64 } else { s };
            let t = BigInt::from(word(w));
            let r = ((&t << s) | (t >> (w as u64 - s))) & mask_bits(w as u64);
            V::new(r, Some(w))
        }
        "gray" => {
            need(1, 1)?;
            let w = word_width(9)?;
            let t = BigInt::from(word(w));
            V::new(&t ^ (&t >> 1u32), Some(w))
        }
        "ungray" => {
            need(1, 1)?;
            let w = word_width(9)?;
            let mut t = BigInt::from(word(w));
            let mut s = 1u64;
            while s < w as u64 {
                t = &t ^ (&t >> s);
                s <<= 1;
            }
            V::new(t, Some(w))
        }
        "alignup" | "aligndown" | "cdiv" => {
            need(2, 2)?;
            let (a, b) = (x(), vals[1].v.clone());
            if !b.is_positive() {
                return Err(Error::at("The divisor must be positive", pos));
            }
            int(match name {
                "alignup" => a.div_ceil(&b) * b,
                "aligndown" => a.div_floor(&b) * b,
                _ => a.div_ceil(&b),
            })
        }
        "abs" => {
            need(1, 1)?;
            V::new(x().abs(), vals[0].w)
        }
        "min" | "max" => {
            if n == 0 {
                return Err(arity_error());
            }
            let pick = vals.iter().map(|v| &v.v);
            int(if name == "min" { pick.min() } else { pick.max() }.cloned().unwrap_or_default())
        }
        "gcd" => {
            need(2, 2)?;
            int(x().gcd(&vals[1].v))
        }
        _ => return Err(arity_error()),
    };
    if v.v.bits() > MAX_BITS {
        return Err(Error::at("Result exceeds 2^20 bits", pos));
    }
    Ok(v)
}
