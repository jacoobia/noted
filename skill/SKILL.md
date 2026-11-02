---
name: noted
description: Fetch any URL (web page, wiki article, docs, JSON API) through the local noted cache instead of curl or WebFetch. Use whenever you read something from the web, re-read a page fetched earlier, or need one section of a long page.
---

# noted

`noted get` fetches a URL once, keeps it in a local cache, and serves it from disk until its TTL runs out. HTML comes back as compact markdown. Repeat reads cost no network request, so fetch again instead of copying pages into notes.

## Reading a page

```
noted get <url> --ttl <duration>
noted get <url> --section "<heading>"
```

- Pick `--ttl` from how often the source changes (table below). It is stored with the entry, so later reads can leave it out.
- Use `--section` for long pages: it prints one heading and everything under it. When the heading is missing, noted exits 1 and lists the headings it found on stderr. Retry with one of those. To see a page's outline without reading it, ask for a heading that doesn't exist.
- Pipe JSON straight into tools: `noted get <url> | jq .field`. Only HTML is converted, everything else comes back byte for byte. Add `--raw` when you need the original HTML.
- stderr carries one status line, e.g. `noted: hit · age 3h · ttl 7d · 200 text/html`. `miss` is a first fetch, `hit` means it came from the cache, `refreshed` means the server sent a new copy, `revalidated` means the server confirmed nothing changed, and `stale` means the server was unreachable, timed out or returned a 5xx, so you got the cached copy.
- Exit 1 with `noted: HTTP <code>` on stderr means stdout holds the server's error page, not the content you asked for.
- If the markdown comes back empty, the page probably renders with JavaScript. Check `--raw` to see what the server sent.

## Choosing a TTL

| Source | TTL |
|---|---|
| Reference that rarely changes (old docs, specs, historical wiki pages) | `30d` |
| Wikis and docs in general | `7d` |
| Something the user says was just updated (a new game update, a release this week) | `1h` to `1d` |
| Live APIs (prices, status, scores) | `5m` or less |
| Must be current right now | `0s` |

If a hook is installed, plain `curl <url>` calls get rewritten to `noted get` with the default TTL (1 day). For live data, call `noted get --ttl 5m <url>` yourself. The hook never rewrites calls to local dev servers (localhost, private IPs), so keep using curl for those.

When the user says the data changed, or the cached copy looks wrong, add `--refresh`. noted rechecks with the server and keeps the cached copy if it's unchanged.

## Housekeeping

- `noted ls` lists entries: age, TTL, fresh/stale, size, hits, URL.
- `noted rm <url>` drops one entry. `noted rm --stale` clears expired ones.
- `noted get --help` has the full flag list.

If `noted` isn't on PATH, tell the user it isn't installed and point them at https://github.com/jacoobia/noted#install. Fall back to curl for now.
