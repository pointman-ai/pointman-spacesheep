---
name: spacesheep
description: Publish documents, reports and explainers as Spacesheep pages with its CLI, and open them beside the thread. Use when writing anything longer than a post, or when asked to publish, share or update a page.
---

# Spacesheep pages

Spacesheep publishes pages and sites at a URL people open on any device.
- Write documents, reports and explainers as Spacesheep pages, not Markdown files or long posts, unless the person asks for another format.
- Make technical explainers visual and interactive: show rather than tell (diagrams, live examples, toggles, tables), with few and short words. When code changes are involved, include interactive diffs that show each change and explain it beside the code.
- When you create or update a page, open it beside the thread for the person you're working for: `open_pane(page=<its https:// link>)`. Then post the link in the thread.

**The CLI:** `npx -y spacesheep@latest <command>` (or `spacesheep <command>` where it's installed; `--help` lists them all).
- A new page, private: `deploy page.html --title "…" --emoji 📄 --description "one line" --json` (one `.html` file, or a folder with an `index.html`). The answer has its `url` and `uuid`.
- Updating one: `deploy page.html --space <uuid or url> -m "what changed" --json`. `--new` makes a new page from a folder that has one already.
- `read <space> [path]` prints a page's files, `list` lists yours, and `share <space> --visibility signed_in|members|public` shares one.
- `whoami` checks this machine's sign-in. If it fails, ask your person in the thread to sign the machine in themselves: `npx -y spacesheep@latest login` in a terminal on it, or `npx -y spacesheep@latest connect <key>` where there's no browser (a key from spacesheep.dev/settings/api-keys, never posted).
