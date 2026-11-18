// node --test references/slides/tally/: the update command's checks and the page's data block.
"use strict";
const test = require("node:test");
const assert = require("node:assert");
const fs = require("fs");
const path = require("path");
const { check, block, BLOCK } = require("./tally-set");

test("a good tally passes and gets a time", () => {
  const d = check({ groups: [{ label: " Developers ", room: 3, online: 40 }] });
  assert.deepStrictEqual(d.groups, [{ label: "Developers", room: 3, online: 40 }]);
  assert.strictEqual(d.title, "Who's in the room");
  assert.ok(!isNaN(Date.parse(d.updated_at)));
  assert.strictEqual(d.debug, undefined);
});

test("a missing count is 0, and unknown fields are dropped", () => {
  const d = check({ title: "Hands up", groups: [{ label: "A", room: 2, colour: "red" }], updated_at: "2026-11-19T03:10:00Z" });
  assert.deepStrictEqual(d, { title: "Hands up", groups: [{ label: "A", room: 2, online: 0 }], updated_at: "2026-11-19T03:10:00.000Z" });
});

test("bad tallies are refused", () => {
  for (const bad of [
    [],
    { groups: "A" },
    { groups: [{ room: 1 }] },
    { groups: [{ label: "A", room: -1 }] },
    { groups: [{ label: "A", online: 1.5 }] },
    { groups: [{ label: "A", room: "3" }] },
    { groups: Array.from({ length: 9 }, (_, i) => ({ label: "G" + i })) },
    { groups: [], updated_at: "soon" },
  ]) assert.throws(() => check(bad), undefined, JSON.stringify(bad));
});

test("the page's data block is found, and a label can't close it", () => {
  const html = fs.readFileSync(path.join(__dirname, "index.html"), "utf8");
  assert.ok(BLOCK.test(html), "index.html has the tally-data block");
  const next = block(JSON.stringify(check({ groups: [{ label: "</script><b>x", room: 1 }] })));
  assert.ok(BLOCK.test(next), "the new block matches the pattern the next update looks for");
  assert.strictEqual(next.indexOf("</script>"), next.length - "</script>".length);
  const json = next.replace(/^<script[^>]*>/, "").replace(/<\/script>$/, "");
  assert.strictEqual(JSON.parse(json).groups[0].label, "</script><b>x");
});

test("the page and the command name the same stream by default", () => {
  const html = fs.readFileSync(path.join(__dirname, "index.html"), "utf8");
  const src = fs.readFileSync(path.join(__dirname, "tally-set"), "utf8");
  const stream = html.match(/<meta name="ss-streams" content="([^"]+)">/)[1];
  assert.ok(src.includes(`|| "${stream}"`), "tally-set defaults to " + stream);
});

// The slide's own model(), run from index.html as it is: what it shows for a tally.
function pageModel() {
  const html = fs.readFileSync(path.join(__dirname, "index.html"), "utf8");
  const helpers = ["num", "fmt"].map(n => html.match(new RegExp(`^ *function ${n}\\(.*$`, "m"))[0]).join("\n");
  const body = html.match(/\/\/ model:start[^\n]*\n([\s\S]*?)\/\/ model:end/)[1];
  return new Function(helpers + "\n" + body + "\nreturn model;")();
}

test("with every online count 0, the slide shows the room alone", () => {
  const m = pageModel()(check({ groups: [{ label: "Developers", room: 12 }, { label: "Founders", room: 5, online: 0 }] }));
  assert.strictEqual(m.showOnline, false);
  assert.strictEqual(m.total, 17);
  assert.strictEqual(m.split, "in the room", "the total above is the room: not said twice");
  assert.strictEqual(m.note, "");
  assert.deepStrictEqual(m.rows.map(r => r.roomPct), [100, 5 / 12 * 100]);
  assert.strictEqual(pageModel()(check({ groups: [] })).showOnline, false, "the empty slide too");
});

test("one online count shows the Online column, each column to its own scale", () => {
  const m = pageModel()(check({ groups: [{ label: "Developers", room: 12, online: 0 }, { label: "Founders", room: 5, online: 40 }] }));
  assert.strictEqual(m.showOnline, true);
  assert.strictEqual(m.total, 57);
  assert.strictEqual(m.split, "17 in the room · 40 online");
  assert.deepStrictEqual(m.rows.map(r => [r.roomPct, r.onlinePct]), [[100, 0], [5 / 12 * 100, 100]]);
});

test("the room-only layout hides every Online part and is the slide's first state", () => {
  const html = fs.readFileSync(path.join(__dirname, "index.html"), "utf8");
  assert.match(html, /\.room-only \.online\{display:none\}/);
  assert.match(html, /<main class="slide room-only" id="slide"/);
  assert.match(html, /classList\.toggle\("room-only", !m\.showOnline\)/);
  // Every Online part carries the online class: the column's heading and each row's cell.
  assert.match(html, /<div class="colh online">/);
  assert.match(html, /cell\("online"\)/);
});
