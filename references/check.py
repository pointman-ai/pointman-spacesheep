#!/usr/bin/env python3
"""Check a Spacesheep page for walls of text before you deploy it.

  python3 check.py page.html          (or a folder with an index.html)
  python3 check.py page.html --json

It prints one line per finding, `rule: where (section heading): what`, and exits 1 when
anything fails (0 when there are only warnings, or nothing). It reads the HTML as written:
text a script adds later isn't seen. It counts only text people see: not scripts, styles,
the bodies of closed <details>, code or <pre>. An inline <code> counts as one word.

  first-visual  fail  a visual within the page's first FIRST_VISUAL_WORDS words
  paragraph     fail  at most PARAGRAPH_SENTENCES sentences and PARAGRAPH_WORDS words
  sentence      fail  at most SENTENCE_WORDS words
  text-run      fail  at most RUN_WORDS words of running text between visuals
  heading       fail  a heading isn't empty or generic (GENERIC_HEADINGS)
  long-page     warn  more than PAGE_WORDS visible words outside <details>
  link          warn  a link in the middle of a paragraph: put it at the end, or fold it
  motion        warn  something that moves on its own: autoplay, <marquee>, an endless animation
  viewport      fail  no <meta name="viewport">

A visual is a table, svg, canvas, img, figure, pre, list (ol or ul), video, iframe, a diff
(class "diff"), KPI tiles (class "kpis"), or anything with role="img".
"""
import argparse
import json
import re
import sys
from html.parser import HTMLParser
from pathlib import Path

FIRST_VISUAL_WORDS = 150
PARAGRAPH_SENTENCES = 3
PARAGRAPH_WORDS = 60
SENTENCE_WORDS = 25
RUN_WORDS = 120
PAGE_WORDS = 1500
GENERIC_HEADINGS = {"overview", "background", "introduction", "intro", "details", "summary", "notes", "note", "context"}

VISUAL_TAGS = {"table", "svg", "canvas", "img", "figure", "pre", "ol", "ul", "video", "iframe", "picture"}
VISUAL_CLASSES = {"diff", "kpis"}
VISUAL_ROLES = {"img"}

HEADINGS = {"h1", "h2", "h3", "h4", "h5", "h6"}
# controls and labels: their words are seen, but they aren't prose
UI = {"button", "label", "summary", "select", "option", "legend", "caption"}
# blocks whose text is a paragraph (a div with loose text is one too)
PROSE = {"p", "li", "dd", "blockquote", "figcaption", "td", "div", "section", "article", "main", "header", "footer", "aside", "body", "address"}
BLOCKS = PROSE | HEADINGS | UI | {"dt", "th", "dl", "figure", "form", "fieldset", "nav", "hgroup", "table", "thead", "tbody", "tfoot", "tr", "ol", "ul", "pre", "details", "html"}
SKIP = {"script", "style", "template", "noscript", "title", "desc", "textarea", "head"}
VOID = {"area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"}
ABBREVIATIONS = {"e.g", "i.e", "etc", "vs", "cf", "approx", "fig", "no", "mr", "mrs", "ms", "dr", "st", "inc", "ltd", "jr", "sr", "a.m", "p.m"}

# an open <p> ends when one of these starts (as browsers do), unless it's inside one of SCOPE first
CLOSES_P = {"address", "article", "aside", "blockquote", "details", "div", "dl", "fieldset", "figcaption", "figure",
            "footer", "form", "header", "hgroup", "hr", "main", "nav", "ol", "p", "pre", "section", "table", "ul"} | HEADINGS
SCOPE = {"html", "table", "td", "th", "caption", "template", "button", "object", "marquee"}
IMPLIED = [  # (tags that start, open tags they end, where the search stops)
    (CLOSES_P, {"p"}, SCOPE),
    ({"li"}, {"li"}, SCOPE | {"ol", "ul"}),
    ({"dt", "dd"}, {"dt", "dd"}, SCOPE | {"dl"}),
    ({"td", "th"}, {"td", "th"}, {"table", "tr"}),
    ({"tr"}, {"tr"}, {"table", "thead", "tbody", "tfoot"}),
    ({"thead", "tbody", "tfoot"}, {"thead", "tbody", "tfoot"}, {"table"}),
    ({"option"}, {"option"}, {"select", "datalist"}),
]
HIDDEN_STYLE = re.compile(r"display\s*:\s*none|visibility\s*:\s*hidden", re.I)
ENDLESS = re.compile(r"animation(?:-iteration-count)?\s*:\s*([^;{}]*\binfinite\b[^;{}]*)", re.I)


class Node:
    def __init__(self, tag, attrs, line):
        self.tag, self.attrs, self.line, self.children = tag, attrs, line, []


class Text(str):
    pass


class Tree(HTMLParser):
    """the page as a tree, closing what browsers close for you (<p>, <li>, <td> …)"""

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.root = Node("#root", {}, 1)
        self.stack = [self.root]

    def _end(self, names, stop):
        for i in range(len(self.stack) - 1, 0, -1):
            tag = self.stack[i].tag
            if tag in names:
                del self.stack[i:]
                return
            if tag in stop:
                return

    def handle_starttag(self, tag, attrs):
        for starts, ends, stop in IMPLIED:
            if tag in starts:
                self._end(ends, stop)
        node = Node(tag, {k: v or "" for k, v in attrs}, self.getpos()[0])
        self.stack[-1].children.append(node)
        if tag not in VOID:
            self.stack.append(node)

    def handle_startendtag(self, tag, attrs):
        self.stack[-1].children.append(Node(tag, {k: v or "" for k, v in attrs}, self.getpos()[0]))

    def handle_endtag(self, tag):
        self._end({tag}, set())

    def handle_data(self, data):
        self.stack[-1].children.append(Text(data))


def words(text):
    return sum(1 for t in text.split() if re.search(r"\w", t))


def sentences(text):
    """split after . ! ? … when the next word starts a sentence, not after e.g. or an initial"""
    out, start = [], 0
    for m in re.finditer(r"[.!?…]+[\"”’')\]]*\s+(?=\S)", text):
        before = text[start:m.start()].split()
        last = before[-1].lower().lstrip("(\"“‘'[").rstrip(".") if before else ""
        nxt = text[m.end()]
        if last in ABBREVIATIONS or (len(last) == 1 and last.isalpha()) or nxt.islower():
            continue
        out.append(text[start:m.end()].strip())
        start = m.end()
    out.append(text[start:].strip())
    return [s for s in out if words(s)]


def snippet(text, n=8):
    toks = text.split()
    return '"' + " ".join(toks[:n]) + ("…" if len(toks) > n else "") + '"'


def plain(node):
    """all of a node's text, alt text included (for headings)"""
    if isinstance(node, Text):
        return str(node)
    if node.tag in SKIP:
        return ""
    return node.attrs.get("alt", "") + "".join(plain(c) for c in node.children)


class Block:
    """one element's own text: a paragraph, a list item, a cell, a heading …"""

    def __init__(self, el, kind, heading):
        self.el, self.kind, self.heading = el, kind, heading
        self.parts = []  # (text, the link it's in or None)


class Check:
    def __init__(self):
        self.findings = []
        self.heading = None  # the latest heading's text
        self.seen = 0  # visible words so far
        self.outside_details = 0
        self.first_visual = None  # (line, tag, heading, words before it)
        self.run = None  # running text since the last visual: [words, line, heading, first text]
        self.viewport = False
        self.links = 0

    def add(self, rule, level, line, what, heading=None):
        heading = heading if heading is not None else self.heading
        self.findings.append({
            "rule": rule, "level": level, "line": line, "section": heading or "top", "what": what,
        })

    # walking the tree

    def walk(self, node, ctx):
        for child in node.children:
            if isinstance(child, Text):
                self.text(child, ctx)
            else:
                self.element(child, ctx)

    def element(self, el, ctx):
        tag, a = el.tag, el.attrs
        if tag == "meta" and a.get("name", "").lower() == "viewport":
            self.viewport = True
        self.motion(el)
        if tag == "head":
            return self.walk(el, {**ctx, "quiet": True})
        if tag in SKIP or "hidden" in a or HIDDEN_STYLE.search(a.get("style", "")) or (tag == "dialog" and "open" not in a):
            return
        if tag == "br" and ctx["block"]:
            ctx["block"].parts.append((" ", None))
            return
        if tag == "code" and not ctx["code"]:
            # one word, whatever it holds, and no sentence ends inside it
            code = plain(el).strip()
            return self.text(Text("`" + re.sub(r"\s+", "·", code) + "`" if code else ""), ctx)
        visual = tag in VISUAL_TAGS or VISUAL_CLASSES & set(a.get("class", "").split()) or a.get("role") in VISUAL_ROLES
        if visual and not ctx["visual"]:
            self.visual(el)
            ctx = {**ctx, "visual": True}
        if tag == "pre":
            ctx = {**ctx, "code": True}
        if tag == "details":
            return self.details(el, ctx)
        if tag in HEADINGS:
            self.new_heading(el)
        if tag == "a" and "href" in a and not ctx["link"]:
            self.links += 1
            ctx = {**ctx, "link": (self.links, el.line, plain(el).strip())}
        if tag in BLOCKS:
            kind = "heading" if tag in HEADINGS else "ui" if tag in UI else "prose" if tag in PROSE else "other"
            if ctx["block"] and ctx["block"].kind in ("heading", "ui") and kind not in ("heading", "ui"):
                kind = ctx["block"].kind  # a div inside a <summary> is still the summary
            block = Block(el, kind, self.heading)
            self.walk(el, {**ctx, "block": block, "aside": ctx["aside"] or kind in ("heading", "ui")})
            return self.paragraph(block)
        self.walk(el, ctx)

    def details(self, el, ctx):
        """the summary is seen; the body only when it's open"""
        inside = {**ctx, "details": True}
        for child in el.children:
            if isinstance(child, Text):
                if "open" in el.attrs:
                    self.text(child, inside)
            elif child.tag == "summary" or "open" in el.attrs:
                self.element(child, inside)

    def text(self, t, ctx):
        if ctx["quiet"] or ctx["code"]:
            return
        n = words(t)
        self.seen += n
        if not ctx["details"]:
            self.outside_details += n
        if n and self.run is not None and not ctx["visual"] and not ctx["aside"]:
            if not self.run[0]:
                self.run[1:] = [ctx["block"].el.line if ctx["block"] else 0, self.heading, str(t)]
            self.run[0] += n
        if ctx["block"]:
            ctx["block"].parts.append((str(t), ctx["link"]))

    def visual(self, el):
        if self.first_visual is None:
            self.first_visual = (el.line, el.tag, self.heading, self.seen)
        self.end_run()
        self.run = [0, None, None, ""]

    def end_run(self):
        if self.run and self.run[0] > RUN_WORDS:
            n, line, heading, first = self.run
            self.add("text-run", "fail", line,
                     f"{n} words of running text before the next visual, from {snippet(first)}; at most {RUN_WORDS}", heading)

    def new_heading(self, el):
        text = " ".join(plain(el).split()) or el.attrs.get("aria-label", "").strip()
        if not text:
            return self.add("heading", "fail", el.line, f"an empty <{el.tag}>")
        self.heading = text if len(text) <= 60 else text[:57] + "…"
        bare = re.sub(r"[^\w\s]", "", text.lower())
        bare = re.sub(r"^(the|a|an|some|more|other|further|general)\s+", "", bare).strip()
        if bare in GENERIC_HEADINGS:
            self.add("heading", "fail", el.line, f'"{text}" is a generic heading; say what this section shows or found')

    def paragraph(self, block):
        if block.kind in ("heading", "ui"):
            return
        text = "".join(t for t, _ in block.parts)
        n = words(text)
        if not n:
            return
        line, heading = block.el.line, block.heading
        found = sentences(text)
        for s in found:
            if words(s) > SENTENCE_WORDS:
                self.add("sentence", "fail", line, f"{words(s)} words in {snippet(s)}; at most {SENTENCE_WORDS}", heading)
        if block.kind != "prose":
            return
        if len(found) > PARAGRAPH_SENTENCES or n > PARAGRAPH_WORDS:
            self.add("paragraph", "fail", line,
                     f"{len(found)} sentences and {n} words in {snippet(text)}; at most {PARAGRAPH_SENTENCES} and {PARAGRAPH_WORDS}", heading)
        for link in {l for _, l in block.parts if l}:
            at = [i for i, (_, l) in enumerate(block.parts) if l == link]
            before = words("".join(t for t, _ in block.parts[:at[0]]))
            after = words("".join(t for t, _ in block.parts[at[-1] + 1:]))
            if before and after:
                self.add("link", "warn", link[1],
                         f'the link "{link[2][:40]}" is in the middle of {snippet(text, 6)}; put it at the end, or fold it into <details>', heading)

    def motion(self, el):
        tag, a = el.tag, el.attrs
        if tag in ("marquee", "blink"):
            self.add("motion", "warn", el.line, f"<{tag}> moves on its own")
        if "autoplay" in a:
            self.add("motion", "warn", el.line, f"<{tag} autoplay> plays on its own")
        if tag in ("animate", "animatetransform", "animatemotion") and a.get("repeatcount") == "indefinite":
            self.add("motion", "warn", el.line, f"an SVG <{tag}> repeats forever")
        if tag == "style":
            css = "".join(c for c in el.children if isinstance(c, Text))
            for m in ENDLESS.finditer(css):
                self.add("motion", "warn", el.line + css[:m.start()].count("\n"),
                         f'"{" ".join(m.group(0).split())}" never stops; run it once, or on a click', "<style>")
        for m in ENDLESS.finditer(a.get("style", "")):
            self.add("motion", "warn", el.line, f'"{" ".join(m.group(0).split())}" never stops; run it once, or on a click')

    def finish(self):
        self.end_run()
        if self.first_visual is None:
            if self.seen:
                self.add("first-visual", "fail", 1, "no visual on the page: a table, list, figure, svg, img, pre or diff", "top")
        else:
            line, tag, heading, before = self.first_visual
            if before > FIRST_VISUAL_WORDS:
                self.add("first-visual", "fail", line,
                         f"{before} words before the first visual (a <{tag}>); at most {FIRST_VISUAL_WORDS}", heading)
        if self.outside_details > PAGE_WORDS:
            self.add("long-page", "warn", 0,
                     f"{self.outside_details:,} visible words outside <details>; fold detail away, or split the page (over {PAGE_WORDS:,})", "whole page")
        if not self.viewport:
            self.add("viewport", "fail", 0, 'no <meta name="viewport" content="width=device-width, initial-scale=1">: a phone shows it zoomed out', "head")


def check_html(html):
    """the findings for a page's HTML, by line, and how many words people see"""
    tree = Tree()
    tree.feed(html.lstrip("﻿"))
    tree.close()
    c = Check()
    c.walk(tree.root, {"quiet": False, "code": False, "visual": False, "details": False,
                       "aside": False, "link": None, "block": None})
    c.finish()
    c.findings.sort(key=lambda f: f["line"])
    return c.findings, c.outside_details


def where(f):
    return "page" if f["rule"] == "long-page" else "head" if f["rule"] == "viewport" else f"line {f['line']}"


def main(argv=None):
    ap = argparse.ArgumentParser(description="Check a Spacesheep page for walls of text before you deploy it.")
    ap.add_argument("page", help="a page's .html file, or a folder with an index.html")
    ap.add_argument("--json", action="store_true", help="the findings as JSON")
    args = ap.parse_args(argv)
    path = Path(args.page)
    if path.is_dir():
        path = path / "index.html"
    if not path.is_file():
        print(f"check.py: no page at {path}", file=sys.stderr)
        return 2
    findings, seen = check_html(path.read_text(encoding="utf-8", errors="replace"))
    fails = sum(f["level"] == "fail" for f in findings)
    warns = len(findings) - fails
    if args.json:
        print(json.dumps({"page": str(path), "words": seen, "fails": fails, "warnings": warns, "findings": findings}, indent=2))
    else:
        for f in findings:
            print(f"{f['rule']}: {where(f)} ({f['section']}): {f['what']}" + (" (warning)" if f["level"] == "warn" else ""))
        print(f"check.py: {path}: {fails} failing, {warns} warnings, {seen:,} words outside <details>", file=sys.stderr)
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
