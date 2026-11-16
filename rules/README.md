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
      "pattern": "~/.gradle/caches",        // ~/ rooted glob: * and ? within a name, ** for any depth
      "category": "packages",              // build | cache | packages | developer | leftovers | old-versions |
                                           // installers | logs | agent-data | containers | extras | trash
      "title": "Gradle caches",            // short, human
      "why": "One sentence of evidence.",
      "regrows": true,                     // comes back by itself
      "safety": "safe"                     // the maintainer's assessment; Disko always treats community
                                           // rules as review (moved to the Trash, never by routines)
    }
  ]
}
```

Rules are data, never code. Disko drops any rule that has a command-like field (`action`,
`command`, `argv`, …), points outside the home folder, targets a protected path (home, Documents,
`/System` …), a `.git` folder, anything in `~/Library` other than `~/Library/Caches/<app>/…` and
`~/Library/Logs/<app>/…`, or credentials (`.ssh`, `.aws`, `.kube`, `.gnupg`, `.config/gh`,
`.docker/config.json`, keychains …), is too broad (`~/*`, `~/Library/Caches/*`, a `**` rule whose
last part isn't a well-known build/cache folder like `node_modules`, `target`, `.dart_tool`), or
doesn't parse. All checks ignore case, like APFS.

Community rules are always **review** items, whatever `safety` says: their findings go to the
Trash (Undo-able) and never take part in routines, so a bad rule in the pack can't delete anything
for good. Only rules a user makes themselves can be `safe`.

Bump `updated` with every change: Disko prefers the newer of its bundled copy and the fetched one.

## How submissions get in

When a user (or their agent, through Disko's MCP server) teaches Disko a new kind of clutter and
marks it as shared, Disko queues an **anonymized** submission: the pattern is generalized (home →
`~`, user names, project names and IDs → `*`/`**`), the size is a bucket (`<100 MB`, `100 MB–1 GB`,
`1–10 GB`, `>10 GB`), and title/why are scrubbed of paths and names. No machine identifiers, no
other paths. Queued submissions are visible in Disko before they're sent.

A maintainer reviews submissions and merges good ones here with a pull request:

1. Is it generic (useful on other Macs, no personal names left)?
2. Is it really rebuildable or abandoned? `safe` only for data that regrows or that the owning
   tool recreates; everything else is `review`. (Disko treats every community rule as `review`
   either way; the field documents the maintainer's judgement.)
3. Is there an owning tool command that should be used instead (`brew cleanup`, `xcrun simctl`)?
   Then it belongs in Disko's built-in rules, not here.
4. `cargo test -p disko-memory` must pass (it validates this file).
