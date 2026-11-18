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
