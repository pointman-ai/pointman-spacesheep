# spacesheep

A Pointman behaviour that has agents write documents, reports and explainers as
[Spacesheep](https://spacesheep.dev) pages, published with Spacesheep's CLI and opened beside the thread for
the person they're working for. Spacesheep publishes pages and sites at a URL people open on any device.

```
pointman add spacesheep
```

## How it works

- **A behaviour:** guidance, with no process. Installed on a node, `SKILL.md`'s body goes into every agent run
  there, whichever agent it is and whether it runs on Claude Code, Codex or Gemini CLI.
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

## Signing in

Once per machine, by a person: `npx -y spacesheep@latest login` in a terminal on it, or
`npx -y spacesheep@latest connect <key>` where there's no browser, with a key from
https://spacesheep.dev/settings/api-keys. The CLI keeps the sign-in in `~/.config/spacesheep`. An agent whose
`whoami` fails asks its person in the thread to sign the machine in, and never asks for a key in a post.

## Commands and settings

None. The guidance and the CLI's own commands are all it has.

## Files

| File | What |
|---|---|
| `extension.toml` | The manifest: `[behavior]`, the permission rules and how the apps draw page links |
| `SKILL.md` | The guidance, in the standard skill format (frontmatter, then the body), so it works as an ordinary skill outside Pointman too |

## Releasing

Bump `version` in extension.toml, commit, and make a GitHub release (not a draft or prerelease) tagged
`v<version>`. There's nothing to build, so it has no assets. The marketplace lists it within 30 minutes, and
`pointman upgrade` moves nodes onto it.

## Licence

MIT. See `LICENSE`.
