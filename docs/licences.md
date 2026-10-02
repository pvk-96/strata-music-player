# Licence audit

Strata is GPL-3.0-or-later (see [LICENSE](../LICENSE)). Every dependency in the lock file has to
be compatible with that.

The figures below come from `cargo metadata` over the full lock file: **131 packages, one licence
expression each**. Run `cargo deny check` where `cargo-deny` is installed — `deny.toml` records
the same policy — or regenerate the table with:

```sh
cargo metadata --format-version 1 | python3 -c '
import collections, json, sys
packages = json.load(sys.stdin)["packages"]
counts = collections.Counter(p["license"] or "NONE" for p in packages)
print(f"{len(packages)} packages")
for licence, count in counts.most_common():
    print(f"{count:>4}  {licence}")'
```

| Count | Licence expression |
| ---: | --- |
| 74 | MIT OR Apache-2.0 |
| 35 | MIT |
| 5 | Apache-2.0 OR MIT |
| 4 | MIT/Apache-2.0 |
| 2 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT |
| 2 | Unlicense OR MIT |
| 2 | Zlib |
| 1 | 0BSD OR MIT OR Apache-2.0 |
| 1 | MIT OR Zlib OR Apache-2.0 |
| 1 | MPL-2.0 |
| 1 | MIT OR Apache-2.0 OR LGPL-2.1-or-later |
| 1 | Apache-2.0 WITH LLVM-exception |
| 1 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| 1 | GPL-3.0-or-later — `strata` itself |

Every expression resolves to a GPL-3.0-or-later compatible licence: MIT, Apache-2.0 (with or
without the LLVM exception), BSD, Zlib, the Unlicense and Unicode. Three entries need a word:

* **MPL-2.0** — `option-ext`, reached through `libsqlite3-sys`. Weak copyleft, file-level, and
  compatible with GPL-3 when used as a library, which is how it is used.
* **LGPL-2.1-or-later** — `r-efi`, a UEFI definitions crate reached through the GTK/winit stack.
  LGPL is designed to be linked from GPL programs.
* **GPL-3.0-or-later** — this project.

There is no proprietary licence, no `network` crate, and nothing that would require Strata to be
released as source beyond the GPL itself.
