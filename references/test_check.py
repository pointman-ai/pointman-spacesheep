"""check.py's tests: python3 -m unittest (from references/)"""
import json
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import check

HERE = Path(__file__).parent
VIEWPORT = '<meta name="viewport" content="width=device-width, initial-scale=1">'
TABLE = "<table><tr><td>a</td><td>1</td></tr></table>"


def sentence(n):
    """a sentence of n words"""
    return " ".join(["word"] * (n - 1) + ["end."]).capitalize()


def para(*lengths):
    return "<p>" + " ".join(sentence(n) for n in lengths) + "</p>"


def ul(*lengths, tag="ul"):
    """a list, one item of each length (in sentences of at most 20 words)"""
    def item(n):
        return " ".join(sentence(min(20, n - k)) for k in range(0, n, 20))
    return f"<{tag}>" + "".join(f"<li>{item(n)}</li>" for n in lengths) + f"</{tag}>"


# a visual of 160 words: what follows it is past the first screen
FIRST_SCREEN = ul(*[20] * 8)


def page(body, head=VIEWPORT):
    return f"<!doctype html><html><head>{head}<title>t</title></head><body><main>{body}</main></body></html>"


def rules(html, level=None):
    findings, _ = check.check_html(html)
    return [f["rule"] for f in findings if level is None or f["level"] == level]


class Template(unittest.TestCase):
    def test_template_passes_with_nothing_to_say(self):
        findings, seen = check.check_html((HERE / "template.html").read_text())
        self.assertEqual(findings, [])
        self.assertGreater(seen, 100)

    def test_a_clean_page_passes(self):
        self.assertEqual(rules(page("<h1>Two fixes</h1>" + para(8, 10) + TABLE + para(12))), [])


class Rules(unittest.TestCase):
    def test_first_visual_after_150_words(self):
        late = page("<h1>Two fixes</h1>" + para(20, 20) * 4 + TABLE)  # 2 + 160 words first
        self.assertEqual(rules(late), ["first-visual"])
        self.assertEqual(rules(page("<h1>Two fixes</h1>" + para(20, 20) * 3 + TABLE)), [])

    def test_no_visual_at_all(self):
        self.assertEqual(rules(page("<h1>Two fixes</h1>" + para(10))), ["first-visual"])

    def test_each_kind_of_visual_counts(self):
        for visual in ["<svg></svg>", "<canvas></canvas>", '<img src="a.png" alt="">', "<figure></figure>",
                       "<pre>x</pre>", "<ol><li>a</li></ol>", "<ul><li>a</li></ul>", '<div class="diff"></div>',
                       '<div class="kpis"></div>', '<div role="img" aria-label="a chart"></div>']:
            with self.subTest(visual=visual):
                self.assertEqual(rules(page(para(20, 20) * 3 + visual + para(20, 20) * 3)), [])

    def test_no_cap_on_a_paragraphs_sentences(self):
        self.assertEqual(rules(page(TABLE + para(5, 5, 5, 5, 5, 5))), [])

    def test_paragraph_over_60_words(self):
        self.assertEqual(rules(page(TABLE + para(20, 20, 21))), ["paragraph"])
        self.assertEqual(rules(page(TABLE + para(20, 20, 20))), [])

    def test_loose_text_in_a_div_is_a_paragraph(self):
        self.assertEqual(rules(page(TABLE + "<div>" + " ".join(sentence(20) for _ in range(4)) + "</div>")), ["paragraph"])

    def test_sentence_over_25_words(self):
        self.assertEqual(rules(page(TABLE + para(26))), ["sentence"])
        self.assertEqual(rules(page(TABLE + para(25))), [])

    def test_long_sentence_in_a_list_or_cell(self):
        self.assertEqual(rules(page(TABLE + f"<ul><li>{sentence(26)}</li></ul>")), ["sentence"])
        self.assertEqual(rules(page(TABLE + f"<table><tr><td>{sentence(26)}</td></tr></table>")), ["sentence", "cell"])

    def test_sentences_split_on_full_stops_not_abbreviations(self):
        self.assertEqual(len(check.sentences("It works, e.g. on phones. It is fast. Very fast!")), 3)
        self.assertEqual(len(check.sentences("Version 3.5 shipped. Then 3.6.")), 2)

    def test_running_text_over_120_words_between_visuals(self):
        self.assertEqual(rules(page(TABLE + para(20) * 7 + TABLE)), ["text-run"])
        self.assertEqual(rules(page(TABLE + para(20) * 6 + TABLE)), [])

    def test_running_text_at_the_end_of_the_page(self):
        self.assertEqual(rules(page(TABLE + para(20) * 7)), ["text-run"])

    def test_headings_and_visuals_are_not_running_text(self):
        body = TABLE + para(20) * 3 + "<h2>Fix 1: the headline counts</h2>" + para(20) * 3 + TABLE
        self.assertEqual(rules(page(body)), [])
        listed = TABLE + para(20) * 3 + "<ul>" + f"<li>{sentence(20)}</li>" * 5 + "</ul>" + para(20) * 3
        self.assertEqual(rules(page(listed)), [])

    def test_generic_heading(self):
        for h in ["Overview", "Background", "Introduction", "Details", "Summary", "Notes", "Context", "The details", "Summary:"]:
            with self.subTest(heading=h):
                self.assertEqual(rules(page(TABLE + f"<h2>{h}</h2>" + para(5))), ["heading"])
        self.assertEqual(rules(page(TABLE + "<h2>Details of the ranker</h2>" + para(5))), [])

    def test_empty_heading(self):
        self.assertEqual(rules(page(TABLE + "<h2> </h2>" + para(5))), ["heading"])
        self.assertEqual(rules(page(TABLE + '<h2><img src="a.png" alt="The ranker"></h2>' + para(5))), [])

    def test_long_page_is_a_warning(self):
        cells = "".join(f"<tr><td>{sentence(10)}</td></tr>" for _ in range(160))  # 1,600 words
        self.assertEqual(rules(page(f"<table>{cells}</table>")), ["long-page"])
        self.assertEqual(rules(page(f"<table>{cells}</table>"), "fail"), [])

    def test_words_in_details_dont_make_a_long_page(self):
        cells = "".join(f"<tr><td>{sentence(10)}</td></tr>" for _ in range(160))
        self.assertEqual(rules(page(TABLE + f"<details><summary>All cases</summary><table>{cells}</table></details>")), [])

    def test_link_mid_paragraph_is_a_warning(self):
        mid = page(TABLE + '<p>Read <a href="https://example.com">the spec</a> before you start.</p>')
        self.assertEqual(rules(mid), ["link"])
        self.assertEqual(rules(mid, "fail"), [])
        end = page(TABLE + '<p>The rules are in <a href="https://example.com">the spec</a>.</p>')
        self.assertEqual(rules(end), [])

    def test_things_that_move_are_warnings(self):
        for html in [
            page(TABLE, VIEWPORT + "<style>.dot{animation: pulse 2s ease infinite}</style>"),
            page(TABLE + '<div style="animation-iteration-count:infinite">x</div>'),
            page(TABLE + '<video autoplay src="a.mp4"></video>'),
            page(TABLE + "<marquee>news</marquee>"),
            page('<svg><circle r="4"><animate attributeName="r" repeatCount="indefinite"/></circle></svg>'),
        ]:
            with self.subTest(html=html):
                self.assertEqual(rules(html), ["motion"])
                self.assertEqual(rules(html, "fail"), [])
        self.assertEqual(rules(page(TABLE, VIEWPORT + "<style>.dot{animation: pulse 2s ease 1}</style>")), [])

    def test_no_viewport(self):
        self.assertEqual(rules(page(TABLE, head="")), ["viewport"])


class DocRules(unittest.TestCase):
    """docrules' changes to the shared list (rules.md, EXT-7)"""

    def test_a_sentence_in_a_step_has_at_most_20_words(self):
        self.assertEqual(rules(page(TABLE + f"<ol><li>{sentence(21)}</li><li>Short.</li></ol>")), ["sentence"])
        self.assertEqual(rules(page(TABLE + f"<ol><li>{sentence(20)}</li></ol>")), [])
        self.assertEqual(rules(page(TABLE + f"<ul><li>{sentence(21)}</li></ul>")), [])
        self.assertEqual(rules(page(TABLE + f"<ol><li>Pick one:<ul><li>{sentence(21)}</li></ul></li></ol>")), [])

    def test_long_list_items_are_running_text(self):
        # 80 words, then a list of two 26-word items: no visual, so the run goes on to 132
        self.assertEqual(rules(page(TABLE + para(20) * 4 + ul(26, 26) + TABLE)), ["text-run"])
        self.assertEqual(rules(page(TABLE + para(20) * 4 + ul(25, 25) + TABLE)), [])

    def test_a_list_with_a_short_item_is_still_a_visual_but_its_long_items_count(self):
        self.assertEqual(rules(page(TABLE + para(20) * 5 + ul(26, 5) + para(20) * 5 + TABLE)), ["text-run"])
        self.assertEqual(rules(page(TABLE + para(20) * 4 + ul(26, 5) + para(20) * 5 + TABLE)), [])

    def test_a_list_of_long_items_isnt_the_first_visual(self):
        self.assertEqual(rules(page(ul(26, 26))), ["first-visual"])

    def test_long_cells_are_running_text(self):
        cells = "".join(f"<tr><th>Case</th><td>{sentence(11)}</td></tr>" for _ in range(4))  # 44 words in cells
        found = rules(page(TABLE + para(20) * 4 + f"<table>{cells}</table>" + TABLE))
        self.assertEqual(sorted(found), ["cell"] * 4 + ["text-run"])

    def test_a_cell_has_at_most_10_words(self):
        self.assertEqual(rules(page(TABLE + f"<table><tr><td>{sentence(11)}</td><td>x</td></tr></table>")), ["cell"])
        self.assertEqual(rules(page(TABLE + f"<table><tr><th>{sentence(11)}</th></tr></table>")), ["cell"])
        self.assertEqual(rules(page(TABLE + f"<table><tr><td>{sentence(10)}</td></tr></table>")), [])

    def test_more_than_6_columns_is_a_warning(self):
        def table(*spans):
            return "<table><tr>" + "".join(f'<td colspan="{n}">a</td>' for n in spans) + "</tr></table>"
        self.assertEqual(rules(page(table(1, 1, 1, 1, 1, 1, 1))), ["columns"])
        self.assertEqual(rules(page(table(1, 1, 1, 1, 1, 2))), ["columns"])
        self.assertEqual(rules(page(table(1, 1, 1, 1, 1, 1))), [])
        self.assertEqual(rules(page(table(1, 1, 1, 1, 1, 1, 1)), "fail"), [])

    def test_at_most_4_kpi_tiles(self):
        def kpis(n, cls=' class="kpi"'):
            return '<div class="kpis">' + f"<div{cls}><b>3%</b> fewer</div>" * n + "</div>"
        self.assertEqual(rules(page(kpis(5))), ["kpis"])
        self.assertEqual(rules(page(kpis(5, ""))), ["kpis"])
        self.assertEqual(rules(page(kpis(4))), [])

    def test_code_names_in_the_first_screen(self):
        for lead in ["<code>rank</code>", "rank()", "the --json flag", "src/check.py", "commit 1dd6d673",
                     "home_env", "matchTitle", "spaces.headline", "~/Documents/pages"]:
            with self.subTest(lead=lead):
                self.assertEqual(rules(page(f"<p>It changes {lead} for you.</p>" + TABLE)), ["code-name"])
        self.assertEqual(rules(page(f"<p>It reads Node.js pages(s) on spacesheep.dev, e.g. on macOS.</p>" + TABLE)), [])

    def test_code_names_after_the_first_screen_are_fine(self):
        self.assertEqual(rules(page(FIRST_SCREEN + "<p>It changes <code>rank</code> in src/check.py.</p>")), [])

    def test_code_names_in_a_heading_or_kpi_label(self):
        self.assertEqual(rules(page(FIRST_SCREEN + "<h2>The <code>--json</code> flag</h2>" + para(5))), ["code-name"])
        self.assertEqual(rules(page(FIRST_SCREEN + "<h2>The rank() function</h2>" + para(5))), ["code-name"])
        kpi = '<div class="kpis"><div class="kpi"><b>3</b> files like clientspec.rs</div></div>'
        self.assertEqual(rules(page(FIRST_SCREEN + kpi)), ["code-name"])

    def test_no_code_names_across_elements(self):
        # "party" and "Edited" are two spans, not one camelCase word
        self.assertEqual(rules(page('<div><span>glass-party</span><span>Edited Sep 18</span></div>' + TABLE)), [])

    def test_a_task_pr_or_commit_link_may_sit_inline(self):
        for href in ["pointman://item/EXT-7", "https://github.com/pointman-ai/pointman/pull/42",
                     "https://github.com/pointman-ai/pointman/commit/1dd6d673"]:
            with self.subTest(href=href):
                self.assertEqual(rules(page(TABLE + f'<p>Read <a href="{href}">the change</a> before you start.</p>')), [])
        tree = "https://github.com/pointman-ai/pointman/tree/clientspec"
        self.assertEqual(rules(page(TABLE + f'<p>Read <a href="{tree}">the branch</a> before you start.</p>')), ["link"])

    def test_text_in_a_picture_isnt_prose(self):
        picture = f'<div role="img" aria-label="the bad cell"><p>{sentence(30)} --settings</p><table><tr><td>{sentence(12)}</td></tr></table></div>'
        self.assertEqual(rules(page(picture)), [])


class Counting(unittest.TestCase):
    def test_text_people_dont_see_isnt_counted(self):
        hidden = sentence(200)
        body = (TABLE + f"<script>var s = '{hidden}';</script><style>/* {hidden} */</style><pre>{hidden}</pre>"
                f"<details><summary>More</summary><p>{hidden}</p></details><template><p>{hidden}</p></template>"
                f'<p hidden>{hidden}</p><div style="display: none">{hidden}</div>')
        findings, seen = check.check_html(page(body))
        self.assertEqual(findings, [])
        self.assertEqual(seen, 2)  # the table's "a" and "1" (the summary is inside <details>)

    def test_an_open_details_is_seen(self):
        self.assertEqual(rules(page(TABLE + f"<details open><summary>More</summary>{para(30)}</details>")), ["sentence"])

    def test_inline_code_is_one_word(self):
        code = "<code>a very long expression with many words in it</code>"
        self.assertEqual(rules(page(FIRST_SCREEN + f"<p>{' '.join(['word'] * 23)} {code} end.</p>")), [])
        self.assertEqual(rules(page(FIRST_SCREEN + f"<p>{' '.join(['word'] * 24)} {code} end.</p>")), ["sentence"])

    def test_a_paragraph_ends_where_a_block_starts(self):
        # browsers close an open <p> at a <div>: two paragraphs, not one of 4 sentences
        self.assertEqual(rules(page(TABLE + f"<p>{sentence(5)} {sentence(5)}<div>{sentence(5)} {sentence(5)}</div>")), [])


class Cli(unittest.TestCase):
    def run_check(self, html, *args, folder=False):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / ("index.html" if folder else "page.html")
            path.write_text(html)
            target = tmp if folder else str(path)
            return subprocess.run([sys.executable, str(HERE / "check.py"), target, *args], capture_output=True, text=True)

    def test_a_failing_page_exits_1_with_one_line_per_finding(self):
        out = self.run_check(page(TABLE + para(20, 20, 21) + "<h2>Overview</h2>" + para(30)))
        self.assertEqual(out.returncode, 1)
        lines = out.stdout.splitlines()
        self.assertEqual([l.split(":")[0] for l in lines], ["paragraph", "heading", "sentence"])
        for line in lines:
            self.assertRegex(line, r"^[a-z-]+: (line \d+|page|head) \(.+\): .+")
        self.assertIn("(Overview)", lines[2])

    def test_warnings_only_exit_0(self):
        out = self.run_check(page(TABLE + '<p>Read <a href="https://example.com">the spec</a> first.</p>'))
        self.assertEqual(out.returncode, 0)
        self.assertTrue(out.stdout.rstrip().endswith("(warning)"))

    def test_json(self):
        out = self.run_check(page(TABLE + para(26)), "--json")
        data = json.loads(out.stdout)
        self.assertEqual((data["fails"], data["warnings"]), (1, 0))
        self.assertEqual(data["findings"][0]["rule"], "sentence")
        self.assertEqual(set(data["findings"][0]), {"rule", "level", "line", "section", "what"})

    def test_a_folder_with_index_html(self):
        self.assertEqual(self.run_check(page(TABLE), folder=True).returncode, 0)

    def test_no_page(self):
        out = subprocess.run([sys.executable, str(HERE / "check.py"), "/nonexistent/page.html"], capture_output=True, text=True)
        self.assertEqual(out.returncode, 2)


if __name__ == "__main__":
    unittest.main()
