# spacesheep

A Pointman extension that has agents write documents, reports and explainers as
[Spacesheep](https://spacesheep.dev) pages, published with Spacesheep's CLI and opened beside the thread for
the person they're working for. Spacesheep publishes pages and sites at a URL people open on any device. With
**Talk** on, a message typed in a page's Session tab reaches the agent that published it, as a post in its
thread, and the agent answers there and on the page.

```
pointman add spacesheep
```

## How it works

- **A behaviour and a small process:** installed on a node, `SKILL.md`'s body goes into every agent run there,
  whichever agent it is and whether it runs on Claude Code, Codex or Gemini CLI. The process (`bin/spacesheep`,
  Rust) is only for Talk, and does nothing until Talk is switched on.
- **What agents do:** anything longer than a post becomes a page, not a Markdown file or a long post, unless
  the person asks for another format. Technical explainers are visual and interactive (diagrams, live examples,
  toggles, tables, and diffs explained beside the code), with few words. A new or updated page opens beside
  the thread (`open_pane`), and its link goes in the thread.
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

- **Two switches,** both off by default: `talk` in this machine's settings for the extension
  (`pointman config set node.extensions.spacesheep.talk true`), and `spacesheep_talk = true` for each agent in
  wake.toml. The account's own Talk switch, at https://spacesheep.dev/settings/talk, has to be on too; the
  extension never turns it on (`talk on`).
- **Listening:** every 60 s it reads the node's agents (`agents.sessions`). For each with Talk on, a session and a
  thread, it runs `spacesheep talk listen --session <id>` (the machine's `spacesheep`, else
  `npx -y spacesheep@latest`), signed in with the machine's own key in `~/.config/spacesheep`. A listener that
  ends (its link gone, Talk off for the account, the CLI failing) starts again after 5 s, doubling to 5 minutes.
  An agent switched off, or with a new session, has its listener stopped. Each listener is a process group of
  its own, kept in the state folder (`listeners.json`), so a restart stops any left from a crash.
- **Each message** is a `board.post` in the thread of the agent's latest run, as the machine. Its mention wakes
  the agent:

  ````
  @scene from spacesheep.dev (Session tab, not signed):

  ```
  Can you make the header blue?
  ```

  Answer here, then send it to the page: `spacesheep talk reply --session 205b1fbd-… "…"`
  ````

  The words are in a code fence longer than any run of backticks in them, so a mention inside wakes nobody (the
  board reads no mention in code). The reply names the session listened for, which is the page's, whatever
  session the agent wakes in. Spacesheep doesn't document the message's JSON: its words are `text`, else
  `message`, else `body`, else the whole JSON, and the first message's JSON goes in the log once.
- **Trust:** messages come from the account's signed-in spacesheep.dev session, not a passkey, so a stolen web
  login could message a switched-on agent. Each lands as a visible post saying where it's from, the agent's own
  permission rules still apply, and `SKILL.md` tells agents to ask in the thread before anything risky.

## Health

| Part | ok | not ok |
|---|---|---|
| `node`: Talk on this node | the `talk` setting is on | stopped: it's off |
| `account`: the account's Talk | `talk status` says `enabled` (read every 10 minutes, and after Spacesheep stops a listener) | failing: Talk off for the account, no Pro or Team plan, or the machine not signed in (`needs: auth`) |
| `listeners`: Talk listeners | listening for each switched-on agent, with the last message's time | failing: one that keeps ending, the node not answering `agents.sessions`, a post that didn't go out, or no CLI; stopped: nobody to listen for |

The `spacesheep.talk` event says whether Talk is on, who's listened for, and when the last message came.

## Signing in

Once per machine, by a person: `npx -y spacesheep@latest login` in a terminal on it, or
`npx -y spacesheep@latest connect <key>` where there's no browser, with a key from
https://spacesheep.dev/settings/api-keys. The CLI keeps the sign-in in `~/.config/spacesheep`. An agent whose
`whoami` fails asks its person in the thread to sign the machine in, and never asks for a key in a post.

## Commands and settings

No commands: the guidance and the CLI's own commands are all agents use. One setting, `talk` (off), per machine.

## Files

| File | What |
|---|---|
| `extension.toml` | The manifest: `[run]` beside `[behavior]`, the permissions (`agents`, `post`, the CLI's rules), the `talk` setting and how the apps draw page links |
| `SKILL.md` | The guidance, in the standard skill format (frontmatter, then the body), so it works as an ordinary skill outside Pointman too |
| `src/main.rs` | The process: the looks, the listeners, the posts and health |
| `src/cli.rs`, `src/said.rs`, `src/words.rs` | Finding and stopping the CLI, reading what `talk listen` prints, and the post's words |
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
