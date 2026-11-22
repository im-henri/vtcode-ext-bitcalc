//! Frame layout. The value is the centerpiece: base readouts, then a bit
//! grid whose columns carry their index above and their hex digit below.
use crate::app::{App, FieldOp, Fmt, Focus, Hit, Tone};
use crate::expr::{self, FUNCTIONS};
use crate::fmt;
use num_bigint::{BigInt, BigUint};
use vtcode_extension_api::editor::{Frame, Span};

/// Left margin where values and the bit grid start (after a 3-letter label).
pub const LEFT: usize = 7;
const PRESETS: [u32; 10] = [1, 4, 8, 16, 32, 64, 128, 256, 512, 1024];
const MAX_VALUE_LINES: usize = 3;

pub struct Cache {
    key: (BigUint, u32, bool),
    hex: String,
    dec: String,
    alt: Option<(&'static str, String)>,
    oct: String,
    floats: Vec<(&'static str, String, String)>,
}

impl Cache {
    fn new(app: &App) -> Self {
        let (v, w) = (&app.value, app.width);
        let negative = v.bit(w as u64 - 1);
        let signed = fmt::dec_grouped(&expr::to_signed(v, w));
        let unsigned = fmt::dec_grouped(&BigInt::from(v.clone()));
        let (dec, alt) = match (app.signed, negative) {
            (true, true) => (signed, Some(("unsigned", unsigned))),
            (false, true) => (unsigned, Some(("signed", signed))),
            _ => (unsigned, None),
        };
        Self {
            key: (v.clone(), w, app.signed),
            hex: fmt::hex_grouped(v, w),
            dec,
            alt,
            oct: format!("0o{}", fmt::oct(v, w)),
            floats: fmt::floats(v, w),
        }
    }
}

// ───────────────────────────── geometry ─────────────────────────────

fn span_of(r: u32) -> usize {
    (r + r / 4 - 1 + (r / 8).saturating_sub(1)) as usize
}

/// Bits per grid row for a view width: 64, 32, 16 or 8 (never wider than
/// the word, rounded up to a byte).
pub fn row_bits(view_w: u16, width: u32) -> u32 {
    let avail = (view_w as usize).saturating_sub(LEFT + 2);
    let r = [64, 32, 16, 8].into_iter().find(|r| span_of(*r) <= avail).unwrap_or(8);
    r.min(width.div_ceil(8) * 8)
}

/// Column of bit `j` (0 = rightmost) in a row of `r` bits.
fn bit_x(r: u32, j: u32) -> usize {
    LEFT + span_of(r) - 1 - (j + j / 4 + j / 8) as usize
}

// ───────────────────────────── builder ─────────────────────────────

#[derive(Default)]
struct Out {
    spans: Vec<Span>,
    x: usize,
    hits: Vec<(u16, u16, Hit)>,
}

impl Out {
    fn styled(&mut self, text: &str, role: &str, bg: &str, bold: bool, underline: bool) -> &mut Self {
        if text.is_empty() {
            return self;
        }
        let n = text.chars().count();
        if let Some(last) = self.spans.last_mut()
            && last.role == role
            && last.background == bg
            && last.bold == bold
            && last.underline == underline
        {
            last.text.push_str(text);
        } else {
            self.spans.push(Span {
                text: text.into(),
                role: role.into(),
                background: bg.into(),
                bold,
                underline,
            });
        }
        self.x += n;
        self
    }
    fn t(&mut self, text: &str, role: &str) -> &mut Self {
        self.styled(text, role, "", false, false)
    }
    fn b(&mut self, text: &str, role: &str) -> &mut Self {
        self.styled(text, role, "", true, false)
    }
    fn on(&mut self, text: &str, role: &str, bg: &str) -> &mut Self {
        self.styled(text, role, bg, false, false)
    }
    fn hit(&mut self, text: &str, role: &str, bg: &str, hit: Hit, hover: Option<Hit>) -> &mut Self {
        let x0 = self.x;
        let hovered = hover == Some(hit);
        self.styled(text, role, bg, false, hovered);
        self.hits.push((x0 as u16, self.x as u16, hit));
        self
    }
    fn pad(&mut self, to: usize, bg: &str) -> &mut Self {
        if to > self.x {
            let s = " ".repeat(to - self.x);
            self.on(&s, "text", bg);
        }
        self
    }
}

/// Drop whatever extends past the view width.
fn clip(spans: &mut Vec<Span>, width: usize) {
    let mut x = 0;
    let mut keep = spans.len();
    for (i, s) in spans.iter_mut().enumerate() {
        let n = s.text.chars().count();
        if x + n > width {
            s.text = s.text.chars().take(width - x).collect();
            keep = if s.text.is_empty() { i } else { i + 1 };
            break;
        }
        x += n;
    }
    spans.truncate(keep);
}

enum Row {
    Out(Out),
    Ruler(u32),
    Bits(u32),
    Hex(u32),
    Fields(u32),
}

fn blank() -> Row {
    Row::Out(Out::default())
}

/// Split long text at separators so wrapped lines break between groups.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let width = width.max(8);
    let mut out = Vec::new();
    let mut start = 0;
    while chars.len() - start > width {
        let window = &chars[start..start + width];
        let cut = window
            .iter()
            .rposition(|c| matches!(c, '_' | ',' | ' '))
            .filter(|p| *p > width / 2)
            .map_or(width, |p| p + 1);
        out.push(chars[start..start + cut].iter().collect());
        start += cut;
    }
    out.push(chars[start..].iter().collect());
    out
}

// ───────────────────────────── render ─────────────────────────────

fn chart(i: usize) -> &'static str {
    ["chart2", "chart1", "chart3", "chart4", "chart5", "chart6", "chart7", "chart8"][i % 8]
}

impl App {
    /// Fields overlapping grid row `r`, clipped to it: (index, hi, lo).
    fn row_fields(&self, r: u32) -> impl Iterator<Item = (usize, u32, u32)> + '_ {
        let rb = row_bits(self.view_w, self.width);
        let lo = self.row_lo(r);
        let hi = (lo + rb).min(self.width) - 1;
        self.fields
            .iter()
            .enumerate()
            .filter(move |(_, f)| f.lo <= hi && f.hi >= lo)
            .map(move |(i, f)| (i, f.hi.min(hi), f.lo.max(lo)))
    }

    /// Brackets under the hex digits naming each field: `└─ rd ─┘`.
    fn field_row(&self, r: u32) -> Out {
        let rb = row_bits(self.view_w, self.width);
        let lo = self.row_lo(r);
        let mut o = Out::default();
        let mut parts: Vec<(usize, u32, u32)> = self.row_fields(r).collect();
        parts.sort_by_key(|p| std::cmp::Reverse(p.1));
        for (i, fhi, flo) in parts {
            let f = &self.fields[i];
            let x0 = bit_x(rb, fhi - lo);
            let x1 = bit_x(rb, flo - lo);
            let span = x1 - x0 + 1;
            let name = &f.name;
            let text: String = if span == 1 {
                name.chars().next().unwrap_or('|').to_string()
            } else if span == 2 {
                "└┘".into()
            } else {
                let inner = span - 2;
                let label: String = if name.chars().count() + 2 <= inner {
                    format!(" {name} ")
                } else {
                    name.chars().take(inner).collect()
                };
                let n = label.chars().count();
                let left = (inner - n) / 2;
                format!("└{}{}{}┘", "─".repeat(left), label, "─".repeat(inner - n - left))
            };
            o.pad(x0, "");
            let hovered = self.hover == Some(Hit::FieldName(i));
            let start = o.x;
            o.styled(&text, chart(i), "", hovered, hovered);
            o.hits.push((start as u16, o.x as u16, Hit::FieldName(i)));
        }
        o
    }

    pub fn render(&mut self) -> Frame {
        let w = self.view_w as usize;
        let h = self.view_h as usize;
        if self.cache.as_ref().is_none_or(|c| c.key != (self.value.clone(), self.width, self.signed)) {
            self.cache = Some(Cache::new(self));
        }
        self.hits.clear();
        let mut lines: Vec<Vec<Span>> = Vec::with_capacity(h);
        let mut cursor = None;

        // Header
        let mut head = Out::default();
        self.header(&mut head, w);
        self.push_line(&mut lines, head);
        lines.push(Vec::new());

        // Input + preview
        let mut input = Out::default();
        cursor = self.input_row(&mut input, w, lines.len()).or(cursor);
        self.push_line(&mut lines, input);
        let mut preview = Out::default();
        self.preview_row(&mut preview, w);
        self.push_line(&mut lines, preview);

        // Body, scrolled between the fixed header rows and the footer.
        let top = lines.len();
        let body_h = h.saturating_sub(top + 1);
        let rows = if self.help { self.help_rows(w) } else { self.body_rows(w) };
        if self.reveal {
            if let Some(i) = rows.iter().position(|r| matches!(r, Row::Bits(b) if self.row_contains(*b, self.cursor_bit))) {
                if i < self.scroll + 1 {
                    self.scroll = i.saturating_sub(1);
                } else if i + 2 > self.scroll + body_h {
                    self.scroll = (i + 2).saturating_sub(body_h);
                }
            }
            self.reveal = false;
        }
        self.scroll = self.scroll.min(rows.len().saturating_sub(body_h));
        let more_below = rows.len() > self.scroll + body_h;
        for row in rows.into_iter().skip(self.scroll).take(body_h) {
            let out = match row {
                Row::Out(o) => o,
                Row::Ruler(r) => self.ruler(r),
                Row::Bits(r) => self.bits(r),
                Row::Hex(r) => self.hex_row(r),
                Row::Fields(r) => self.field_row(r),
            };
            self.push_line(&mut lines, out);
        }
        while lines.len() < h.saturating_sub(1) {
            lines.push(Vec::new());
        }
        let mut foot = Out::default();
        self.footer(&mut foot, w, more_below);
        if h > top {
            self.push_line(&mut lines, foot);
        }
        lines.truncate(h);
        Frame {
            width: self.view_w,
            height: self.view_h,
            lines,
            cursor,
            document: None,
            title: Some("Bitcalc".into()),
            pointer_buttons: Vec::new(),
            key_capture: ["Alt+W", "Alt+S", "Alt+H", "Tab", "Shift+Tab", "Ctrl+Z", "Ctrl+Y", "Ctrl+A", "Ctrl+C"]
                .map(String::from)
                .to_vec(),
        }
    }

    fn push_line(&mut self, lines: &mut Vec<Vec<Span>>, mut out: Out) {
        let y = lines.len() as u16;
        clip(&mut out.spans, self.view_w as usize);
        self.hits.extend(out.hits.into_iter().map(|(x0, x1, hit)| (y, x0, x1, hit)));
        lines.push(out.spans);
    }

    fn row_count(&self) -> u32 {
        self.width.div_ceil(row_bits(self.view_w, self.width))
    }

    /// Low bit of grid row `r` (row 0 is the top, most significant).
    fn row_lo(&self, r: u32) -> u32 {
        (self.row_count() - 1 - r) * row_bits(self.view_w, self.width)
    }

    fn row_contains(&self, r: u32, bit: u32) -> bool {
        let lo = self.row_lo(r);
        bit >= lo && bit < lo + row_bits(self.view_w, self.width)
    }

    fn header(&self, o: &mut Out, w: usize) {
        let hv = self.hover;
        o.on(" ", "text", "header").styled("Bitcalc", "accent", "header", true, false).on("   ", "text", "header");
        let width_label = format!(" {} bits ▾ ", self.width);
        let wrole = if self.focus == Focus::Width { "cursor" } else { "text" };
        let wbg = if self.focus == Focus::Width { "cursor" } else { "panel" };
        o.hit(&width_label, wrole, wbg, Hit::WidthChip, hv).on(" ", "text", "header");
        let sign = if self.signed { " signed " } else { " unsigned " };
        o.hit(sign, "text", "panel", Hit::SignChip, hv);
        let help = if self.help { " close help " } else { " help " };
        let right = w.saturating_sub(help.len() + 1);
        o.pad(right, "header").hit(help, "muted", "header", Hit::HelpChip, hv).pad(w, "header");
    }

    fn input_row(&self, o: &mut Out, w: usize, y: usize) -> Option<(u16, u16)> {
        if self.focus == Focus::Width {
            o.on("  width ", "accent", "panel");
            let x0 = o.x;
            let text = self.width_input.text();
            let sel = self.width_input.selection();
            for (i, c) in text.chars().enumerate() {
                let selected = sel.is_some_and(|(a, b)| i >= a && i < b);
                o.on(&c.to_string(), "text", if selected { "selection" } else { "panel" });
            }
            let cur = (x0 + self.width_input.cursor) as u16;
            o.pad(x0 + 6, "panel").on("bits    ", "muted", "panel");
            for p in PRESETS {
                let label = format!(" {p} ");
                let (role, bg) = if p == self.width { ("accent", "selection") } else { ("text", "panel") };
                if o.x + label.len() < w {
                    o.hit(&label, role, bg, Hit::Preset(p), self.hover);
                }
            }
            o.pad(w, "panel");
            return Some((cur, y as u16));
        }
        let focused = self.focus == Focus::Input;
        o.on("  ", "text", "panel").on("› ", if focused { "accent" } else { "muted" }, "panel");
        let x0 = o.x;
        let avail = w.saturating_sub(x0 + 1).max(1);
        let chars = self.input.chars();
        if chars.is_empty() {
            let hint = if focused {
                "type an expression · 0xff << 4 · x[7:0] · {8'hA5, 4'b1010} · name = …"
            } else {
                ""
            };
            let hint: String = hint.chars().take(avail).collect();
            o.on(&hint, "muted", "panel");
            o.hits.push((x0 as u16, w as u16, Hit::Input(0)));
            o.pad(w, "panel");
            return focused.then_some((x0 as u16, y as u16));
        }
        let offset = (self.input.cursor + 1).saturating_sub(avail);
        let sel = self.input.selection();
        let err_pos = if self.failed {
            expr::evaluate(&self.input.text(), &self.env()).err().and_then(|e| e.pos)
        } else {
            None
        };
        let err_char = err_pos.map(|p| self.input.text()[..p.min(self.input.text().len())].chars().count());
        for (i, c) in chars.iter().enumerate().skip(offset).take(avail) {
            let selected = sel.is_some_and(|(a, b)| i >= a && i < b);
            let bg = if selected { "selection" } else { "panel" };
            let is_err = err_char == Some(i);
            o.styled(&c.to_string(), if is_err { "error" } else { "text" }, bg, false, is_err);
            o.hits.push(((o.x - 1) as u16, o.x as u16, Hit::Input(i)));
        }
        o.hits.push((o.x as u16, w as u16, Hit::Input(chars.len())));
        o.pad(w, "panel");
        focused.then_some(((x0 + self.input.cursor - offset) as u16, y as u16))
    }

    fn preview_row(&self, o: &mut Out, w: usize) {
        let text = self.input.text();
        if text.trim().is_empty() || self.focus == Focus::Width {
            return;
        }
        o.t("    ", "text");
        if let Some(cmd) = text.trim().strip_prefix(':') {
            let mut probe = App::new();
            probe.width = self.width;
            probe.signed = self.signed;
            probe.vars = self.vars.clone();
            match probe.command(cmd, false) {
                Ok(msg) => o.t(&msg, "muted"),
                Err(e) => o.t(&e, if self.failed { "error" } else { "muted" }),
            };
            return;
        }
        match expr::evaluate(&text, &self.env()) {
            Ok(out) => {
                let (name, width) = match &out.assign {
                    Some(expr::Target::Var(n)) => (
                        format!("{n} = "),
                        if out.field.is_some() { self.vars.get(n).map_or(self.width, |v| v.width) } else { self.width },
                    ),
                    _ => ("= ".to_string(), self.width),
                };
                let word = expr::truncate(&out.value, width);
                o.t(&name, "muted");
                let hex = fmt::hex_grouped(&word, width);
                let dec = fmt::dec_grouped(&if self.signed { expr::to_signed(&word, width) } else { BigInt::from(word.clone()) });
                let room = w.saturating_sub(o.x + 2);
                if hex.len() + dec.len() + 5 <= room {
                    o.t(&hex, "success").t("   ", "text").t(&dec, "muted");
                } else {
                    let clipped: String = hex.chars().take(room.saturating_sub(1)).collect();
                    o.t(&clipped, "success");
                    if clipped.len() < hex.len() {
                        o.t("…", "muted");
                    }
                }
                if out.field.is_none()
                    && !expr::fits(&out.value, self.width, false)
                    && !expr::fits(&out.value, self.width, true)
                {
                    let need = expr::bits_needed(&out.value, false);
                    let msg = format!("   needs {need} bits");
                    if o.x + msg.len() < w {
                        o.t(&msg, "warning");
                    }
                }
            }
            Err(e) => {
                // While typing at the end of the line an error is usually just
                // an unfinished expression: keep it quiet until Enter.
                let quiet = !self.failed && e.pos.is_none_or(|p| p >= text.trim_end().len());
                let msg: String = e.message.chars().take(w.saturating_sub(6)).collect();
                o.t(&msg, if quiet { "muted" } else { "error" });
            }
        }
    }

    fn body_rows(&self, w: usize) -> Vec<Row> {
        let mut rows = vec![blank()];
        let cache = self.cache.as_ref().expect("cache");
        let hv = self.hover;
        let avail = w.saturating_sub(LEFT + 1);
        let mut value_line = |label: &str, text: &str, role: &str, fmt_: Fmt, extra: Option<(&str, &str)>| {
            let mut parts = wrap(text, avail);
            let total = parts.len();
            if total > MAX_VALUE_LINES {
                let hidden: usize = parts[MAX_VALUE_LINES - 1..].iter().map(|p| p.chars().count()).sum();
                parts.truncate(MAX_VALUE_LINES - 1);
                parts.push(format!("… {hidden} more characters · click to copy all"));
            }
            let n = parts.len();
            for (i, part) in parts.iter().enumerate() {
                let mut o = Out::default();
                o.t("  ", "text");
                if i == 0 {
                    o.hit(label, "muted", "", Hit::Copy(fmt_), hv);
                }
                o.pad(LEFT, "");
                let muted = part.starts_with('…') && total > MAX_VALUE_LINES && i == n - 1;
                let role = if muted { "muted" } else { role };
                let x0 = o.x;
                o.styled(part, role, "", fmt_ == Fmt::Hex && !muted, hv == Some(Hit::Copy(fmt_)));
                o.hits.push((x0 as u16, o.x as u16, Hit::Copy(fmt_)));
                if i == n - 1
                    && let Some((what, alt)) = extra
                {
                    let note = format!("   {what} {alt}");
                    if o.x + note.chars().count() < w {
                        if what.is_empty() {
                            o.t("    ", "text").hit(alt, "muted", "", Hit::Copy(Fmt::Verilog), hv);
                        } else {
                            o.t(&note, "muted");
                        }
                    }
                }
                rows.push(Row::Out(o));
            }
        };
        let verilog = fmt::verilog(&self.value, self.width);
        value_line("HEX", &cache.hex, "text", Fmt::Hex, Some(("", verilog.as_str())));
        value_line(
            "DEC",
            &cache.dec,
            "text",
            Fmt::Dec,
            cache.alt.as_ref().map(|(k, v)| (*k, v.as_str())),
        );
        if self.width <= 64 {
            value_line("OCT", &cache.oct, "muted", Fmt::Oct, None);
        }
        for (i, (label, val, parts)) in cache.floats.iter().enumerate() {
            let mut o = Out::default();
            o.t("  ", "text").hit(label, "muted", "", Hit::Copy(Fmt::Float(i)), hv).pad(LEFT, "");
            o.hit(val, "text", "", Hit::Copy(Fmt::Float(i)), hv);
            let detail = format!("   {parts}");
            if o.x + detail.chars().count() < w {
                o.t(&detail, "muted");
            }
            rows.push(Row::Out(o));
        }
        if let Some((exact, need)) = &self.overflow {
            let mut o = Out::default();
            let kept = if exact.sign() == num_bigint::Sign::Minus { "wrapped" } else { "kept the low bits" };
            o.pad(LEFT, "").t(&format!("Result needs {need} bits; {kept}  "), "warning");
            o.hit(&format!(" widen to {need} "), "accent", "panel", Hit::Widen(*need), hv);
            rows.push(Row::Out(o));
        }

        // Bit grid
        rows.push(blank());
        for r in 0..self.row_count() {
            rows.push(Row::Ruler(r));
            rows.push(Row::Bits(r));
            rows.push(Row::Hex(r));
            if self.row_fields(r).next().is_some() {
                rows.push(Row::Fields(r));
            }
        }

        // Field values
        let visible: Vec<usize> = (0..self.fields.len()).filter(|i| self.fields[*i].lo < self.width).collect();
        if !visible.is_empty() {
            rows.push(blank());
            let mut o = Out::default();
            o.t("  ", "text").pad(LEFT, "");
            for i in visible {
                let f = &self.fields[i];
                let hi = f.hi.min(self.width - 1);
                let n = hi - f.lo + 1;
                let v = self.field_value(hi, f.lo);
                let val = if n <= 4 { format!("{v}") } else { fmt::hex_grouped(&v, n) };
                let val = if val.chars().count() > 22 { format!("{}…", val.chars().take(21).collect::<String>()) } else { val };
                let len = f.name.len() + val.chars().count() + 4;
                if o.x + len > w && o.x > LEFT {
                    rows.push(Row::Out(std::mem::take(&mut o)));
                    o.pad(LEFT, "");
                }
                let x0 = o.x;
                let hovered = hv == Some(Hit::FieldName(i));
                o.styled(&f.name, chart(i), "", true, hovered).t(" ", "text").t(&val, "text").t("    ", "text");
                o.hits.push((x0 as u16, (o.x - 4) as u16, Hit::FieldName(i)));
            }
            rows.push(Row::Out(o));
        }

        // Field strip
        if let Some((hi, lo)) = self.field() {
            rows.push(blank());
            let n = hi - lo + 1;
            let v = self.field_value(hi, lo);
            let mut o = Out::default();
            let name = if hi == lo { format!("[{hi}]") } else { format!("[{hi}:{lo}]") };
            let field_name = self.fields.iter().find(|f| f.hi.min(self.width - 1) == hi && f.lo == lo).map(|f| f.name.clone());
            o.t("  ", "text");
            if let Some(fname) = &field_name {
                o.b(fname, "accent").t(" ", "text").t(&name, "muted");
            } else {
                o.b(&name, "accent");
            }
            let mut desc = Vec::new();
            if n == 1 {
                o.t("  = ", "muted");
                desc.push((format!("{}", v), "text"));
            } else {
                o.t(&format!("  {n} bits  "), "muted");
                desc.push((fmt::hex_grouped(&v, n), "text"));
            }
            if n > 1 {
                desc.push((fmt::dec_grouped(&BigInt::from(v.clone())), "text"));
                if n > 1 && v.bit(n as u64 - 1) {
                    desc.push((format!("signed {}", fmt::dec_grouped(&expr::to_signed(&v, n))), "muted"));
                }
                if n <= 16 {
                    desc.push((format!("0b{}", fmt::group(&fmt::bin(&v, n), 4, '_')), "muted"));
                }
            }
            let actions = [
                ("set", FieldOp::Set),
                ("clear", FieldOp::Clear),
                ("invert", FieldOp::Invert),
                ("write…", FieldOp::Write),
                ("extract", FieldOp::Extract),
                ("copy", FieldOp::Copy),
            ];
            let actions_len: usize = actions.iter().map(|(l, _)| l.len() + 3).sum::<usize>() + 3;
            let room = w.saturating_sub(o.x + actions_len);
            for (text, role) in desc {
                if text.chars().count() + 3 <= room.saturating_sub(o.x.saturating_sub(LEFT)) || o.x < LEFT + 10 {
                    let clipped: String = text.chars().take(w.saturating_sub(o.x + 2)).collect();
                    o.t(&clipped, role).t("   ", "text");
                }
            }
            let mut a = if o.x + actions_len <= w {
                o
            } else {
                rows.push(Row::Out(o));
                let mut a = Out::default();
                a.pad(LEFT, "");
                a
            };
            for (label, op) in actions {
                a.hit(&format!(" {label} "), "accent", "panel", Hit::Field(op), hv).t(" ", "text");
            }
            a.hit(" ✕ ", "muted", "", Hit::Field(FieldOp::Close), hv);
            rows.push(Row::Out(a));
        }

        // Names
        if !self.vars.is_empty() {
            rows.push(blank());
            let mut o = Out::default();
            o.t("  ", "text").t("names", "muted").pad(LEFT + 2, "");
            for (i, (name, var)) in self.vars.iter().enumerate() {
                let val = fmt::hex_grouped(&var.value, var.width);
                let val = if val.len() > 24 { format!("{}…", &val[..22]) } else { val };
                let item_len = name.len() + val.len() + 4;
                if o.x + item_len > w && o.x > LEFT + 2 {
                    rows.push(Row::Out(std::mem::take(&mut o)));
                    o.pad(LEFT + 2, "");
                }
                let x0 = o.x;
                o.b(name, "text").t(" ", "text").t(&val, "muted").t("    ", "text");
                o.hits.push((x0 as u16, (o.x - 3) as u16, Hit::Var(i)));
            }
            rows.push(Row::Out(o));
        }

        // History
        if !self.history.is_empty() {
            rows.push(blank());
            let mut o = Out::default();
            o.t("  history", "muted");
            rows.push(Row::Out(o));
            let expr_w = self.history.iter().map(|e| e.expr.chars().count()).max().unwrap_or(0).min(w / 2).max(12);
            for (i, e) in self.history.iter().enumerate().take(100) {
                let mut o = Out::default();
                o.t("  ", "text");
                let x0 = o.x;
                let expr: String = if e.expr.chars().count() > expr_w {
                    e.expr.chars().take(expr_w - 1).chain(['…']).collect()
                } else {
                    e.expr.clone()
                };
                let hovered = hv == Some(Hit::History(i));
                o.styled(&expr, "text", "", false, hovered).pad(x0 + expr_w + 3, "");
                let val = fmt::hex_grouped(&e.value, e.width);
                let room = w.saturating_sub(o.x + 1);
                let val: String = if val.chars().count() > room { val.chars().take(room.saturating_sub(1)).chain(['…']).collect() } else { val };
                o.t(&val, "muted");
                if e.width != self.width && o.x + 8 < w {
                    o.t(&format!("  ({}b)", e.width), "muted");
                }
                o.hits.push((x0 as u16, w as u16, Hit::History(i)));
                rows.push(Row::Out(o));
            }
        }
        rows
    }

    // ── bit grid rows ──

    fn active_bit(&self) -> Option<u32> {
        match self.hover {
            Some(Hit::Bit(b)) => Some(b),
            _ if self.focus == Focus::Bits => Some(self.cursor_bit),
            _ => None,
        }
    }

    fn ruler(&self, r: u32) -> Out {
        let rb = row_bits(self.view_w, self.width);
        let lo = self.row_lo(r);
        let hi = (lo + rb).min(self.width) - 1;
        let len = LEFT + span_of(rb) + 1;
        let mut buf = vec![' '; len];
        let place = |buf: &mut Vec<char>, b: u32| {
            let x = bit_x(rb, b - lo);
            let label = b.to_string();
            let start = (x + 1).saturating_sub(label.len());
            for (k, c) in label.chars().enumerate() {
                if let Some(slot) = buf.get_mut(start + k) {
                    *slot = c;
                }
            }
            (start, x + 1)
        };
        let mut b = lo;
        while b <= hi {
            place(&mut buf, b);
            b += 4;
        }
        // Exact indices for the selection's ends and the hovered/cursor bit
        // replace the nearby nibble labels.
        let mut marks: Vec<u32> = Vec::new();
        if let Some((a, c)) = self.sel {
            marks.extend([a.max(c), a.min(c)]);
        }
        marks.extend(self.active_bit());
        let mut accent = vec![false; len];
        for m in marks.into_iter().filter(|b| *b >= lo && *b <= hi) {
            let x = bit_x(rb, m - lo);
            let label = m.to_string();
            let start = (x + 1).saturating_sub(label.len());
            for k in start.saturating_sub(1)..(x + 2).min(len) {
                buf[k] = ' ';
                accent[k] = false;
            }
            for (k, c) in label.chars().enumerate() {
                buf[start + k] = c;
                accent[start + k] = true;
            }
        }
        let mut o = Out::default();
        let mut i = 0;
        while i < len {
            let j = (i..len).find(|k| accent[*k] != accent[i]).unwrap_or(len);
            let text: String = buf[i..j].iter().collect();
            if accent[i] {
                o.b(&text, "accent");
            } else {
                o.t(&text, "muted");
            }
            i = j;
        }
        o
    }

    fn in_sel(&self, b: u32) -> bool {
        self.sel.is_some_and(|(a, c)| b >= a.min(c) && b <= a.max(c))
    }

    fn bits(&self, r: u32) -> Out {
        let rb = row_bits(self.view_w, self.width);
        let lo = self.row_lo(r);
        let hi = (lo + rb).min(self.width) - 1;
        let mut o = Out::default();
        if r == 0 {
            o.t("  ", "text").hit("BIN", "muted", "", Hit::Copy(Fmt::Bin), self.hover);
        }
        let hover_bit = match self.hover {
            Some(Hit::Bit(b)) => Some(b),
            _ => None,
        };
        for b in (lo..=hi).rev() {
            let x = bit_x(rb, b - lo);
            if x > o.x {
                // Gap: keep selection background continuous inside a selection.
                let gap_sel = self.in_sel(b) && self.in_sel(b + 1);
                o.pad(x, if gap_sel { "selection" } else { "" });
                if let Some(last) = o.hits.last_mut()
                    && matches!(last.2, Hit::Bit(_))
                {
                    last.1 = x as u16;
                }
            }
            let one = self.value.bit(b as u64);
            let changed = self.prev.bit(b as u64) != one;
            let sign = self.signed && b == self.width - 1;
            let mut role = if changed {
                "changed"
            } else if sign && one {
                "warning"
            } else if one {
                "text"
            } else {
                "muted"
            };
            let mut bg = if self.in_sel(b) { "selection" } else { "" };
            if self.focus == Focus::Bits && b == self.cursor_bit {
                bg = "cursor";
                role = "cursor";
            }
            let x0 = o.x;
            o.styled(if one { "1" } else { "0" }, role, bg, one || changed, hover_bit == Some(b));
            o.hits.push((x0 as u16, o.x as u16, Hit::Bit(b)));
        }
        o
    }

    fn hex_row(&self, r: u32) -> Out {
        let rb = row_bits(self.view_w, self.width);
        let lo = self.row_lo(r);
        let hi = (lo + rb).min(self.width) - 1;
        let mut o = Out::default();
        let mut b = (hi / 4) * 4;
        loop {
            let top = (b + 3).min(hi);
            let x_left = bit_x(rb, top - lo);
            let x_digit = bit_x(rb, (b + 2).min(top) - lo);
            let nib = ((&self.value >> b) & BigUint::from(15u32)).iter_u32_digits().next().unwrap_or(0);
            let old = ((&self.prev >> b) & BigUint::from(15u32)).iter_u32_digits().next().unwrap_or(0);
            let digit = format!("{nib:X}");
            let selected = (b..=top).all(|k| self.in_sel(k));
            let role = if nib != old { "changed" } else { "accent" };
            o.pad(x_left, "");
            let x0 = o.x;
            let hovered = self.hover == Some(Hit::Nibble(b));
            let bg = if selected { "selection" } else { "" };
            o.on(&" ".repeat(x_digit - x_left), "text", bg);
            o.styled(&digit, role, bg, true, hovered);
            let x_right = bit_x(rb, b - lo) + 1;
            o.pad(x_right, bg);
            o.hits.push((x0 as u16, o.x as u16, Hit::Nibble(b)));
            if b < lo + 4 {
                break;
            }
            b -= 4;
        }
        o
    }

    fn footer(&self, o: &mut Out, w: usize, more: bool) {
        o.t(" ", "text");
        let hint: Vec<(&str, &str)> = match self.focus {
            Focus::Input => vec![
                ("Enter", "evaluate"),
                ("↑↓", "history"),
                ("Tab", "bits"),
                ("Alt+W", "width"),
                ("Alt+S", "sign"),
                ("Alt+H", "help"),
            ],
            Focus::Bits => vec![
                ("←→↑↓", "move"),
                ("Shift", "select"),
                ("Space", "toggle"),
                ("1 0 ~", "set clear invert"),
                ("Enter", "write field"),
                ("x", "extract"),
                ("Esc", "back"),
            ],
            Focus::Width => vec![("↑↓", "±1 (Shift ±8)"), ("Enter", "apply"), ("Esc", "cancel")],
        };
        let status: Option<(String, &str)> = if let Some((text, tone)) = &self.flash {
            Some((
                text.clone(),
                match tone {
                    Tone::Good => "success",
                    Tone::Bad => "error",
                    Tone::Info => "text",
                },
            ))
        } else {
            match self.hover {
                Some(Hit::Bit(b)) => {
                    let one = self.value.bit(b as u64);
                    Some((
                        format!("bit {b} = {}  ·  byte {}  ·  click toggles, drag selects", one as u8, b / 8),
                        "text",
                    ))
                }
                Some(Hit::Nibble(b)) => Some((format!("bits [{}:{b}]  ·  click selects the nibble", (b + 3).min(self.width - 1)), "text")),
                Some(Hit::Copy(f)) => {
                    let s = self.format(f);
                    let s = if s.chars().count() > 40 { format!("{}…", s.chars().take(39).collect::<String>()) } else { s };
                    Some((format!("click to copy {s}"), "text"))
                }
                Some(Hit::WidthChip) => Some(("click to change the width · wheel ±1".into(), "text")),
                Some(Hit::SignChip) => Some(("click to switch signed / unsigned interpretation".into(), "text")),
                Some(Hit::History(_)) => Some(("click loads this result  ·  double-click edits the expression  ·  ↑ recalls it".into(), "text")),
                Some(Hit::FieldName(i)) => self.fields.get(i).map(|f| {
                    (format!("{} [{}:{}]  ·  click selects it  ·  double-click writes it", f.name, f.hi, f.lo), "text")
                }),
                Some(Hit::Var(_)) => Some(("click inserts the name into the expression".into(), "text")),
                _ => None,
            }
        };
        if let Some((text, role)) = status {
            let text: String = text.chars().take(w.saturating_sub(2)).collect();
            o.t(&text, role);
            return;
        }
        for (key, what) in hint {
            let len = key.chars().count() + what.chars().count() + 4;
            if o.x + len > w.saturating_sub(if more { 8 } else { 0 }) {
                break;
            }
            o.t(key, "accent").t(" ", "text").t(what, "muted").t("   ", "text");
        }
        if more {
            o.pad(w.saturating_sub(7), "").t("more ↓", "muted");
        }
    }

    fn help_rows(&self, w: usize) -> Vec<Row> {
        let mut rows = vec![blank()];
        let mut head = Out::default();
        head.t("  ", "text").b("Reference", "text").t("   Esc or Alt+H closes · anything else you do closes it too", "muted");
        rows.push(Row::Out(head));
        rows.push(blank());
        let section = |rows: &mut Vec<Row>, title: &str, items: &[(&str, &str)]| {
            let mut o = Out::default();
            o.t("  ", "text").b(title, "accent");
            rows.push(Row::Out(o));
            let key_w = items.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0).min(30) + 3;
            for (k, v) in items {
                let mut o = Out::default();
                o.t("  ", "text").t(k, "text").pad(2 + key_w, "");
                let room = w.saturating_sub(o.x + 1);
                for (i, piece) in wrap(v, room.max(10)).into_iter().enumerate() {
                    if i > 0 {
                        rows.push(Row::Out(std::mem::take(&mut o)));
                        o.pad(2 + key_w, "");
                    }
                    o.t(&piece, "muted");
                }
                rows.push(Row::Out(o));
            }
            rows.push(blank());
        };
        section(
            &mut rows,
            "Numbers",
            &[
                ("255  0xFF  0b1111  0o377", "decimal, hex, binary, octal; _ separates digits"),
                ("8'hFF  12'b1010  'd99  8'sh80", "Verilog literals; a size truncates, 's sign-extends"),
                ("4K  2M  1G", "binary multiples (4K = 4096)"),
                ("ans  [15:8]", "the current value; a bare range selects from it"),
            ],
        );
        section(
            &mut rows,
            "Operators",
            &[
                ("+ - * / % **", "exact integer arithmetic; / truncates toward zero"),
                ("& | ^ ~", "bitwise; ~ inverts within the width"),
                ("<< >> >>>", ">> is logical, >>> arithmetic (sign-filling)"),
                ("== != < <= > >= && || !", "comparisons and logic give 0 or 1"),
                ("x[7]  x[15:8]  x[8+:4]  x[11-:4]", "bit select and part selects, as in Verilog"),
                ("{a, b, 4'hF}  {4{2'b10}}", "concatenation and replication; unsized parts use the word width"),
            ],
        );
        section(
            &mut rows,
            "Assignments",
            &[
                ("name = expr", "store a value; click a name to insert it"),
                ("name[7:4] = 0xA", "write a field of a stored value"),
                ("[31] = 1    [15:8] = 0xA5", "write a field of the current value"),
            ],
        );
        section(
            &mut rows,
            "Register fields",
            &[
                (":fields op[6:0] rd[11:7]", "name bit ranges; drawn under the bits and listed with their values"),
                ("rd   rd = 5   rd[1:0] = 3", "a field name reads or writes that range of the current value"),
                ("click a field", "select its bits; double-click to write it"),
                (":fields", "with nothing after it, removes the layout"),
            ],
        );
        section(
            &mut rows,
            "Commands",
            &[
                (":width 48   :w 48", "set the word width"),
                (":signed   :unsigned", "set how values are interpreted"),
                (":del name", "forget a stored value"),
                (":clear", "clear the history"),
            ],
        );
        let fns: Vec<(String, String)> = FUNCTIONS
            .iter()
            .map(|f| (format!("{}({})", f.name, f.args), f.help.to_string()))
            .collect();
        let fns_ref: Vec<(&str, &str)> = fns.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        section(&mut rows, "Functions", &fns_ref);
        section(
            &mut rows,
            "Bits",
            &[
                ("click a bit", "toggle it"),
                ("drag across bits", "select a field; then set, clear, invert, write, extract or copy it"),
                ("click a hex digit", "select its four bits"),
                ("Tab", "move focus to the grid; arrows move, Shift extends, Space toggles"),
                ("1  0  ~", "set, clear or invert the selected bits"),
                ("Enter  x  c", "write the field from an expression, extract it, copy it"),
            ],
        );
        section(
            &mut rows,
            "Width and sign",
            &[
                ("Alt+W or click the width", "choose any width from 1 to 8192 bits; wheel adjusts by 1"),
                ("Alt+S or click the sign", "signed changes how values read: decimals, ans, >>> and comparisons"),
                ("results", "are exact, then truncated to the width; overflow offers to widen"),
                ("Ctrl+Z  Ctrl+Y", "undo and redo value changes"),
                ("click HEX / DEC / BIN …", "copy that representation; Ctrl+C copies hex"),
            ],
        );
        rows
    }
}
