use crate::app::{App, Event, FieldOp, Focus, Hit};
use crate::expr::{self, Env, Var};
use num_bigint::BigUint;
use std::collections::BTreeMap;

fn eval_w(src: &str, width: u32, signed: bool) -> Result<String, String> {
    let ans = BigUint::from(0xF0u32);
    let mut vars = BTreeMap::new();
    vars.insert("x".to_string(), Var { value: BigUint::from(0xA5u32), width: 8 });
    vars.insert("neg".to_string(), Var { value: BigUint::from(0xFFu32), width: 8 });
    let fields = expr::parse_fields("hi8[31:24] lo4[3:0]").unwrap();
    let env = Env { width, signed, ans: &ans, vars: &vars, fields: &fields };
    expr::evaluate(src, &env)
        .map(|o| format!("{:X}", expr::truncate(&o.value, width)))
        .map_err(|e| e.message)
}

fn ev(src: &str) -> String {
    eval_w(src, 32, false).unwrap_or_else(|e| panic!("{src}: {e}"))
}

#[test]
fn literals() {
    assert_eq!(ev("255"), "FF");
    assert_eq!(ev("0xdead_beef"), "DEADBEEF");
    assert_eq!(ev("0b1010_0101"), "A5");
    assert_eq!(ev("0o17"), "F");
    assert_eq!(ev("8'hFF"), "FF");
    assert_eq!(ev("4'hFF"), "F");
    assert_eq!(ev("12'b1010"), "A");
    assert_eq!(ev("'d99"), "63");
    assert_eq!(ev("8'sh80"), "FFFFFF80");
    assert_eq!(ev("4K"), "1000");
    assert_eq!(ev("1M + 1"), "100001");
    assert!(eval_w("12ab", 32, false).is_err());
    assert!(eval_w("8'hxz", 32, false).unwrap_err().contains("x/z"));
}

#[test]
fn arithmetic_and_precedence() {
    assert_eq!(ev("1 + 2 * 3"), "7");
    assert_eq!(ev("(1 + 2) * 3"), "9");
    assert_eq!(ev("2 ** 3 ** 2"), "200");
    assert_eq!(ev("-2 ** 2"), "FFFFFFFC");
    assert_eq!(ev("-1"), "FFFFFFFF");
    assert_eq!(ev("7 / 2"), "3");
    assert_eq!(ev("-7 / 2"), "FFFFFFFD");
    assert_eq!(ev("1 | 2 ^ 3 & 4"), "3");
    assert_eq!(ev("1 << 4 + 1"), "20");
    assert_eq!(ev("~0"), "FFFFFFFF");
    assert_eq!(ev("!5"), "0");
    assert_eq!(ev("3 > 2 && 2 > 1"), "1");
    assert!(eval_w("1 / 0", 32, false).unwrap_err().contains("zero"));
}

#[test]
fn shifts_are_width_aware() {
    assert_eq!(ev("0x80000000 >> 4"), "8000000");
    assert_eq!(ev("0x80000000 >>> 4"), "F8000000");
    assert_eq!(ev("-16 >> 2"), "3FFFFFFC");
    assert_eq!(eval_w("8'h80 >>> 1", 32, false).unwrap(), "FFFFFFC0");
    assert_eq!(eval_w("8'h80 >> 1", 32, false).unwrap(), "40");
    assert_eq!(ev("1 <<< 3"), "8");
}

#[test]
fn wide_values() {
    assert_eq!(eval_w("1 << 127", 128, false).unwrap(), format!("8{}", "0".repeat(31)));
    assert_eq!(eval_w("mask(1000)", 1000, false).unwrap().len(), 250);
    assert_eq!(eval_w("-1", 7, false).unwrap(), "7F");
    assert_eq!(eval_w("0x1FF", 1, false).unwrap(), "1");
}

#[test]
fn selects_and_concat() {
    assert_eq!(ev("0xABCD[15:8]"), "AB");
    assert_eq!(ev("0xABCD[3]"), "1");
    assert_eq!(ev("0xABCD[4+:8]"), "BC");
    assert_eq!(ev("0xABCD[11-:8]"), "BC");
    assert_eq!(ev("[7:4]"), "F");
    assert_eq!(ev("ans[7:4]"), "F");
    assert_eq!(ev("{4'hA, 4'h5}"), "A5");
    assert_eq!(ev("{4{2'b10}}"), "AA");
    assert_eq!(ev("{x, x[3:0]}"), "A55");
    assert_eq!(ev("{8'h1, 8'd1}"), "101");
    assert_eq!(eval_w("{8'h1, 1}", 16, false).unwrap(), "1");
    assert!(eval_w("0xFF[0:7]", 32, false).unwrap_err().contains("high bit first"));
}

#[test]
fn signed_mode_reads_names_as_signed() {
    assert_eq!(eval_w("neg", 16, false).unwrap(), "FF");
    assert_eq!(eval_w("neg", 16, true).unwrap(), "FFFF");
    assert_eq!(eval_w("neg < 0", 16, true).unwrap(), "1");
    assert_eq!(eval_w("neg < 0", 16, false).unwrap(), "0");
}

#[test]
fn functions() {
    assert_eq!(ev("popcount(0xFF00)"), "8");
    assert_eq!(ev("clz(1)"), "1F");
    assert_eq!(ev("clz(1, 8)"), "7");
    assert_eq!(ev("ctz(0x80)"), "7");
    assert_eq!(ev("clog2(1025)"), "B");
    assert_eq!(ev("clog2(1024)"), "A");
    assert_eq!(ev("log2(1025)"), "A");
    assert_eq!(ev("rev(1, 8)"), "80");
    assert_eq!(ev("bswap(0x11223344)"), "44332211");
    assert_eq!(ev("rol(0x80000001, 1)"), "3");
    assert_eq!(ev("ror(1, 1, 8)"), "80");
    assert_eq!(ev("sext(0x80, 8)"), "FFFFFF80");
    assert_eq!(ev("zext(-1, 4)"), "F");
    assert_eq!(ev("mask(15, 8)"), "FF00");
    assert_eq!(ev("set(0, 31)"), "80000000");
    assert_eq!(ev("clr(0xFF, 7, 4)"), "F");
    assert_eq!(ev("flip(0, 3, 0)"), "F");
    assert_eq!(ev("ins(0xFFFF, 11, 4, 0)"), "F00F");
    assert_eq!(ev("gray(5)"), "7");
    assert_eq!(ev("ungray(7)"), "5");
    assert_eq!(ev("alignup(4097, 4K)"), "2000");
    assert_eq!(ev("aligndown(4097, 4K)"), "1000");
    assert_eq!(ev("cdiv(65, 64)"), "2");
    assert_eq!(ev("msb(0x100)"), "8");
    assert_eq!(ev("lsb(0)"), "FFFFFFFF");
    assert_eq!(ev("max(3, 9, 2)"), "9");
    assert_eq!(ev("gcd(12, 18)"), "6");
    assert_eq!(ev("f32(1.5)"), "3FC00000");
    assert_eq!(ev("f32(-2)"), "C0000000");
    assert_eq!(ev("f16(1.0)"), "3C00");
    assert_eq!(ev("f16(65504)"), "7BFF");
    assert_eq!(ev("bf16(1.0)"), "3F80");
    assert_eq!(eval_w("f64(0.1)", 64, false).unwrap(), "3FB999999999999A");
    assert!(eval_w("1.5 + 1", 32, false).is_err());
    assert!(eval_w("nosuch(1)", 32, false).unwrap_err().contains("Unknown function"));
    assert!(eval_w("rol(1)", 32, false).unwrap_err().contains("Usage"));
}

#[test]
fn error_positions() {
    let env_ans = BigUint::from(0u32);
    let vars = BTreeMap::new();
    let env = Env { width: 32, signed: false, ans: &env_ans, vars: &vars, fields: &[] };
    let e = expr::evaluate("1 + foo", &env).unwrap_err();
    assert_eq!(e.pos, Some(4));
    let e = expr::evaluate("(1 + 2", &env).unwrap_err();
    assert!(e.message.contains("')'"));
    let e = expr::evaluate("1 2", &env).unwrap_err();
    assert_eq!(e.message, "Missing operator");
}

fn app() -> App {
    let mut a = App::new();
    a.event(&Event { kind: "resize".into(), width: 110, height: 40, ..Event::default() });
    a
}

fn run(a: &mut App, src: &str) {
    a.input.set(src);
    a.evaluate();
}

fn hex(a: &App) -> String {
    format!("{:X}", a.value)
}

#[test]
fn evaluate_assign_and_history() {
    let mut a = app();
    run(&mut a, "x = 0x1234");
    assert_eq!(hex(&a), "1234");
    assert_eq!(a.vars["x"].value, BigUint::from(0x1234u32));
    run(&mut a, "x[3:0] = 0xF");
    assert_eq!(hex(&a), "123F");
    assert_eq!(a.vars["x"].value, BigUint::from(0x123Fu32));
    run(&mut a, "[31] = 1");
    assert_eq!(hex(&a), "8000123F");
    assert_eq!(a.sel, Some((31, 31)));
    assert_eq!(a.history.len(), 3);
    assert_eq!(a.history[0].expr, "[31] = 1");
    assert!(a.input.is_empty());
}

#[test]
fn overflow_and_widen() {
    let mut a = app();
    a.set_width(8);
    run(&mut a, "0x1FF");
    assert_eq!(hex(&a), "FF");
    assert_eq!(a.overflow.as_ref().map(|o| o.1), Some(9));
    a.widen(9);
    assert_eq!(a.width, 9);
    assert_eq!(hex(&a), "1FF");
    // -1 is an intended all-ones pattern, not an overflow.
    run(&mut a, "-1");
    assert!(a.overflow.is_none());
    // 0xFF in an 8-bit signed word is just the bit pattern.
    a.set_width(8);
    a.toggle_signed();
    run(&mut a, "0xFF");
    assert!(a.overflow.is_none());
}

#[test]
fn width_change_sign_extends_in_signed_mode() {
    let mut a = app();
    a.set_width(8);
    run(&mut a, "0x80");
    a.set_width(16);
    assert_eq!(hex(&a), "80");
    a.set_width(8);
    a.toggle_signed();
    a.set_width(16);
    assert_eq!(hex(&a), "FF80");
    a.set_width(4);
    assert_eq!(hex(&a), "0");
}

#[test]
fn undo_redo() {
    let mut a = app();
    run(&mut a, "5");
    run(&mut a, "7");
    a.set_width(64);
    a.undo();
    assert_eq!(a.width, 32);
    a.undo();
    assert_eq!(hex(&a), "5");
    a.redo();
    assert_eq!(hex(&a), "7");
}

fn hit_pos(a: &mut App, hit: Hit) -> (u16, u16) {
    a.render();
    let h = a.hits.iter().find(|h| h.3 == hit).unwrap_or_else(|| panic!("no hit {hit:?}"));
    (h.1, h.0)
}

#[test]
fn click_toggles_and_drag_selects() {
    let mut a = app();
    let (x, y) = hit_pos(&mut a, Hit::Bit(5));
    a.event(&Event::mouse("down", x, y));
    a.event(&Event::mouse("up", x, y));
    assert_eq!(hex(&a), "20");
    assert!(a.sel.is_none());

    let (x0, y0) = hit_pos(&mut a, Hit::Bit(15));
    let (x1, y1) = hit_pos(&mut a, Hit::Bit(8));
    a.event(&Event::mouse("down", x0, y0));
    a.event(&Event::mouse("drag", x1, y1));
    a.event(&Event::mouse("up", x1, y1));
    assert_eq!(a.field(), Some((15, 8)));
    assert_eq!(hex(&a), "20");
    a.field_op(FieldOp::Set);
    assert_eq!(hex(&a), "FF20");
    a.field_op(FieldOp::Invert);
    assert_eq!(hex(&a), "20");
    a.field_op(FieldOp::Write);
    assert_eq!(a.input.text(), "[15:8] = ");
    assert_eq!(a.focus, Focus::Input);
    a.input.insert("0xA5");
    a.evaluate();
    assert_eq!(hex(&a), "A520");
    a.field_op(FieldOp::Extract);
    assert_eq!(hex(&a), "A5");
}

#[test]
fn nibble_click_selects_four_bits() {
    let mut a = app();
    let (x, y) = hit_pos(&mut a, Hit::Nibble(8));
    a.event(&Event::mouse("down", x, y));
    assert_eq!(a.field(), Some((11, 8)));
}

#[test]
fn keyboard_grid() {
    let mut a = app();
    a.event(&Event::key("tab"));
    assert_eq!(a.focus, Focus::Bits);
    a.event(&Event::ch(' '));
    assert_eq!(hex(&a), "1");
    a.event(&Event::key("left"));
    a.event(&Event { shift: true, ..Event::key("left") });
    a.event(&Event { shift: true, ..Event::key("left") });
    assert_eq!(a.field(), Some((3, 1)));
    a.event(&Event::ch('1'));
    assert_eq!(hex(&a), "F");
    // Typing an expression character jumps back to the input.
    a.event(&Event::ch('a'));
    assert_eq!(a.focus, Focus::Input);
    assert_eq!(a.input.text(), "a");
}

#[test]
fn history_recall_and_copy() {
    let mut a = app();
    run(&mut a, "1 + 1");
    run(&mut a, "2 + 2");
    a.input.insert("dra");
    a.event(&Event::key("up"));
    assert_eq!(a.input.text(), "2 + 2");
    a.event(&Event::key("up"));
    assert_eq!(a.input.text(), "1 + 1");
    a.event(&Event::key("down"));
    a.event(&Event::key("down"));
    assert_eq!(a.input.text(), "dra");
    a.input.clear();
    a.event(&Event { code: "command".into(), text: Some("copy".into()), ..Event::default() });
    assert_eq!(a.clipboard_out.as_deref(), Some("0x00000004"));
    let (x, y) = hit_pos(&mut a, Hit::Copy(crate::app::Fmt::Verilog));
    a.event(&Event::mouse("down", x, y));
    assert_eq!(a.clipboard_out.as_deref(), Some("32'h0000_0004"));
}

#[test]
fn width_editor() {
    let mut a = app();
    a.event(&Event { alt: true, ..Event::ch('w') });
    assert_eq!(a.focus, Focus::Width);
    for c in "37".chars() {
        a.event(&Event::ch(c));
    }
    a.event(&Event::key("enter"));
    assert_eq!(a.width, 37);
    assert_eq!(a.focus, Focus::Input);
    a.open_width_editor();
    a.event(&Event::key("up"));
    assert_eq!(a.width, 38);
}

#[test]
fn persistence_roundtrip() {
    let mut a = app();
    a.set_width(100);
    a.toggle_signed();
    run(&mut a, "y = 1 << 99");
    let json = serde_json::to_string(&a.save()).unwrap();
    let mut b = App::new();
    b.load(serde_json::from_str(&json).unwrap());
    assert_eq!(b.width, 100);
    assert!(b.signed);
    assert_eq!(b.value, a.value);
    assert_eq!(b.vars, a.vars);
    assert_eq!(b.history, a.history);
}

#[test]
fn frames_fit_limits_at_max_width() {
    let mut a = app();
    a.set_width(expr::MAX_WIDTH);
    run(&mut a, "-1");
    for (w, h) in [(40, 12), (110, 40), (200, 120)] {
        a.event(&Event { kind: "resize".into(), width: w, height: h, ..Event::default() });
        let f = a.render();
        assert!(f.lines.len() <= h as usize);
        for line in &f.lines {
            let cols: usize = line.iter().map(|s| s.text.chars().count()).sum();
            assert!(cols <= w as usize, "line too wide at {w}: {cols}: {:?}", line.iter().map(|s| s.text.as_str()).collect::<String>());
            assert!(line.iter().all(|s| !s.text.chars().any(char::is_control)));
        }
        let spans: usize = f.lines.iter().map(Vec::len).sum();
        assert!(spans < 32768);
        assert!(serde_json::to_string(&f).unwrap().len() < 2 * 1024 * 1024);
    }
}

#[test]
fn field_names_read_and_write_ans() {
    assert_eq!(eval_w("hi8", 32, false).unwrap(), "0");
    assert_eq!(eval_w("lo4", 32, false).unwrap(), "0");
    assert_eq!(eval_w("hi8 = 0xAB", 32, false).unwrap(), "AB0000F0");
    assert_eq!(eval_w("lo4 = 0xF", 32, false).unwrap(), "FF");
    assert_eq!(eval_w("hi8[3:0] = 0xF", 32, false).unwrap(), "F0000F0");
    assert!(eval_w("lo4[4] = 1", 32, false).unwrap_err().contains("only 4 bits"));
}

#[test]
fn parse_fields_validates() {
    let f = expr::parse_fields("a[3:0], b[7:4] c[8]").unwrap();
    assert_eq!(f.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["c", "b", "a"]);
    assert!(expr::parse_fields("a[3:0] b[4:2]").unwrap_err().contains("overlaps"));
    assert!(expr::parse_fields("a[0:3]").unwrap_err().contains("high bit first"));
    assert!(expr::parse_fields("mask[3:0]").unwrap_err().contains("function"));
    assert!(expr::parse_fields("a").is_err());
    assert!(expr::parse_fields("").unwrap().is_empty());
}

#[test]
fn commands() {
    let mut a = app();
    run(&mut a, ":fields opcode[6:0] rd[11:7] funct3[14:12] rs1[19:15] rs2[24:20] funct7[31:25]");
    assert_eq!(a.fields.len(), 6);
    run(&mut a, "0x00B50533");
    run(&mut a, "rd");
    assert_eq!(hex(&a), "A");
    run(&mut a, "0x00B50533");
    run(&mut a, "rd = 3");
    assert_eq!(hex(&a), "B501B3");
    run(&mut a, ":w 64");
    assert_eq!(a.width, 64);
    run(&mut a, ":signed");
    assert!(a.signed);
    run(&mut a, "v = 1");
    run(&mut a, ":del v");
    assert!(a.vars.is_empty());
    run(&mut a, ":bogus");
    assert!(a.failed);
    assert_eq!(a.input.text(), ":bogus");
    a.input.clear();
    run(&mut a, ":fields");
    assert!(a.fields.is_empty());
    a.undo();
    assert_eq!(a.fields.len(), 6);
    let json = serde_json::to_string(&a.save()).unwrap();
    let mut b = App::new();
    b.load(serde_json::from_str(&json).unwrap());
    assert_eq!(b.fields, a.fields);
}

#[test]
fn clicking_a_field_label_selects_it() {
    let mut a = app();
    run(&mut a, ":fields rd[11:7]");
    let (x, y) = hit_pos(&mut a, Hit::FieldName(0));
    a.event(&Event::mouse("down", x, y));
    assert_eq!(a.field(), Some((11, 7)));
}

#[test]
fn drag_from_a_gap_still_selects() {
    let mut a = app();
    let (x, y) = hit_pos(&mut a, Hit::Bit(8));
    // The column right of bit 8 is a byte gap.
    assert_eq!(a.hit_at(x + 1, y), Some(Hit::Bit(8)));
}
