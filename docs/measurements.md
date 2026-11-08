# Measurements

Memory is the Task Manager *Memory* column (private working set). CPU is the share of one
logical core. All builds use the release profile.

## Phase 0 — Renderer spike (Nov 7, 2026)

`crates/ui/examples/renderer_spike.rs`: 5,000-row `ListView`, each row a 96 px image and
two text lines. `--autoscroll` scrolls at ~3,400 px/s and wraps at the end, a repeatable
stand-in for a sustained flick-scroll.

```bash
cargo build --release -p ui --example renderer_spike
cargo build --release -p ui --example renderer_spike --features femtovg
```

Run with `SLINT_BACKEND=winit-software` or `SLINT_BACKEND=winit-femtovg`, and
`SLINT_DEBUG_PERFORMANCE=refresh_lazy,console` to print frames per second.
Each figure: 5 s warm-up, then a 10 s sample.

Machine: i7-12700K, RTX 3070, Windows 11 26200, Rust 1.99.1, Slint 1.18.1. Measured on
the primary display (1920×1080 at 239 Hz); Hush targets the secondary 144 Hz display,
where fewer frames are drawn, so these CPU figures are an upper bound.

| Renderer | Window | State | Private WS (MB) | Working set (MB) | CPU (% of 1 core) | FPS |
| --- | --- | --- | --- | --- | --- | --- |
| software | 1280×800 | idle | 8.6 | 23.7 | 0 | 0 |
| software | 1280×800 | autoscroll | 8.6 | 23.7 | 11.1 | 187–197 |
| software | maximized | idle | 12.0 | 27.3 | 0.2 | 0 |
| software | maximized | autoscroll | 12.1 | 27.4 | 29.5 | 164–198 |
| FemtoVG | 1280×800 | idle | 55.0 | 86.6 | 0 | 0 |
| FemtoVG | 1280×800 | autoscroll | 54.8 | 86.3 | 14.7 | 195–197 |
| FemtoVG | maximized | idle | 44.4 | 76.0 | 0.2 | 0 |
| FemtoVG | maximized | autoscroll | 55.4 | 87.0 | 11.1 | 196–205 |

**Result:** software renderer. It costs ~43 MB less than FemtoVG (more than the plan's
15–30 MB estimate). Maximized, it renders ~180 fps — more frames than a 144 Hz display
shows — at 29.5% of one core, just over the 25% bar under a sustained scroll that a real
flick only reaches briefly. Re-check with the real UI in M3.

## M2 — Connect device and audio chain (Nov 7, 2026)

`cargo build --release -p playback --example m2`. Idle is 12 s after the device
registered; playing is read from Task Manager during the exit test.

| State | Private WS (MB) | Working set (MB) | Web API requests |
| --- | --- | --- | --- |
| registered, idle | 4.2 | 21.1 | 0 |
| playing | ~10 | — | 0 |

## M3 — UI shell (Nov 7, 2026)

`cargo build --release -p app`, software renderer, the art-led UI at 1280×800 and
maximized (1920×1080). "Bar animating" has a track loaded and the progress bar gliding
between its 250 ms ticks; it was driven by sample data, not real playback, so the audio
chain is not included (M2 measured it at ~6 MB more). CPU is averaged over 10 s.

| State | Window | Private WS (MB) | Working set (MB) | CPU (% of 1 core) | Web API requests |
| --- | --- | --- | --- | --- | --- |
| connected, idle | 1280×800 | 9.1 | 30.8 | 0.0 | 0 |
| connected, idle | maximized | 12.6 | 34.2 | 0.0 | 0 |
| bar animating | 1280×800 | 9.5 | 31.1 | 3.6 | 0 |
| bar animating | maximized | 12.8 | 34.4 | 3.9 | 0 |

**Result:** the software renderer stays. The real UI costs far less than the 70 MB budget,
and the continuously animated progress bar costs under 4% of one core. The 10-minute
playback exit test (driven from a phone) is still to be run by hand.

## M4 — Gateway and cache (Nov 7, 2026)

`cargo build --release -p app`, 1280×800, read 15 s after launch with the device
registered and nothing playing. The only Web API call is `/me` at startup (24 h TTL).

| Launch | Private WS (MB) | Working set (MB) | Web API requests |
| --- | --- | --- | --- |
| first (empty `http_cache`) | 9.7 | 32.6 | 1 |
| second (warm cache) | 9.4 | 32.3 | 0 |

`/me` returns an `ETag`. `cargo run -p webapi --example m4` passes all five exit checks
against a `wiremock` server: 50 concurrent identical GETs send 1 request, `Retry-After: 5`
holds every lane for at least 5 s, `QUOTA_EXCEEDED` opens the breaker and fails a queued
P3 request without sending it, a 401 sends exactly one retry with a refreshed token, and a
cancelled queued request is never sent.

## M5 — Library (Nov 8, 2026)

`cargo build --release -p app`, 1280×800, Now Playing page in front. Library: 4,383 Liked
Songs, 4 saved albums, 85 playlists (34 owned). Requests are the gateway's per-minute log;
memory is Task Manager's private working set and working set (`Get-Process`).

| Run | Web API requests | Private WS (MB) | Working set (MB) |
| --- | --- | --- | --- |
| first sync, empty tables (~6 min, no 429s) | 231: 88 `/me/tracks`, 1 `/me/albums`, 2 `/me/playlists`, 140 `/playlists/{id}/items` | 20.6 (at the end) | 45.9 |
| second launch, synced minutes before | 0 | 12.8 | 35.6 |
| launch with the sync due, library unchanged | 4: 1 `/me/tracks`, 1 `/me/albums`, 2 `/me/playlists` | 12.3 | 33.4 |

An unchanged daily sync costs 1 request per list plus 1 per 50 playlists: 3 up to 50
playlists, 4 for this account. No owned playlist was refetched (snapshots matched). Four
owned playlists store a few tracks fewer than Spotify counts: local files and unavailable
entries without an ID are skipped. Still to check by hand: Liked Songs scrolling at 144 fps
and RSS with the list open and scrolled.
