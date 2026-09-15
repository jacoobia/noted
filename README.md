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

The body goes to stdout. A status line goes to stderr, e.g. `noted: hit · age 3h · ttl 7d · 200 text/html`. `-q` hides it.

The cache lives in my OS cache dir (`%LOCALAPPDATA%\noted\cache.db` on Windows). Set `NOTED_DB` to use another file. Set `NOTED_CONTACT` to add contact details to the User-Agent, which the OSRS Wiki asks for.

## After cloning

```
git config core.hooksPath .githooks
```

The pre-commit hook runs fmt, clippy and the tests against what's staged.
