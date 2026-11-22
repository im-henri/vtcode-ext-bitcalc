//! Calculator state, input handling and actions. Host-independent so the
//! whole behavior is testable natively.
use crate::expr::{self, Env, Field, Target, Var, MAX_WIDTH};
use crate::fmt;
use crate::line::Line;
use num_bigint::{BigInt, BigUint};
use num_traits::{One, Zero};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const HISTORY_LIMIT: usize = 200;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Event {
    pub code: String,
    pub text: Option<String>,
    pub shift: bool,
    pub control: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub kind: String,
    pub button: String,
    pub click_count: u8,
    pub view_column: u16,
    pub view_row: u16,
    pub width: u16,
    pub height: u16,
    pub direction: i32,
    pub axis: String,
}

impl Event {
    pub fn key(code: &str) -> Self {
        Self { code: code.into(), ..Self::default() }
    }
    pub fn ch(c: char) -> Self {
        Self { code: "char".into(), text: Some(c.to_string()), ..Self::default() }
    }
    pub fn mouse(kind: &str, x: u16, y: u16) -> Self {
        Self { kind: kind.into(), button: "left".into(), click_count: 1, view_column: x, view_row: y, ..Self::default() }
    }
    fn ctrl(&self) -> bool {
        self.control || self.ctrl
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Input,
    Bits,
    Width,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fmt {
    Hex,
    Dec,
    Oct,
    Bin,
    Verilog,
    Float(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldOp {
    Set,
    Clear,
    Invert,
    Write,
    Extract,
    Copy,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Bit(u32),
    Nibble(u32),
    Copy(Fmt),
    WidthChip,
    SignChip,
    HelpChip,
    Preset(u32),
    Widen(u32),
    Field(FieldOp),
    History(usize),
    Var(usize),
    /// A register field label (index into `App::fields`).
    FieldName(usize),
    Input(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Info,
    Good,
    Bad,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub expr: String,
    pub value: BigUint,
    pub width: u32,
}

#[derive(Clone, Debug)]
struct Snapshot {
    value: BigUint,
    prev: BigUint,
    width: u32,
    signed: bool,
    vars: BTreeMap<String, Var>,
    fields: Vec<Field>,
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    anchor: u32,
    moved: bool,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Saved {
    pub width: u32,
    pub signed: bool,
    pub value: String,
    pub vars: Vec<(String, String, u32)>,
    pub history: Vec<(String, String, u32)>,
    pub fields: String,
}

pub struct App {
    pub width: u32,
    pub signed: bool,
    pub value: BigUint,
    /// Value before the last change; differing bits are highlighted.
    pub prev: BigUint,
    pub vars: BTreeMap<String, Var>,
    /// Register layout drawn under the bit grid.
    pub fields: Vec<Field>,
    pub history: Vec<Entry>,
    pub input: Line,
    pub width_input: Line,
    pub focus: Focus,
    pub cursor_bit: u32,
    /// Bit selection as (anchor, head); both inclusive.
    pub sel: Option<(u32, u32)>,
    pub hover: Option<Hit>,
    pub scroll: usize,
    pub reveal: bool,
    pub view_w: u16,
    pub view_h: u16,
    pub flash: Option<(String, Tone)>,
    /// Exact result of the last evaluation when it did not fit the width.
    pub overflow: Option<(BigInt, u32)>,
    /// Set after Enter fails so the error is shown prominently.
    pub failed: bool,
    pub help: bool,
    pub hits: Vec<(u16, u16, u16, Hit)>,
    history_pos: Option<usize>,
    draft: String,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    drag: Option<Drag>,
    /// Clipboard text for the host to write.
    pub clipboard_out: Option<String>,
    /// State changed in a way worth persisting.
    pub save_needed: bool,
    pub(crate) cache: Option<crate::render::Cache>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

fn fields_spec(fields: &[Field]) -> String {
    fields
        .iter()
        .map(|f| if f.hi == f.lo { format!("{}[{}]", f.name, f.hi) } else { format!("{}[{}:{}]", f.name, f.hi, f.lo) })
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_hex(s: &str) -> BigUint {
    BigUint::parse_bytes(s.as_bytes(), 16).unwrap_or_default()
}

impl App {
    pub fn new() -> Self {
        Self {
            width: 32,
            signed: false,
            value: BigUint::zero(),
            prev: BigUint::zero(),
            vars: BTreeMap::new(),
            fields: Vec::new(),
            history: Vec::new(),
            input: Line::default(),
            width_input: Line::default(),
            focus: Focus::Input,
            cursor_bit: 0,
            sel: None,
            hover: None,
            scroll: 0,
            reveal: false,
            view_w: 100,
            view_h: 32,
            flash: None,
            overflow: None,
            failed: false,
            help: false,
            hits: Vec::new(),
            history_pos: None,
            draft: String::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            drag: None,
            clipboard_out: None,
            save_needed: false,
            cache: None,
        }
    }

    // ───────────────────────── persistence ─────────────────────────

    pub fn save(&self) -> Saved {
        Saved {
            width: self.width,
            signed: self.signed,
            value: self.value.to_str_radix(16),
            vars: self.vars.iter().map(|(k, v)| (k.clone(), v.value.to_str_radix(16), v.width)).collect(),
            history: self
                .history
                .iter()
                .map(|e| (e.expr.clone(), e.value.to_str_radix(16), e.width))
                .collect(),
            fields: fields_spec(&self.fields),
        }
    }

    pub fn load(&mut self, saved: Saved) {
        let clamp = |w: u32| w.clamp(1, MAX_WIDTH);
        self.width = clamp(if saved.width == 0 { 32 } else { saved.width });
        self.signed = saved.signed;
        self.value = expr::truncate(&BigInt::from(parse_hex(&saved.value)), self.width);
        self.prev = self.value.clone();
        self.vars = saved
            .vars
            .into_iter()
            .filter(|(k, _, _)| !k.is_empty())
            .map(|(k, v, w)| {
                let w = clamp(w);
                (k, Var { value: expr::truncate(&BigInt::from(parse_hex(&v)), w), width: w })
            })
            .collect();
        self.history = saved
            .history
            .into_iter()
            .map(|(e, v, w)| Entry { expr: e, value: parse_hex(&v), width: clamp(w) })
            .collect();
        self.history.truncate(HISTORY_LIMIT);
        self.fields = expr::parse_fields(&saved.fields).unwrap_or_default();
    }

    // ─────────────────────────── helpers ───────────────────────────

    pub fn env(&self) -> Env<'_> {
        Env { width: self.width, signed: self.signed, ans: &self.value, vars: &self.vars, fields: &self.fields }
    }

    /// Field targeted by bit actions: the selection, else the bit cursor when
    /// the grid has focus.
    pub fn field(&self) -> Option<(u32, u32)> {
        match self.sel {
            Some((a, b)) => Some((a.max(b), a.min(b))),
            None if self.focus == Focus::Bits => Some((self.cursor_bit, self.cursor_bit)),
            None => None,
        }
    }

    pub fn field_value(&self, hi: u32, lo: u32) -> BigUint {
        (&self.value >> lo) & ((BigUint::one() << (hi - lo + 1)) - 1u32)
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            value: self.value.clone(),
            prev: self.prev.clone(),
            width: self.width,
            signed: self.signed,
            vars: self.vars.clone(),
            fields: self.fields.clone(),
        }
    }

    fn checkpoint(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > 500 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.save_needed = true;
    }

    fn restore(&mut self, s: Snapshot) {
        self.value = s.value;
        self.prev = s.prev;
        self.width = s.width;
        self.signed = s.signed;
        self.vars = s.vars;
        self.fields = s.fields;
        self.overflow = None;
        self.clamp_bits();
        self.save_needed = true;
    }

    fn clamp_bits(&mut self) {
        let top = self.width - 1;
        self.cursor_bit = self.cursor_bit.min(top);
        if let Some((a, b)) = self.sel {
            if a.min(b) > top {
                self.sel = None;
            } else {
                self.sel = Some((a.min(top), b.min(top)));
            }
        }
    }

    /// Replace the value (recording undo) and highlight changed bits.
    fn set_value(&mut self, value: BigUint) {
        self.checkpoint();
        self.prev = std::mem::replace(&mut self.value, value);
        self.overflow = None;
    }

    pub fn say(&mut self, text: impl Into<String>, tone: Tone) {
        self.flash = Some((text.into(), tone));
    }

    fn copy(&mut self, text: String, what: &str) {
        let shown = if text.chars().count() > 48 {
            format!("{}… ({} chars)", text.chars().take(40).collect::<String>(), text.chars().count())
        } else {
            text.clone()
        };
        self.say(format!("Copied {what} {shown}"), Tone::Good);
        self.clipboard_out = Some(text);
    }

    pub fn format(&self, f: Fmt) -> String {
        match f {
            Fmt::Hex => format!("0x{}", fmt::hex(&self.value, self.width)),
            Fmt::Dec => fmt::dec_plain(&self.value, self.width, self.signed),
            Fmt::Oct => format!("0o{}", fmt::oct(&self.value, self.width)),
            Fmt::Bin => format!("0b{}", fmt::bin(&self.value, self.width)),
            Fmt::Verilog => fmt::verilog(&self.value, self.width),
            Fmt::Float(i) => fmt::floats(&self.value, self.width).get(i).map(|f| f.1.replace('−', "-")).unwrap_or_default(),
        }
    }

    pub fn copy_format(&mut self, f: Fmt) {
        let name = match f {
            Fmt::Hex => "hex",
            Fmt::Dec => "decimal",
            Fmt::Oct => "octal",
            Fmt::Bin => "binary",
            Fmt::Verilog => "Verilog",
            Fmt::Float(_) => "float",
        };
        self.copy(self.format(f), name);
    }

    // ─────────────────────────── actions ───────────────────────────

    pub fn evaluate(&mut self) {
        let src = self.input.text();
        if src.trim().is_empty() {
            return;
        }
        if let Some(cmd) = src.trim().strip_prefix(':') {
            match self.command(cmd, true) {
                Ok(msg) => {
                    self.say(msg, Tone::Good);
                    if self.history.first().is_none_or(|e| e.expr != src.trim()) {
                        let entry = Entry { expr: src.trim().to_string(), value: self.value.clone(), width: self.width };
                        self.history.insert(0, entry);
                        self.history.truncate(HISTORY_LIMIT);
                    }
                    self.input.clear();
                    self.history_pos = None;
                    self.failed = false;
                }
                Err(e) => {
                    self.failed = true;
                    self.say(e, Tone::Bad);
                }
            }
            return;
        }
        let out = match expr::evaluate(&src, &self.env()) {
            Ok(out) => out,
            Err(e) => {
                self.failed = true;
                self.say(e.message, Tone::Bad);
                return;
            }
        };
        self.checkpoint();
        let mut ans = expr::truncate(&out.value, self.width);
        match &out.assign {
            Some(Target::Var(name)) => {
                let width = match (out.field, self.vars.get(name)) {
                    (Some(_), Some(v)) => v.width,
                    _ => self.width,
                };
                let value = expr::truncate(&out.value, width);
                ans = expr::truncate(&BigInt::from(value.clone()), self.width);
                self.vars.insert(name.clone(), Var { value, width });
            }
            Some(Target::Ans) | None => {}
        }
        self.overflow = None;
        if out.field.is_none()
            && !expr::fits(&out.value, self.width, false)
            && !expr::fits(&out.value, self.width, true)
        {
            let need = expr::bits_needed(&out.value, false).min(MAX_WIDTH as u64) as u32;
            self.overflow = Some((out.value.clone(), need));
        }
        self.prev = std::mem::replace(&mut self.value, ans.clone());
        if self.history.first().is_none_or(|e| e.expr != src || e.value != ans || e.width != self.width) {
            self.history.insert(0, Entry { expr: src.trim().to_string(), value: ans, width: self.width });
            self.history.truncate(HISTORY_LIMIT);
        }
        if let Some((hi, lo)) = out.field
            && out.assign == Some(Target::Ans)
        {
            self.sel = Some((hi, lo));
        }
        self.flash = out.note.map(|n| (n, Tone::Info));
        self.input.clear();
        self.history_pos = None;
        self.failed = false;
        self.clamp_bits();
    }

    /// `:command` input. With `apply` false only validates and describes.
    pub fn command(&mut self, cmd: &str, apply: bool) -> Result<String, String> {
        let (name, arg) = cmd.trim().split_once(char::is_whitespace).unwrap_or((cmd.trim(), ""));
        let arg = arg.trim();
        match name {
            "fields" | "f" => {
                let fields = expr::parse_fields(arg)?;
                let msg = match fields.len() {
                    0 => "Fields cleared".to_string(),
                    1 => "1 field: use its name to read or write it".to_string(),
                    n => format!("{n} fields: use their names to read or write them"),
                };
                if !apply && let Some(f) = fields.iter().find(|f| f.hi >= self.width) {
                    return Ok(format!("{msg} (note: {} is beyond the {}-bit width)", f.name, self.width));
                }
                if apply {
                    self.checkpoint();
                    self.fields = fields;
                }
                Ok(msg)
            }
            "w" | "width" => {
                let w: u32 = arg.parse().map_err(|_| format!("Usage: :width N (1 to {MAX_WIDTH})"))?;
                if !(1..=MAX_WIDTH).contains(&w) {
                    return Err(format!("Width must be 1 to {MAX_WIDTH}"));
                }
                if apply {
                    self.set_width(w);
                }
                Ok(format!("Width {w} bits"))
            }
            "signed" | "unsigned" | "s" | "u" => {
                let want = name.starts_with('s');
                if apply && self.signed != want {
                    self.toggle_signed();
                }
                Ok(if want { "Signed interpretation" } else { "Unsigned interpretation" }.into())
            }
            "del" | "rm" => {
                let names: Vec<&str> = arg.split([' ', ',']).filter(|s| !s.is_empty()).collect();
                if names.is_empty() {
                    return Err("Usage: :del name [name …]".into());
                }
                if let Some(n) = names.iter().find(|n| !self.vars.contains_key(**n)) {
                    return Err(format!("No stored value named '{n}'"));
                }
                if apply {
                    self.checkpoint();
                    for n in &names {
                        self.vars.remove(*n);
                    }
                }
                Ok(format!("Deleted {}", names.join(", ")))
            }
            "clear" => {
                if apply {
                    self.history.clear();
                    self.save_needed = true;
                }
                Ok("History cleared".into())
            }
            "" => Err("Commands: :fields, :width, :signed, :unsigned, :del, :clear".into()),
            other => Err(format!("Unknown command :{other} (try :fields, :width, :signed, :del, :clear)")),
        }
    }

    pub fn set_width(&mut self, width: u32) {
        let width = width.clamp(1, MAX_WIDTH);
        if width == self.width {
            return;
        }
        self.checkpoint();
        let current = if self.signed {
            expr::to_signed(&self.value, self.width)
        } else {
            BigInt::from(self.value.clone())
        };
        self.width = width;
        self.value = expr::truncate(&current, width);
        self.prev = self.value.clone();
        self.overflow = None;
        self.clamp_bits();
        self.cache = None;
    }

    pub fn widen(&mut self, width: u32) {
        let Some((exact, _)) = self.overflow.clone() else { return };
        self.set_width(width);
        self.value = expr::truncate(&exact, self.width);
        self.prev = self.value.clone();
        if let Some(entry) = self.history.first_mut() {
            entry.value = self.value.clone();
            entry.width = self.width;
        }
        self.say(format!("Width is now {} bits", self.width), Tone::Good);
    }

    pub fn toggle_signed(&mut self) {
        self.checkpoint();
        self.signed = !self.signed;
        self.cache = None;
    }

    pub fn toggle_bit(&mut self, bit: u32) {
        if bit >= self.width {
            return;
        }
        let mut v = self.value.clone();
        v.set_bit(bit as u64, !v.bit(bit as u64));
        self.set_value(v);
    }

    pub fn field_op(&mut self, op: FieldOp) {
        let Some((hi, lo)) = self.field() else { return };
        let n = hi - lo + 1;
        let mask = ((BigUint::one() << n) - 1u32) << lo;
        let all = (BigUint::one() << self.width) - 1u32;
        match op {
            FieldOp::Set => self.set_value(&self.value | &mask),
            FieldOp::Clear => self.set_value(&self.value & (&all ^ &mask)),
            FieldOp::Invert => self.set_value(&self.value ^ &mask),
            FieldOp::Write => {
                self.input.set(&if hi == lo { format!("[{hi}] = ") } else { format!("[{hi}:{lo}] = ") });
                self.focus = Focus::Input;
            }
            FieldOp::Extract => {
                let v = self.field_value(hi, lo);
                self.set_value(v);
                self.sel = None;
                self.say(format!("Extracted [{hi}:{lo}] into bits [{}:0]", n - 1), Tone::Info);
            }
            FieldOp::Copy => {
                let v = self.field_value(hi, lo);
                self.copy(format!("0x{}", fmt::hex(&v, n)), &format!("[{hi}:{lo}]"));
            }
            FieldOp::Close => {
                self.sel = None;
                if self.focus == Focus::Bits {
                    self.focus = Focus::Input;
                }
            }
        }
    }

    pub fn undo(&mut self) {
        match self.undo.pop() {
            Some(s) => {
                let now = self.snapshot();
                self.redo.push(now);
                self.restore(s);
                self.say("Undone", Tone::Info);
            }
            None => self.say("Nothing to undo", Tone::Info),
        }
    }

    pub fn redo(&mut self) {
        match self.redo.pop() {
            Some(s) => {
                let now = self.snapshot();
                self.undo.push(now);
                self.restore(s);
                self.say("Redone", Tone::Info);
            }
            None => self.say("Nothing to redo", Tone::Info),
        }
    }

    pub fn open_width_editor(&mut self) {
        self.focus = Focus::Width;
        self.width_input.reset(&self.width.to_string());
        self.width_input.select_all();
    }

    fn apply_width_input(&mut self) {
        match self.width_input.text().trim().parse::<u32>() {
            Ok(w) if (1..=MAX_WIDTH).contains(&w) => {
                self.set_width(w);
                self.focus = Focus::Input;
            }
            _ => self.say(format!("Width must be 1 to {MAX_WIDTH}"), Tone::Bad),
        }
    }

    fn recall(&mut self, older: bool) {
        if self.history.is_empty() {
            return;
        }
        let next = match (self.history_pos, older) {
            (None, true) => {
                self.draft = self.input.text();
                Some(0)
            }
            (None, false) => None,
            (Some(i), true) => Some((i + 1).min(self.history.len() - 1)),
            (Some(0), false) => None,
            (Some(i), false) => Some(i - 1),
        };
        self.history_pos = next;
        let text = match next {
            Some(i) => self.history[i].expr.clone(),
            None => self.draft.clone(),
        };
        self.input.reset(&text);
    }

    pub fn load_history(&mut self, i: usize) {
        let Some(e) = self.history.get(i).cloned() else { return };
        let v = expr::truncate(&BigInt::from(e.value), self.width);
        self.set_value(v);
        self.say(format!("Loaded {}", e.expr), Tone::Info);
    }

    pub fn paste(&mut self, text: &str) {
        let line = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ");
        match self.focus {
            Focus::Width => self.width_input.insert(line.trim()),
            _ => {
                self.focus = Focus::Input;
                self.input.insert(&line);
            }
        }
    }

    pub fn copy_command(&mut self) {
        match self.focus {
            Focus::Input if self.input.selection().is_some() => {
                let text = self.input.selected_text().unwrap_or_default();
                self.clipboard_out = Some(text);
            }
            Focus::Bits if self.field().is_some() => self.field_op(FieldOp::Copy),
            _ => self.copy_format(Fmt::Hex),
        }
    }

    // ──────────────────────────── input ────────────────────────────

    /// Handle one host event. Returns true when the view should repaint.
    pub fn event(&mut self, ev: &Event) -> bool {
        match ev.kind.as_str() {
            "resize" => {
                if ev.width > 0 && ev.height > 0 {
                    self.view_w = ev.width;
                    self.view_h = ev.height;
                }
                return true;
            }
            "move" | "leave" => {
                let hit = if ev.kind == "move" { self.hit_at(ev.view_column, ev.view_row) } else { None };
                let changed = hit != self.hover;
                self.hover = hit;
                return changed;
            }
            "down" | "up" | "drag" | "cancel" => {
                self.mouse(ev);
                return true;
            }
            _ => {}
        }
        if ev.direction != 0 && ev.code.is_empty() {
            self.scroll_event(ev);
            return true;
        }
        if ev.code.is_empty() {
            return false;
        }
        if !matches!(ev.code.as_str(), "command") || ev.text.as_deref() != Some("copy") {
            self.flash = None;
        }
        self.key(ev);
        true
    }

    pub fn hit_at(&self, x: u16, y: u16) -> Option<Hit> {
        self.hits.iter().find(|(row, x0, x1, _)| *row == y && x >= *x0 && x < *x1).map(|h| h.3)
    }

    fn scroll_event(&mut self, ev: &Event) {
        if self.hit_at(ev.view_column, ev.view_row) == Some(Hit::WidthChip) {
            let step = if ev.shift { 8 } else { 1 };
            let w = if ev.direction < 0 { self.width.saturating_add(step) } else { self.width.saturating_sub(step).max(1) };
            self.set_width(w);
            return;
        }
        let step = if ev.shift { 10 } else { 3 };
        self.scroll = if ev.direction < 0 { self.scroll.saturating_sub(step) } else { self.scroll + step };
    }

    fn mouse(&mut self, ev: &Event) {
        let hit = self.hit_at(ev.view_column, ev.view_row);
        match ev.kind.as_str() {
            "down" => {
                self.flash = None;
                self.drag = None;
                match hit {
                    Some(Hit::Bit(b)) => {
                        if ev.shift {
                            let anchor = self.sel.map_or(self.cursor_bit, |s| s.0);
                            self.sel = Some((anchor, b));
                        } else {
                            self.drag = Some(Drag { anchor: b, moved: false });
                        }
                        self.cursor_bit = b;
                        self.focus = Focus::Bits;
                    }
                    Some(Hit::Nibble(lo)) => {
                        let hi = (lo + 3).min(self.width - 1);
                        self.sel = Some((hi, lo));
                        self.cursor_bit = lo;
                        self.focus = Focus::Bits;
                    }
                    Some(Hit::Copy(f)) => self.copy_format(f),
                    Some(Hit::WidthChip) => {
                        if self.focus == Focus::Width {
                            self.focus = Focus::Input;
                        } else {
                            self.open_width_editor();
                        }
                    }
                    Some(Hit::SignChip) => self.toggle_signed(),
                    Some(Hit::HelpChip) => self.help = !self.help,
                    Some(Hit::Preset(w)) => {
                        self.set_width(w);
                        self.focus = Focus::Input;
                    }
                    Some(Hit::Widen(w)) => self.widen(w),
                    Some(Hit::Field(op)) => self.field_op(op),
                    Some(Hit::History(i)) => {
                        if ev.click_count >= 2 || ev.shift {
                            if let Some(e) = self.history.get(i) {
                                let text = e.expr.clone();
                                self.input.set(&text);
                                self.focus = Focus::Input;
                            }
                        } else {
                            self.load_history(i);
                        }
                    }
                    Some(Hit::FieldName(i)) => {
                        if let Some(f) = self.fields.get(i).cloned() {
                            if f.lo < self.width {
                                self.sel = Some((f.hi.min(self.width - 1), f.lo));
                                self.cursor_bit = f.lo;
                                self.focus = Focus::Bits;
                            }
                            if ev.click_count >= 2 {
                                self.input.set(&format!("{} = ", f.name));
                                self.focus = Focus::Input;
                            }
                        }
                    }
                    Some(Hit::Var(i)) => {
                        if let Some(name) = self.vars.keys().nth(i).cloned() {
                            self.focus = Focus::Input;
                            self.input.insert(&name);
                        }
                    }
                    Some(Hit::Input(i)) => {
                        if self.focus == Focus::Width {
                            self.width_input.move_to(i, ev.shift);
                        } else {
                            self.focus = Focus::Input;
                            if ev.click_count >= 3 {
                                self.input.select_all();
                            } else if ev.click_count == 2 {
                                self.input.select_word(i);
                            } else {
                                self.input.move_to(i, ev.shift);
                            }
                        }
                    }
                    None => {}
                }
            }
            "drag" => match (self.drag, hit) {
                (Some(d), Some(Hit::Bit(b))) => {
                    if b != d.anchor || d.moved {
                        self.drag = Some(Drag { moved: true, ..d });
                        self.sel = Some((d.anchor, b));
                        self.cursor_bit = b;
                    }
                }
                (None, Some(Hit::Input(i))) if self.focus == Focus::Input => self.input.move_to(i, true),
                _ => {}
            },
            "up" => {
                if let Some(d) = self.drag.take()
                    && !d.moved
                    && hit == Some(Hit::Bit(d.anchor))
                {
                    self.sel = None;
                    self.toggle_bit(d.anchor);
                }
            }
            _ => self.drag = None,
        }
    }

    fn key(&mut self, ev: &Event) {
        let text = ev.text.as_deref().unwrap_or("");
        let ctrl = ev.ctrl();
        if ev.code == "command" {
            match text {
                "copy" => self.copy_command(),
                "cut" if self.focus == Focus::Input => {
                    if let Some(t) = self.input.selected_text() {
                        self.clipboard_out = Some(t);
                        self.input.backspace(false);
                    }
                }
                "undo" => {
                    if !(self.focus == Focus::Input && self.input.undo()) {
                        self.undo();
                    }
                }
                "redo" => {
                    if !(self.focus == Focus::Input && self.input.redo()) {
                        self.redo();
                    }
                }
                "select_all" => {
                    if self.focus == Focus::Bits {
                        self.sel = Some((self.width - 1, 0));
                    } else {
                        self.focus = Focus::Input;
                        self.input.select_all();
                    }
                }
                "find" | "goto_line" => self.focus = Focus::Input,
                _ => {}
            }
            return;
        }
        if ev.code == "paste" {
            self.paste(text);
            return;
        }
        // Global shortcuts
        if ev.alt && ev.code == "char" {
            match text.to_ascii_lowercase().as_str() {
                "w" => return self.open_width_editor(),
                "s" => return self.toggle_signed(),
                "h" => {
                    self.help = !self.help;
                    return;
                }
                _ => {}
            }
        }
        if ctrl && ev.code == "char" {
            match text.to_ascii_lowercase().as_str() {
                "z" if ev.shift => return self.key(&Event { code: "command".into(), text: Some("redo".into()), ..Event::default() }),
                "z" => return self.key(&Event { code: "command".into(), text: Some("undo".into()), ..Event::default() }),
                "y" => return self.key(&Event { code: "command".into(), text: Some("redo".into()), ..Event::default() }),
                "c" => return self.copy_command(),
                "a" => return self.key(&Event { code: "command".into(), text: Some("select_all".into()), ..Event::default() }),
                _ => {}
            }
        }
        match ev.code.as_str() {
            "pageUp" => {
                self.scroll = self.scroll.saturating_sub(self.view_h as usize / 2);
                return;
            }
            "pageDown" => {
                self.scroll += self.view_h as usize / 2;
                return;
            }
            _ => {}
        }
        if self.help && ev.code == "escape" {
            self.help = false;
            return;
        }
        match self.focus {
            Focus::Input => self.input_key(ev, text, ctrl),
            Focus::Bits => self.bits_key(ev, text),
            Focus::Width => self.width_key(ev, text, ctrl),
        }
    }

    fn input_key(&mut self, ev: &Event, text: &str, ctrl: bool) {
        let line = &mut self.input;
        match ev.code.as_str() {
            "char" if !ctrl && !ev.alt => {
                line.insert(text);
                self.history_pos = None;
                self.failed = false;
            }
            "backspace" => line.backspace(ctrl),
            "delete" => line.delete(ctrl),
            "left" => line.left(ev.shift, ctrl),
            "right" => line.right(ev.shift, ctrl),
            "home" => line.move_to(0, ev.shift),
            "end" => line.move_to(usize::MAX, ev.shift),
            "enter" => self.evaluate(),
            "up" => self.recall(true),
            "down" => self.recall(false),
            "escape" => {
                if line.selection().is_some() {
                    line.anchor = None;
                } else if !line.is_empty() {
                    line.clear();
                    self.history_pos = None;
                    self.failed = false;
                } else if self.sel.is_some() {
                    self.sel = None;
                } else {
                    self.help = false;
                }
            }
            "tab" | "backTab" => {
                self.focus = Focus::Bits;
                self.reveal = true;
            }
            _ => {}
        }
    }

    fn row_bits(&self) -> u32 {
        crate::render::row_bits(self.view_w, self.width)
    }

    fn move_bit(&mut self, to: i64, select: bool) {
        let to = to.clamp(0, self.width as i64 - 1) as u32;
        if select {
            let anchor = self.sel.map_or(self.cursor_bit, |s| s.0);
            self.sel = Some((anchor, to));
        } else {
            self.sel = None;
        }
        self.cursor_bit = to;
        self.reveal = true;
    }

    fn bits_key(&mut self, ev: &Event, text: &str) {
        let c = self.cursor_bit as i64;
        let row = self.row_bits() as i64;
        let step = if ev.control || ev.ctrl { 4 } else { 1 };
        match ev.code.as_str() {
            "left" => self.move_bit(c + step, ev.shift),
            "right" => self.move_bit(c - step, ev.shift),
            "up" => self.move_bit(c + row, ev.shift),
            "down" => self.move_bit(c - row, ev.shift),
            "home" => self.move_bit(self.width as i64 - 1, ev.shift),
            "end" => self.move_bit(0, ev.shift),
            "enter" => self.field_op(FieldOp::Write),
            "escape" => {
                if self.sel.is_some() {
                    self.sel = None;
                } else {
                    self.focus = Focus::Input;
                }
            }
            "tab" | "backTab" => self.focus = Focus::Input,
            "backspace" | "delete" => self.field_op(FieldOp::Clear),
            "char" => match text {
                " " => {
                    if self.sel.is_some() {
                        self.field_op(FieldOp::Invert);
                    } else {
                        self.toggle_bit(self.cursor_bit);
                    }
                }
                "1" => self.field_op(FieldOp::Set),
                "0" => self.field_op(FieldOp::Clear),
                "~" | "!" => self.field_op(FieldOp::Invert),
                "=" => self.field_op(FieldOp::Write),
                "x" | "X" if !ev.shift || text == "X" => self.field_op(FieldOp::Extract),
                "c" | "y" => self.field_op(FieldOp::Copy),
                _ => {
                    // Anything else starts an expression.
                    self.focus = Focus::Input;
                    self.input.insert(text);
                }
            },
            _ => {}
        }
    }

    fn width_key(&mut self, ev: &Event, text: &str, ctrl: bool) {
        let step = if ev.shift { 8 } else { 1 };
        let current = self.width_input.text().trim().parse::<u32>().unwrap_or(self.width);
        match ev.code.as_str() {
            "char" if !ctrl && text.chars().all(|c| c.is_ascii_digit()) => self.width_input.insert(text),
            "backspace" => self.width_input.backspace(ctrl),
            "delete" => self.width_input.delete(ctrl),
            "left" => self.width_input.left(ev.shift, ctrl),
            "right" => self.width_input.right(ev.shift, ctrl),
            "home" => self.width_input.move_to(0, ev.shift),
            "end" => self.width_input.move_to(usize::MAX, ev.shift),
            "up" => {
                self.width_input.reset(&current.saturating_add(step).min(MAX_WIDTH).to_string());
                self.apply_preview_width();
            }
            "down" => {
                self.width_input.reset(&current.saturating_sub(step).max(1).to_string());
                self.apply_preview_width();
            }
            "enter" => self.apply_width_input(),
            "escape" | "tab" => self.focus = Focus::Input,
            _ => {}
        }
    }

    /// Arrow keys in the width editor apply immediately.
    fn apply_preview_width(&mut self) {
        if let Ok(w) = self.width_input.text().parse::<u32>() {
            self.set_width(w);
        }
    }
}
