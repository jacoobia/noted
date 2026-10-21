# noted

A fetch CLI with a local cache. Agents call it instead of curl, so a page I've already pulled (an OSRS Wiki article, an API response) comes back from disk until its TTL runs out.

## Build

```
cargo build --release
```

## Usage

```
noted get https://oldschool.runescape.wiki/w/Abyssal_whip --ttl 7d
```

The first call fetches and stores the page. Calls after that come from the cache until the TTL runs out. Without `--ttl`, a new entry gets 1 day and an existing one keeps the TTL it was stored with.

Once an entry goes stale, noted asks the server whether it changed (ETag / Last-Modified). A `304` just resets the clock, so an expired page costs a tiny request instead of a full download. `--ttl 0s` checks every time, and `--refresh` rechecks with the server even when the entry is still fresh. If the server is down, returns a 5xx, or takes longer than 30s (`NOTED_TIMEOUT` changes that), I get the stale copy with a warning. Other errors (404 and so on) aren't cached and exit 1.

HTML comes back as markdown. noted keeps the article body (`.mw-parser-output` on wikis, else `main`, `article` or `body`), drops scripts, nav, images, the table of contents and navboxes, and squashes table padding. Icon-only links turn into their title text, so column labels like Stab/Slash survive. On the whip page that's 18KB of markdown against 98KB from a plain conversion. `--raw` gives the original HTML. Other content types come back byte for byte.

To read one part of a page:

```
noted get https://oldschool.runescape.wiki/w/Abyssal_demon --section Drops
```

That prints the heading and everything under it, up to the next heading at the same level or higher. The match ignores case, and the first matching heading wins. If nothing matches, it exits 1 and lists the headings it found. It works on HTML (after the markdown conversion) and on any other text response.

The body goes to stdout. A status line goes to stderr, e.g. `noted: hit · age 3h · ttl 7d · 200 text/html`. `-q` hides it.

The cache lives in my OS cache dir (`%LOCALAPPDATA%\noted\cache.db` on Windows). Set `NOTED_DB` to use another file. Set `NOTED_CONTACT` to add contact details to the User-Agent, which the OSRS Wiki asks for.

## Looking after the cache

```
noted ls                 # age, ttl, fresh/stale, size, hits, url (newest first)
noted rm <url>...        # drop specific entries
noted rm --stale         # drop everything past its TTL
noted rm --all
noted stats              # totals and the DB path
```

## Claude Code hook

`noted hook` reads a PreToolUse event on stdin and does two things:

- A plain `curl <url>` (only `-s -S -L -f` and their long forms, one http(s) URL, optionally piped into something) becomes `noted get <url>`. When it's piped, it becomes `noted get --raw <url> | ...` so the next command still gets the original bytes, and the status line on stderr still shows how old the data is. Local hosts (localhost, `.local`, private and loopback IPs) are left alone so a dev server is never cached. Anything with headers, a method, a body, `-o`, credentials in the URL, curl globs (`[1-3]`, `{a,b}`), output redirection (`>`) or `;`/`&&` before the pipe is left alone.
- WebFetch is denied with a message telling the agent to use `noted get` instead. `NOTED_ALLOW_WEBFETCH=1` turns that off.

The rewritten command still goes through the normal permission prompt. It always exits 0, so a bug in it never blocks a tool call. Bad arguments exit 64 instead of clap's usual 2, because Claude Code reads a 2 from a hook as "block this call".

Setup: `cargo install --path .`, then add this to `~/.claude/settings.json`:

```json
{
  "hooks": {
    "PreToolUse": [
      { "matcher": "Bash|WebFetch", "hooks": [{ "type": "command", "command": "noted hook" }] }
    ]
  }
}
```

## Agent skill

`skill/SKILL.md` tells an agent when to use noted and how to pick a TTL. To install it for Claude Code:

```
cp -r skill ~/.claude/skills/noted
```

The hook covers agents that reach for curl or WebFetch by habit. The skill covers picking the TTL and using `--section`.

## After cloning

```
git config core.hooksPath .githooks
```

The pre-commit hook runs fmt, clippy and the tests against what's staged.
