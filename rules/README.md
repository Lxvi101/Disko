# Community rules

`community.json` is the rule pack every Disko install uses (unless the user turns it off). Disko
ships a copy and fetches the latest version of this file from `main` at most once a day.

## Format

```json
{
  "schema_version": 1,
  "updated": "YYYY-MM-DD",
  "rules": [
    {
      "id": "gradle-caches",               // unique, a-z 0-9 - _ .
      "pattern": "~/.gradle/caches",        // ~/ or / rooted glob: * and ? within a name, ** for any depth
      "category": "packages",              // build | cache | packages | developer | leftovers | old-versions |
                                           // installers | logs | agent-data | containers | extras | trash
      "title": "Gradle caches",            // short, human
      "why": "One sentence of evidence.",
      "regrows": true,                     // comes back by itself
      "safety": "safe"                     // safe = deleted in one tap / by routines; review = moved to the Trash
    }
  ]
}
```

Rules are data, never code. Disko drops any rule that has a command-like field (`action`,
`command`, `argv`, …), targets a protected path (home, Documents, Keychains, `.ssh`, `/System` …),
is too broad (`~/*`, `/**`, `~/Library/Caches/*`, `**` without an exact last name), or doesn't
parse. Bump `updated` with every change: Disko prefers the newer of its bundled copy and the
fetched one.

## How submissions get in

When a user (or their agent, through Disko's MCP server) teaches Disko a new kind of clutter and
marks it as shared, Disko queues an **anonymized** submission: the pattern is generalized (home →
`~`, user names, project names and IDs → `*`/`**`), the size is a bucket (`<100 MB`, `100 MB–1 GB`,
`1–10 GB`, `>10 GB`), and title/why are scrubbed of paths and names. No machine identifiers, no
other paths. Queued submissions are visible in Disko before they're sent.

A maintainer reviews submissions and merges good ones here with a pull request:

1. Is it generic (useful on other Macs, no personal names left)?
2. Is it really rebuildable or abandoned? `safe` only for data that regrows or that the owning
   tool recreates; everything else is `review`.
3. Is there an owning tool command that should be used instead (`brew cleanup`, `xcrun simctl`)?
   Then it belongs in Disko's built-in rules, not here.
4. `cargo test -p disko-memory` must pass (it validates this file).
