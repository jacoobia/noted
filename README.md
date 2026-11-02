# noted

A fetch CLI for coding agents. It keeps web pages and API responses in a local cache and returns HTML as compact markdown, so an agent can re-read the same page without another network request or another 100KB page landing in its context.

```
$ noted get https://oldschool.runescape.wiki/w/Abyssal_whip --ttl 7d --section "Combat stats"
noted: miss · age 0s · ttl 7d · 200 text/html; charset=UTF-8
## Combat stats

| [Equipment Stats](/w/Equipment_Stats#Attack_bonuses) [Attack bonuses](/w/Equipment_Stats#Attack_bonuses) |  |  |  |  |
| --- | --- | --- | --- | --- |
| [Stab](/w/Stab) | [Slash](/w/Slash) | [Crush](/w/Crush) | [Magic](/w/Magic) | [Ranged](/w/Ranged) |
| +0 | +82 | +0 | +0 | +0 |
...
```

The second call for that page comes from disk until the TTL runs out. After that, noted asks the server whether the page changed, which is usually a 304 and costs almost nothing.

## Install

macOS and Linux:

```
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/jacoobia/noted/releases/latest/download/noted-installer.sh | sh
```

Windows (PowerShell):

```
powershell -ExecutionPolicy Bypass -c "irm https://github.com/jacoobia/noted/releases/latest/download/noted-installer.ps1 | iex"
```

From source, with Rust 1.89 or newer:

```
cargo install --locked --git https://github.com/jacoobia/noted
```

## Usage

```
noted get <url> [--ttl <duration>] [--section <heading>] [--refresh] [--raw] [-q]
noted ls
noted rm <url>... | --stale | --all
noted stats
```

- `--ttl` takes durations like `5m`, `1h`, `7d` or `30d`. A new entry defaults to 1 day. A later call without `--ttl` keeps the TTL the entry was stored with, and `--ttl 0s` checks with the server every time.
- `--refresh` rechecks with the server even when the entry is still fresh.
- `--section` prints one heading and everything under it, up to the next heading at the same level or higher. Matching ignores case, and the first match wins. If nothing matches, noted exits 1 and lists the headings it found, which also makes it a cheap way to see a long page's outline.
- `--raw` prints HTML as the server sent it instead of converting it. Other content types (JSON included) already come back byte for byte, so `noted get <url> | jq .name` works as is.
- The body goes to stdout and a status line goes to stderr: `hit` (served from cache), `miss` (first fetch), `revalidated` (server said unchanged), `refreshed` (server sent a new copy) or `stale` (the server was unreachable, timed out or returned a 5xx, so you got the cached copy, with a warning line saying why). `-q` hides the status line but not the warning.
- Any other HTTP error (404 and so on) isn't cached: noted exits 1, prints `noted: HTTP <code>` on stderr and the server's error page on stdout.

### HTML to markdown

noted picks the article body (`.mw-parser-output` on MediaWiki sites, else `main`, `article` or `body`). It then drops scripts, navigation, images, tables of contents and navboxes. Icon-only links become their title text, so table labels survive. On the page above that's 18KB of markdown, against 98KB from a plain HTML-to-markdown conversion. Other content types come back byte for byte.

## Using it with agents

The CLI works with any agent that can run shell commands. Tell it to use `noted get` instead of curl, for example in `AGENTS.md`.

### Skill

`skill/SKILL.md` teaches an agent when to reach for noted and how to choose a TTL. For Claude Code:

```
mkdir -p ~/.claude/skills/noted
curl -fsSL https://raw.githubusercontent.com/jacoobia/noted/main/skill/SKILL.md -o ~/.claude/skills/noted/SKILL.md
```

### Claude Code hook

`noted hook` handles PreToolUse events. Add it to `~/.claude/settings.json`:

```json
{
  "hooks": {
    "PreToolUse": [
      { "matcher": "Bash|WebFetch", "hooks": [{ "type": "command", "command": "noted hook" }] }
    ]
  },
  "permissions": {
    "allow": ["Bash(noted get:*)"]
  }
}
```

What it does:

- A plain `curl <url>` becomes `noted get <url>`. A piped `curl <url> | jq ...` becomes `noted get --raw <url> | jq ...`. Only `-s`, `-S`, `-L`, `-f` and their long forms are accepted. Anything else is left alone: headers, a method, a body, `-o`, credentials or tokens in the URL (`user:pass@`, `?access_token=`, `?api_key=`, signatures), curl globs, local hosts (localhost, `.local`, private IPs), more than one URL, a path like `/usr/bin/curl`, and any shell substitution, redirection or `;`/`&&`/`||` before the first pipe.
- WebFetch is denied with a message pointing the agent at `noted get`. Set `NOTED_ALLOW_WEBFETCH=1` to switch that off.

Rewritten commands go through the normal permission check, which is why the snippet allows `noted get`. In auto mode, Claude Code may hold back a rewritten call once and ask the model to issue it again. The hook exits 0 even on bad input, and usage errors exit 64 rather than 2, so a bug in it can't block a tool call.

## Configuration

| Variable | Effect |
|---|---|
| `NOTED_DB` | Cache file. Defaults to `noted/cache.db` in the OS cache dir (`%LOCALAPPDATA%` on Windows, `~/Library/Caches` on macOS, `$XDG_CACHE_HOME` or `~/.cache` on Linux). |
| `NOTED_CONTACT` | Added to the User-Agent after the project URL. Some sites, including the OSRS Wiki, ask automated clients for contact details. |
| `NOTED_TIMEOUT` | Request timeout, e.g. `10s`. Defaults to 30s. |
| `NOTED_ALLOW_WEBFETCH` | `1` stops the hook from denying WebFetch. |

Several agents can share one cache at the same time: it's a single SQLite file in WAL mode.

## Security and privacy

- Cached pages are stored unencrypted in the cache file. `noted rm` overwrites removed pages on disk, and `noted rm --all` empties the cache.
- A cached page can carry text written to manipulate an agent, and it keeps serving that text until it expires. Treat fetched content as untrusted input, cached or not.
- noted only does GET. The hook won't rewrite requests with auth headers, credentials, or common token and signature parameters in the URL. If you run `noted get` yourself on a URL containing a secret, the URL and response are cached like any other.

## Limitations

- Pages that render with JavaScript come back nearly empty. Try `--raw` to see what the server sent.
- Non-UTF-8 pages are decoded lossily when converted to markdown. `--raw` keeps the original bytes.
- On Windows, writing binary output straight to a console window fails. Pipe it or redirect it to a file.
- Responses over 20MB are refused.

## Development

```
git config core.hooksPath .githooks
cargo test
```

The pre-commit hook runs `cargo fmt --check`, clippy with warnings as errors, and the tests against a copy of the staged tree.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option.
