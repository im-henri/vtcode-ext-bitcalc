"""Render JSON frames from `cargo run --example frames` into an HTML page."""
import html
import json
import sys

FG = {"text": "#d4d4d4", "muted": "#7f8796", "accent": "#61afef", "offset": "#c678dd",
      "changed": "#e5c07b", "warning": "#e5c07b", "error": "#e06c75", "success": "#98c379",
      "selection": "#ffffff", "cursor": "#1e1e1e"}
BG = {"": "", "panel": "#2a2f38", "header": "#21252b", "selection": "#3e4f6a",
      "cursor": "#61afef", "match": "#5c4b1e"}


def main(src, dst):
    frames = json.load(open(src, encoding="utf-8"))
    out = ["<!doctype html><meta charset=utf-8><style>body{background:#111;color:#ddd;font-family:sans-serif}"
           ".f{background:#1e1e1e;font:14px/1.25 Consolas,monospace;white-space:pre;display:inline-block;"
           "padding:6px;margin:6px 0 24px;border:1px solid #333}.l{height:1.25em}</style>"]
    for item in frames:
        f = item["frame"]
        out.append(f"<h3>{html.escape(item['name'])}</h3><div class=f style='width:{f['width']}ch'>")
        cur = f.get("cursor")
        for y, line in enumerate(f["lines"]):
            out.append("<div class=l>")
            x = 0
            for s in line:
                text = s["text"]
                style = f"color:{FG.get(s.get('role', ''), FG['text'])};"
                bg = BG.get(s.get("background", ""), "")
                if bg:
                    style += f"background:{bg};"
                if s.get("bold"):
                    style += "font-weight:bold;"
                if s.get("underline"):
                    style += "text-decoration:underline;"
                if cur and cur[1] == y and x <= cur[0] < x + len(text):
                    k = cur[0] - x
                    out.append(f"<span style='{style}'>{html.escape(text[:k])}</span>"
                               f"<span style='{style}outline:1px solid #fff'>{html.escape(text[k] if text[k] != ' ' else ' ')}</span>"
                               f"<span style='{style}'>{html.escape(text[k+1:])}</span>")
                else:
                    out.append(f"<span style='{style}'>{html.escape(text)}</span>")
                x += len(text)
            if cur and cur[1] == y and cur[0] >= x:
                out.append(" " * (cur[0] - x) + "<span style='outline:1px solid #fff'> </span>")
            out.append("</div>")
        out.append("</div>")
    open(dst, "w", encoding="utf-8").write("".join(out))


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
