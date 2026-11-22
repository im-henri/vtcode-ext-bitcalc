# Bitcalc

A programmer's calculator for VTCode with no 64-bit ceiling. The word can be
any width from 1 to 8192 bits, and expressions use the Verilog notation
hardware engineers already think in: `x[15:8]`, `{a, b}`, `8'hFF`.

```
 Bitcalc    32 bits ▾   unsigned                                                    help

  › rd = 3
    = 0x00B5_01B3   11,862,451

  HEX  0x00B5_01B3    32'h00b5_01b3
  DEC  11,862,451
  OCT  0o00055200663
  F32  1.662283436942839e−38   s 0  exp 0x1 (2^−126)  frac 0x3501B3

         28   24    20   16    12    8     4    0
  BIN  0000 0000  1011 0101  0000 0001  1011 0011
        0    0     B    5     0    1     B    3
       └funct7┘└ rs2 ┘ └ rs1 ┘└f┘ └ rd ─┘└opcode┘

       funct7 0x00    rs2 0x0B    rs1 0x0A    funct3 0    rd 0x03    opcode 0x33

  rd [11:7]  5 bits  0x03   3     set   clear   invert   write…   extract   copy   ✕
```

## The idea

The current value is the centerpiece. It is shown in every base at once. The
bit grid labels each column with its bit index above and its hex digit below,
so you never count bits by hand. Everything else is there to change that
value:

- **Type an expression** and press Enter. The line under the input previews the
  result as you type.
- **Click a bit** to toggle it. **Drag across bits** to select a field, then
  set, clear, invert, write, extract or copy it. Clicking a hex digit selects
  its nibble.
- **Name register fields** with `:fields op[6:0] rd[11:7] …`. They are drawn
  under the bits and listed with their values. The names then work in
  expressions: `rd` reads the field and `rd = 5` writes it.
- Bits changed by the last operation are highlighted, so you can see what an
  operation did.

Results are computed exactly and then truncated to the word width, as
hardware does. If a result does not fit, Bitcalc says how many bits it
needs and offers a one-click **widen to N**. Writing `-1` to get all ones is
not treated as an overflow.

## Expressions

| Write | Meaning |
| --- | --- |
| `255` `0xFF` `0b1111` `0o377` `1_000` | Numbers; `_` separates digits |
| `8'hFF` `12'b1010` `'d99` `8'sh80` | Verilog literals; a size truncates, `'s` sign-extends |
| `4K` `2M` `1G` | Binary multiples (`4K` = 4096) |
| `+ - * / % **` | Exact integer arithmetic; `/` truncates toward zero |
| `& \| ^ ~` | Bitwise operators; `~` inverts within the width |
| `<< >> >>>` | `>>` is logical, `>>>` is arithmetic |
| `== != < <= > >= && \|\| !` | Comparisons and logic give 0 or 1 |
| `x[7]` `x[15:8]` `x[8+:4]` `x[11-:4]` | Bit select and part selects |
| `{a, b, 4'hF}` `{4{2'b10}}` | Concatenation and replication; unsized parts use the word width |
| `ans` `[15:8]` | The current value; a bare range selects from it |
| `name = expr` `name[7:4] = 0xA` | Store a value or write one of its fields |
| `[31] = 1` `[15:8] = 0xA5` | Write a field of the current value |

Functions: `mask(n)`, `mask(hi, lo)`, `bit(n)`, `set/clr/flip(x, i [, lo])`,
`ins(x, hi, lo, v)`, `sext(x, w)`, `zext(x, w)`, `popcount`, `parity`,
`clz(x [, w])`, `ctz`, `msb`, `lsb`, `clog2`, `log2`, `rev(x [, w])`,
`bswap(x [, w])`, `rol/ror(x, n [, w])`, `gray`, `ungray`, `alignup`,
`aligndown`, `cdiv`, `abs`, `min`, `max`, `gcd`. `f16`, `bf16`, `f32` and
`f64` encode a real number, e.g. `f32(1.5)`. At 16, 32 and 64 bits the value is
also decoded as IEEE 754, with sign, exponent and fraction.

Commands: `:fields …` (a bare `:fields` clears the layout), `:width N`,
`:signed`, `:unsigned`, `:del name`, `:clear`.

## Keys

| Where | Keys |
| --- | --- |
| Input | Enter evaluates · ↑↓ recall history · Tab moves to the bits · Esc clears |
| Bits | ←→↑↓ move · Shift extends · Space toggles · `1` `0` `~` set/clear/invert · Enter writes the field · `x` extracts · `c` copies · typing jumps back to the input |
| Anywhere | Alt+W width · Alt+S signed/unsigned · Alt+H help · Ctrl+Z / Ctrl+Y undo/redo · Ctrl+C copy |

Click HEX, DEC, OCT, BIN or the Verilog literal to copy that form. A wheel turn
on the width chip changes the width by 1. Width, sign, value, named values,
fields and history persist per workspace.

## Build

Requires Rust 1.98.0 with `wasm32-unknown-unknown` (the pinned
`rust-toolchain.toml` requests it) and Python 3.11+.

```powershell
python build.py --install-local C:\path\to\workspace
```

Then run **Extensions: Reload** and **Bitcalc: Open Calculator** in VTCode.
`python build.py --package dist/vtcode-extension.zip` creates the runtime-only
release package. `python tools/licenses.py` regenerates the third-party
notices after dependency changes.

To review the UI without the editor:

```powershell
cargo run --example frames > target/frames.json
python tools/frames_html.py target/frames.json target/frames.html
```
