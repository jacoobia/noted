# AGENTS.md

Rules for coding agents working on noted. They apply to every change. `CONTRIBUTING.md` explains them for people. When the two disagree, this file wins for agents.

## Project

noted is a Rust CLI that fetches URLs through a SQLite cache and returns HTML as markdown. It is one binary with no daemon. The module map is in `CONTRIBUTING.md` under "Where things live".

## Every change

1. **Branch** from `main` as `feat/short-name`, `fix/short-name` or `chore/short-name`.
2. **Write the test first** and run it. It must fail for the reason you expect before you touch the code. Bug fixes start with a test that reproduces the bug. Integration tests use `tests/common/mod.rs` and a local test server, never the real network.
3. **Make the smallest change** that turns the test green. Reuse what is already in the module before adding code. Stay within what the task asks for.
4. **Run the gates** and read their output:
   ```
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   ```
   A change is done only when all three pass. Report failures as they are.
5. **Update the docs** when flags, output or behaviour change: `README.md` for people, `skill/SKILL.md` for agents.
6. **Commit** one logical change at a time, each small enough to review in a minute or two.

## Commit messages

- Subject: `feat: …`, `fix: …` or `chore: …`, then a lowercase summary of about ten words. Names like SQLite or Claude Code keep their case.
- Body: only when the reason isn't obvious from the subject.
- **Every commit you author or substantially shape ends with a co-author trailer naming you**, for example `Co-Authored-By: Claude <noreply@anthropic.com>` or the equivalent for your tool. The maintainers want to know which contributions came from AI.

## Code

- Code explains itself through names, small functions and plain control flow.
- Comments are rare. Write one only for a reason the code can't express, such as a dependency's quirk or an external constraint. Doc comments go on public items whose name doesn't make the usage clear.
- Clippy runs with `pedantic` and `-D warnings`. Fix the lint; allow a lint only when the fix would make the code worse, and say why in the commit body.
- Add dependencies with `cargo add`, only when a few lines of code won't do. The MSRV is Rust 1.89.
- All SQL stays in `src/cache.rs`. All stdout writes go through `output::emit`, which handles closed pipes.

## The hook

`src/hook.rs` rewrites shell commands an agent is about to run, so treat it as security-sensitive:

- When a command is ambiguous, return no rewrite.
- Every new rule, and every new reason to refuse a rewrite, gets its own test in the `each_guard_rejects_on_its_own` or `rewrites_simple_curl` tables.
- The hook must exit 0 on every input, and usage errors exit 64, never 2.

## Prose

README, skill, `--help` text and commit bodies are read by people. Write plainly and specifically. Leave out marketing words and em dashes.
