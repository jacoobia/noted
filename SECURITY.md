# Security

Report security problems privately at https://github.com/jacoobia/noted/security/advisories/new, not in a public issue.

Include the output of `noted --version`, your OS, and the command or input that shows the problem. For a hook bug, include the PreToolUse JSON the hook was given.

These count as security problems:

- the hook rewriting a curl command it should leave alone, such as one with credentials, a token in the URL or a local host
- a way to put extra headers into a request through the User-Agent or a URL
- a request that skips the robots.txt check
- a cached page served for a URL it doesn't belong to

Fetched pages containing text written to steer an agent aren't a noted bug. The README covers this under Security and privacy: treat fetched content as untrusted.

Fixes go into the latest release only.
