# Contributing to noted

Bug reports, fixes and features are all welcome. For anything bigger than a small fix, open an issue first so we can agree on the approach before you spend time on it.

## Setup

You need Rust 1.89 or newer.

```
git clone https://github.com/jacoobia/noted
cd noted
git config core.hooksPath .githooks
cargo test
```

The pre-commit hook checks a copy of what you've staged: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` (with `clippy::pedantic` on), and `cargo test`. A commit that fails any of them is rejected. CI runs the same checks on Linux, macOS and Windows.

## Where things live

| File | What it does |
|---|---|
| `src/main.rs` | Argument parsing and dispatch |
| `src/lib.rs` | The module list |
| `src/get.rs` | The `get` flow: fresh, stale, miss, revalidate, fallback |
| `src/cache.rs` | The SQLite store. All SQL lives here. |
| `src/config.rs` | The saved User-Agent and `NOTED_USER_AGENT` |
| `src/robots.rs` | robots.txt parsing and matching (RFC 9309) |
| `src/fetch.rs` | HTTP requests and timeouts |
| `src/view.rs` | HTML to markdown, `--section` |
| `src/manage.rs` | `ls`, `rm`, `stats` |
| `src/hook.rs` | The Claude Code hook and curl rewriting |
| `src/output.rs` | Writing to stdout, size and duration formatting |
| `skill/SKILL.md` | The agent skill |
| `tests/` | Integration tests, run against a local test server |
| `tests/fixtures/` | Saved pages the HTML conversion tests read |

## Standards

**Tests come first.** Write a test that fails without your change, watch it fail, then make it pass. Bug fixes start with a test that reproduces the bug. Integration tests use the helpers in `tests/common/mod.rs` and never touch the real network.

**Code explains itself.** Use clear names, small functions and plain control flow. Comments are for a reason the code can't show, like a workaround for a dependency's behaviour. Doc comments go on public items only when the name doesn't make the usage clear.

**Dependencies are added with `cargo add`** and only when a few lines of code won't do. The minimum supported Rust version is 1.89. CI checks it.

**The hook is security-sensitive.** `src/hook.rs` rewrites shell commands an agent is about to run. When a command is ambiguous, leave it alone. Every new rewrite rule needs a test, and so does every new reason to refuse one.

**User-facing changes update the docs.** If a change alters flags, output or behaviour, update `README.md`, and `skill/SKILL.md` too if agents need to know. Keep prose plain and specific: no marketing words, no em dashes.

## Branches and commits

- Branch names: `feat/short-name`, `fix/short-name` or `chore/short-name`.
- Commit subjects: `feat: …`, `fix: …` or `chore: …` followed by a lowercase summary of about ten words, for example `fix: match --section against link text in linked headings`. Add a body when the reason for the change isn't obvious.
- Keep each commit to one logical change, small enough to review in a minute or two.

## Using AI tools

AI-assisted contributions are welcome. If an AI tool wrote or substantially shaped a commit, please mention it. A trailer naming the tool is the easiest way:

```
Co-Authored-By: <tool or model name> <email or noreply address>
```

You're still responsible for the change. Read it, run it, and make sure the tests prove what you say they prove. Coding agents working in this repo should follow [AGENTS.md](AGENTS.md).

## Pull requests

Before opening one:

- [ ] `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass
- [ ] New behaviour has tests, and bug fixes have a test that failed before the fix
- [ ] README and skill are updated if anything user-facing changed
- [ ] Commits follow the format above

Describe what changed and why, and link the issue if there is one.

## Licence

Unless you say otherwise, anything you contribute is dual licensed under MIT or Apache-2.0, the same as the rest of the project.
