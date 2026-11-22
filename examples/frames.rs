//! Render scripted scenarios to JSON frames for visual review:
//! `cargo run --example frames > target/frames.json`, then
//! `python tools/frames_html.py target/frames.json target/frames.html`.
use vtcode_bitcalc::app::{App, Event};

fn app(w: u16, h: u16) -> App {
    let mut a = App::new();
    a.event(&Event { kind: "resize".into(), width: w, height: h, ..Event::default() });
    a
}

fn run(a: &mut App, exprs: &[&str]) {
    for e in exprs {
        a.input.set(e);
        a.evaluate();
    }
}

fn typed(a: &mut App, text: &str) {
    for c in text.chars() {
        a.event(&Event::ch(c));
    }
}

fn hover_bit(a: &mut App, bit: u32) {
    a.render();
    if let Some((y, x0, _, _)) = a.hits.iter().find(|h| h.3 == vtcode_bitcalc::app::Hit::Bit(bit)).copied() {
        a.event(&Event::mouse("move", x0, y));
    }
}

fn drag_bits(a: &mut App, from: u32, to: u32) {
    a.render();
    let find = |a: &App, b| a.hits.iter().find(|h| h.3 == vtcode_bitcalc::app::Hit::Bit(b)).copied().unwrap();
    let (y0, x0, _, _) = find(a, from);
    let (y1, x1, _, _) = find(a, to);
    a.event(&Event::mouse("down", x0, y0));
    a.event(&Event::mouse("drag", x1, y1));
    a.event(&Event::mouse("up", x1, y1));
}

fn main() {
    let mut frames = Vec::new();
    let mut push = |name: &str, a: &mut App| {
        let f = a.render();
        frames.push(serde_json::json!({ "name": name, "frame": f }));
    };

    let mut a = app(110, 34);
    push("empty 110x34", &mut a);

    let mut a = app(110, 34);
    run(&mut a, &["base = 0x4000_0000", "0xdead_beef", "[7:4] = 0xA"]);
    typed(&mut a, "ans ^ mask(15, 8)");
    push("typing with history 110x34", &mut a);

    let mut a = app(110, 34);
    run(&mut a, &["0xdead_beef"]);
    drag_bits(&mut a, 15, 8);
    hover_bit(&mut a, 20);
    push("selection + hover 110x34", &mut a);

    let mut a = app(110, 40);
    a.set_width(48);
    run(&mut a, &["{16'hCAFE, 32'h0123_4567}", "x = ans", "x[47] = 1"]);
    push("48-bit with names", &mut a);

    let mut a = app(120, 44);
    a.set_width(128);
    run(&mut a, &["mask(128) ^ (0xFF << 60)"]);
    a.toggle_bit(3);
    push("128-bit 120 wide", &mut a);

    let mut a = app(120, 44);
    a.set_width(1000);
    run(&mut a, &["(1 << 999) | 0xdead_beef"]);
    push("1000-bit", &mut a);

    let mut a = app(100, 30);
    a.set_width(10);
    run(&mut a, &["0x3A5", "ans + 0x100"]);
    push("10-bit overflow", &mut a);

    let mut a = app(100, 30);
    run(&mut a, &["f32(1.5)"]);
    a.toggle_signed();
    a.event(&Event::key("tab"));
    a.event(&Event { code: "left".into(), shift: true, ..Event::key("left") });
    a.event(&Event { code: "left".into(), shift: true, ..Event::key("left") });
    push("f32 signed, bits focus", &mut a);

    let mut a = app(100, 30);
    a.open_width_editor();
    push("width editor", &mut a);

    let mut a = app(100, 30);
    typed(&mut a, "foo + 3");
    a.evaluate();
    push("error", &mut a);

    let mut a = app(60, 30);
    run(&mut a, &["0x1234_5678", "ans >> 4"]);
    push("narrow 60", &mut a);

    let mut a = app(110, 40);
    run(&mut a, &[":fields funct7[31:25] rs2[24:20] rs1[19:15] funct3[14:12] rd[11:7] opcode[6:0]", "0x00B50533", "rd = 3"]);
    hover_bit(&mut a, 9);
    push("riscv fields", &mut a);

    let mut a = app(110, 40);
    a.help = true;
    push("help", &mut a);

    println!("{}", serde_json::to_string(&frames).unwrap());
}
