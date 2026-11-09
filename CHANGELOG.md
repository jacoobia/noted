# Changelog

## 0.1.0 - 2026-11-09

First release.

- `noted get` caches pages and API responses for a TTL, then revalidates them with ETag and Last-Modified.
- HTML comes back as compact markdown. `--section`, `--outline` and `--plain` cut it down further, and `--raw` prints the original.
- When the server is unreachable, times out or returns a 5xx, you get the stale copy with a warning.
- Requests need a User-Agent that names the agent, saved once with `noted config user-agent`.
- robots.txt is checked before every request, including each redirect hop.
- Requests to a host are spaced out by its robots.txt Crawl-delay or by `noted config rate`, across every agent sharing the cache.
- `noted ls`, `noted rm` and `noted stats` manage the cache.
- `noted hook` rewrites plain curl calls in Claude Code to `noted get` and turns WebFetch away.
- `skill/SKILL.md` teaches agents when to use noted and how to pick a TTL.
