# kdb

A CLI and language server for a markdown workspace. It checks the links between your notes and tracks projects and tasks in a SQLite file in the same folder.

```
cargo install --path .
kdb init
```

## Notes

```
$ kdb check
notes/index.md:5:3 broken link roadmap.md (target file not found: notes/roadmap.md)
1 orphan file (run `kdb check --orphans` to list)
1 error
1 warning

$ kdb outline notes/architecture.md
# Architecture  L1
## Overview     L3
## Components   L7

$ kdb refs notes/architecture.md#components
notes/index.md:3:3  architecture.md#components

$ kdb render SOP/release.md        # resolves ![[file#heading]] embeds
```

`kdb lsp` shows the same broken links in your editor.

## Tasks

```
$ kdb projects add hermaeus --alias HRM --path projects/hermaeus
$ kdb tasks add -P hermaeus "Deploy Archil mount" -p 1
added task HRM-0001
$ kdb tasks add -P hermaeus "Entity resolver backfill" --after HRM-0001
added task HRM-0002
$ kdb tasks edit HRM-0001 --status in_progress

$ kdb tasks list -P hermaeus
id        st    p  title
HRM-0001  [~]  1  Deploy Archil mount
HRM-0002  [ ]  3  Entity resolver backfill  (blocked by HRM-0001)

$ kdb tasks done HRM-0001
released: HRM-0002
HRM-0001 -> done

$ kdb render -P hermaeus           # writes projects/hermaeus/.tasks/index.md
```

Full command list: [docs/commands.md](docs/commands.md).

## License

MIT, Humanitas Labs.
