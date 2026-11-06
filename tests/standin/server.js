// A stand-in for Spacesheep's server, for the real CLI: GET /v1/doors.json (a trimmed table naming `spaces list` and
// `versions list`, or 404 with --no-doors) and the two calls, in pages of --page-size. Synthetic data only.
// Usage: node server.js [--no-doors] [--page-size N] [--spaces N] [--versions N] [--fail spaces|versions:STATUS]
"use strict";
const http = require("http");
const args = process.argv.slice(2);
const opt = (n, d) => { const i = args.indexOf(n); return i < 0 ? d : args[i + 1]; };
const noDoors = args.includes("--no-doors");
const size = Number(opt("--page-size", 2));
const nSpaces = Number(opt("--spaces", 5));
const nVersions = Number(opt("--versions", 5));
const [failWhat, failStatus] = String(opt("--fail", ":")).split(":");

const table = {
  services: [{
    name: "spaces",
    methods: [
      {
        http: "GET", path: "/v1/spaces", where: "query", doc: "List your spaces.",
        inputs: [{ name: "limit", type: "integer" }, { name: "after", type: "text" }],
        cli: { command: "spaces list", usage: "spaces list [--limit N] [--after CURSOR]", args: [],
          flags: [{ flag: "--limit", input: "limit", type: "integer" }, { flag: "--after", input: "after", type: "text" }] },
      },
      {
        http: "GET", path: "/v1/spaces/{space.id}/versions", where: "query", doc: "List a space's versions.",
        inputs: [{ name: "space_id", type: "text", at: ["space", "id"], ref: "id", required: true },
          { name: "limit", type: "integer" }, { name: "after", type: "text" }],
        cli: { command: "versions list", usage: "versions list --space-id ID [--limit N] [--after CURSOR]", args: [],
          flags: [{ flag: "--space-id", input: "space_id", type: "text", required: true },
            { flag: "--limit", input: "limit", type: "integer" }, { flag: "--after", input: "after", type: "text" }] },
      },
    ],
  }],
};

const spaces = Array.from({ length: nSpaces }, (_, i) => ({ id: `space-${i + 1}`, title: `Page ${i + 1}`, updated_at: `2026-11-0${(i % 9) + 1}T00:00:00Z` }));
const versions = (id) => Array.from({ length: nVersions }, (_, i) => ({
  id: `v${nVersions - i}`, created_at: `2026-11-01T00:${String(59 - i).padStart(2, "0")}:00Z`,
  client: { session_id: `session-of-${id}-${i === nVersions - 1 ? "first" : "later"}` },
}));
const paged = (rows, key, url) => {
  const after = url.searchParams.get("after") || "";
  const start = after ? Number(after.replace("c", "")) : 0;
  const rowsOut = rows.slice(start, start + size);
  const next = start + size < rows.length ? `c${start + size}` : "";
  return { [key]: rowsOut, page: { next } };
};

http.createServer((req, res) => {
  const url = new URL(req.url, "http://x");
  const send = (status, body) => { res.writeHead(status, { "content-type": "application/json" }); res.end(JSON.stringify(body)); };
  if (url.pathname === "/v1/doors.json") return noDoors ? send(404, { error: { code: "not_found", message: "no" } }) : send(200, table);
  if (!/^Bearer synthetic-/.test(req.headers.authorization || "")) return send(401, { error: { code: "unauthenticated", message: "that key is not accepted" } });
  const m = /^\/v1\/spaces\/([^/]+)\/versions$/.exec(url.pathname);
  if (url.pathname === "/v1/spaces") {
    if (failWhat === "spaces") return send(Number(failStatus), { error: { code: "internal", message: "the server is having trouble" } });
    return send(200, paged(spaces, "spaces", url));
  }
  if (m) {
    if (failWhat === "versions") return send(Number(failStatus), { error: { code: "internal", message: "the server is having trouble" } });
    return send(200, paged(versions(decodeURIComponent(m[1])), "versions", url));
  }
  send(404, { error: { code: "not_found", message: "no such call" } });
}).listen(0, "127.0.0.1", function () { console.log(this.address().port); });
