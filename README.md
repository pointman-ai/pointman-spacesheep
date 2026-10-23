# spacesheep

A Pointman extension that has agents write documents, reports and explainers as
[Spacesheep](https://spacesheep.dev) pages, published with Spacesheep's CLI and opened beside the thread for
the person they're working for. Spacesheep publishes pages and sites at a URL people open on any device. With
**Talk** on, a message typed in a page's Session tab reaches the agent that published it (or the agents it starts by calling), as a post in its
thread, and the agent answers there and on the page.

```
pointman add spacesheep
```

## How it works

- **A behaviour and a small process:** installed on a node, `SKILL.md`'s body goes into every agent run there,
  whichever agent it is and whether it runs on Claude Code, Codex or Gemini CLI. The process (`bin/spacesheep`,
  Rust) is only for Talk, and listens only while the account's Talk switch is on.
- **What agents do:** anything longer than a post becomes a page, not a Markdown file or a long post, unless
  the person asks for another format. Technical explainers are visual and interactive (diagrams, live examples,
  toggles, tables, and diffs explained beside the code), with few words. A new or updated page opens beside
  the thread (`open_pane`), and its link goes in the thread.
- **Pages for tasks:** a page for Pointman tasks starts its title with their refs (`AGC-16 · …`, or the project's
  key for several: `AGC · …`), links each task near the top (`pointman://item/AGC-16`, which opens it in the app), and
  is added to each task's links with `links_add` (core release 19), which keeps the task's other links, so the task
  shows it.
- **The CLI:** agents publish with `npx -y spacesheep@latest` (or `spacesheep` where it's installed): `deploy`
  for a new page or a new version of one, `read`, `list`, `share` and `whoami`. The extension's permission
  rules go into each run's allowed tools, so agents use the CLI without a prompt:
  `Bash(spacesheep:*)` and `Bash(npx -y spacesheep@latest:*)`.
- **Page links:** while the workspace runs Spacesheep, a page's link in a post
  (`https://spacesheep.dev/@<owner>/<space>`) shows in the apps as a pill with the page's name.

## Talk from a page

Spacesheep's Session tab messages the coding session that published a page: each deploy records the session that
ran it (`$CLAUDE_CODE_SESSION_ID`). The node knows each agent's session, so the extension listens for it with
Spacesheep's own CLI, as it is (1.26), and nothing goes into `~/.claude`: no hooks, no `machine on`, no
sessionpipe. The spec: https://spacesheep.dev/@james/spacesheep-talk-through-the-pointman-node (Pointman thread
6914).

- **One switch:** the account's own, "Talk to your sessions" at https://spacesheep.dev/settings/talk, read every
  minute (`talk status`). Nothing listens while it's off, and the extension never turns it on (`talk on`). It has
  no settings of its own (James, thread 6914 [16235]: no list of agents).
- **Who's listened for:** each agent here whose current session published a page. Every 5 minutes, while Talk is
  on, it reads the account's latest pages (`spacesheep list --json`) and, for each new or changed one, who pushed
  its versions (`versions <page> --json`: each version's `pushed_by.session_id`), kept in the state folder
  (`pages.json`). That list is the latest 50, so a page that leaves it is kept for 30 days from its last change,
  up to 500 pages: a burst of other pages (core's docs job updates dozens at once) doesn't silence it. Every 60 s it reads the node's agents (`agents.sessions`), and for each publisher with a thread
  it runs `spacesheep talk listen --session <id>` (the machine's `spacesheep`, else `npx -y spacesheep@latest`),
  signed in with the machine's own key in `~/.config/spacesheep`. So one process per agent that has a page, not
  one per agent: a node may run dozens. A listener that ends (its link gone, the CLI failing) starts again after
  5 s, doubling to 5 minutes. An agent with a new session, or Talk turned off, has its listener stopped. Each
  listener is a process group of its own, kept in the state folder (`listeners.json`), so a restart stops any left
  from a crash.
- **Each message** is a `board.post` in the thread of the publisher's latest run, as the machine. Its mention wakes
  the publisher:

  ````
  @scene from spacesheep.dev (Session tab, not signed):

  ```
  Can you make the header blue?
  ```

  Answer here, then send it to the page: `spacesheep talk reply --session 205b1fbd-… "…"`
  ````

  The words are in a code fence longer than any run of backticks in them, so a mention inside wakes nobody (the
  board reads no mention in code). The reply names the session listened for, which is the page's, whatever
  session the agent wakes in. Once the CLI's `talk reply` takes `--to` (its help, read every hour, says
  `[--to MESSAGE_ID]`), it also names the message's `id`: otherwise Spacesheep answers the session's newest
  message, and with comments waiting on two pages an answer lands under the other one. A message Spacesheep sends
  again (`redelivered`: a new listener gets back, for a day, what no `talk reply` answered) says so under the
  fence, since the agent may have answered it another way, and that a short answer sent with the post's line closes
  it. Without `--to` that line answers the newest message waiting, so the note says to use it only when this is the
  one.
- **Pages whose agent has finished** (despawned, or moved on to a new session): the extension remembers each
  agent's session and thread (`agents.json` in the state folder, pruned to sessions still behind a recent page) and
  keeps listening on the 10 such pages changed most recently. Only sessions an agent here once had count: every node
  reads the same account's pages, so another node's agents and the person's own sessions are left alone. A message
  there that calls agents reaches them, in the page's thread; one that calls nobody gets a reply on the page saying
  the agent has finished and how to ask another.
- **The header says where it was written, in fixed words:** "a comment on the page" when Spacesheep's `asked_in` says
  exactly that, else the Session tab. Nothing of Spacesheep's goes outside the fence.
- **Comments:** a comment on a page that mentions `@session` is Spacesheep's own way to ask the session that made
  the page: it arrives as a Talk message, and the agent's `talk reply` is posted under the comment. Only the page's
  owner can ask (Spacesheep's rule), so someone else's comment can't wake an agent. No sign-in beyond the machine's.
- **Calling an agent:** a message that starts with handles (`@editor cut the intro`, `@editor, @scene: …`, or
  `@pm.editor` / `@pointman.editor`) is for those agents instead, any agent on the board, in the same thread. In a comment write
  `@session @pm.editor …`: a bare `@editor` there is also a mention of a Spacesheep user called editor. Then: "@editor from spacesheep.dev (Session tab of
  `@scene`'s page, not signed):", the publisher in code so naming it doesn't wake it too. Its reply line still names the page's session, so @editor's answer shows on
  @scene's page. A handle later in the message stays in the fence. Spacesheep doesn't document the message's JSON: its words are `text`, else
  `message`, else `body`, else the whole JSON, and the first message's JSON goes in the log once.
- **Trust:** messages come from the account's signed-in spacesheep.dev session, not a passkey, so a stolen web
  login could message any agent while Talk is on. Each lands as a visible post saying where it's from, the agent's own
  permission rules still apply, and `SKILL.md` tells agents to trust such a post only when this machine posted it
  (anyone on the board can type its first line), and to ask in the thread before anything risky.

## Health

| Part | ok | not ok |
|---|---|---|
| `account`: the account's Talk, the one switch | `talk status` says `enabled` (read every minute, and after Spacesheep stops a listener) | stopped: Talk off for the account, or no Pro or Team plan; failing: the machine not signed in (`needs: auth`), or `talk status` failing |
| `listeners`: Talk listeners | listening for each agent here whose session published a page, with the last message's time | failing: one that keeps ending, the node not answering `agents.sessions`, the pages not readable, a post that didn't go out, or no CLI; stopped: Talk off, or no agent here has published a page in its current session |

The `spacesheep.talk` event says whether Talk is on, who's listened for, and when the last message came.

## Signing in

Once per machine, by a person: `npx -y spacesheep@latest login` in a terminal on it, or
`npx -y spacesheep@latest connect <key>` where there's no browser, with a key from
https://spacesheep.dev/settings/api-keys. The CLI keeps the sign-in in `~/.config/spacesheep`. An agent whose
`whoami` fails asks its person in the thread to sign the machine in, and never asks for a key in a post.

## Commands and settings

No commands: the guidance and the CLI's own commands are all agents use. No settings: the account's Talk switch is
the one switch.

## Files

| File | What |
|---|---|
| `extension.toml` | The manifest: `[run]` beside `[behavior]`, the permissions (`agents`, `post`, the CLI's rules) and how the apps draw page links |
| `SKILL.md` | The guidance, in the standard skill format (frontmatter, then the body), so it works as an ordinary skill outside Pointman too |
| `references/` | Shipped with the behaviour and handed to every run: `pages.md` (how to write a page people take in: 15 researched rules with sources), `check.py` (the checker agents run before each deploy; `python3 -m unittest` runs its tests) and `template.html` (a starter that passes it) |
| `src/main.rs` | The process: the looks, the listeners, the posts and health |
| `src/cli.rs`, `src/said.rs`, `src/pages.rs`, `src/words.rs` | Finding and stopping the CLI, reading what `talk listen` prints, who published each page, and the post's words (and whom a message calls) |
| `tests/` | Under the SDK's stand-in node with a fake `spacesheep` (`talk.rs`), the manifest (`manifest.rs`), and the real CLI signed out (`real_cli.rs`, when `SPACESHEEP_REAL_CLI` is npx's folder) |

## Development

`cargo test` runs it all with a fake CLI; nothing reaches spacesheep.dev or the board.
`SPACESHEEP_REAL_CLI=/opt/homebrew/bin cargo test` also runs Spacesheep's real CLI through npx, signed out.

## Releasing

Bump `version` in extension.toml and Cargo.toml (a test checks they match), commit, and push the tag
`v<version>`. `.github/workflows/release.yml` builds each platform (two Macs, two musl Linuxes) and makes the
release with every archive (extension.toml, bin/spacesheep, SKILL.md, README.md, LICENSE). The marketplace lists
it within 30 minutes, and `pointman upgrade` moves nodes onto it.

## Licence

MIT. See `LICENSE`.
