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

That prints the heading and everything under it, up to the next heading at the same level or higher. The match ignores case. If nothing matches, it exits 1 and lists the headings it found. It works on HTML (after the markdown conversion) and on plain text.

The body goes to stdout. A status line goes to stderr, e.g. `noted: hit · age 3h · ttl 7d · 200 text/html`. `-q` hides it.

The cache lives in my OS cache dir (`%LOCALAPPDATA%\noted\cache.db` on Windows). Set `NOTED_DB` to use another file. Set `NOTED_CONTACT` to add contact details to the User-Agent, which the OSRS Wiki asks for.

## After cloning

```
git config core.hooksPath .githooks
```

The pre-commit hook runs fmt, clippy and the tests against what's staged.
