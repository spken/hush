# Decisions

| Date | Decision | Reason |
| --- | --- | --- |
| 2026-11-07 | Slint `=1.18.1` instead of the plan's 1.17.x; workspace `rust-version = "1.92"` | 1.18.1 is the current stable patch (Sep 21); it requires Rust 1.92, above AGENTS.md's 1.85 floor. |
| 2026-11-07 | Software renderer by default; FemtoVG stays behind the `ui/femtovg` feature | Spike: FemtoVG costs ~43 MB more private working set for a modest CPU saving. Numbers in `measurements.md`. Confirm in M3 with the real UI. |
