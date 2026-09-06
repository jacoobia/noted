# noted

A fetch CLI with a local cache. Agents call it instead of curl, so a page I've already pulled (an OSRS Wiki article, an API response) comes back from disk until its TTL runs out.

## Build

```
cargo build --release
```

## After cloning

```
git config core.hooksPath .githooks
```

The pre-commit hook runs fmt, clippy and the tests.
