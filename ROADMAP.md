# Hush — Development Roadmap

Derived from *Hush — Implementation Plan* (Nov 7, 2026). The milestones keep the plan's
risk-first order: the two things that can sink the project (librespot login and renderer
memory) are settled before any feature work. Each milestone ends with an **exit test**;
do not start the next milestone until it passes.

Architecture and coding rules live in [AGENTS.md](AGENTS.md).

---

## Changes from the plan

Small adjustments made while turning the plan into steps. Each one removes coupling or
duplication; none changes the product.

| # | Plan says | Roadmap does | Why |
| --- | --- | --- | --- |
| 1 | `catalog` depends on `playback` | `catalog` depends on `librespot-core` only; `app` passes the `Session` in | Catalog only needs the session, not the player. Keeps playback swappable. |
| 2 | Coalescing, cancellation and request/reply live in gateway, catalog and images separately | One shared `kit` crate: `Coalescer<K, V>`, `Req<T>`, `backoff`, byte-weighted LRU wrapper | The same logic was going to be written three times. |
| 3 | `images` depends on `model` only | `images` also depends on `store` (average colour is stored in SQLite) | The plan stores placeholder colours in SQLite. |
| 4 | Gateway and images each fetch over HTTP | One `reqwest::Client` built in `app`, passed to both | One connection pool, one TLS config. |
| 5 | `cargo run --bin m1` | Spikes are `examples/` in the crate they exercise (`cargo run -p playback --example m1`) | Spike code never ships in the release binary and never needs a second `[[bin]]`. |
| 6 | M2 = Connect device **and** custom `EqSink` + mixer | Split into **M2a** (Connect) and **M2b** (audio chain) | Settle the Connect risk before writing audio DSP. |
| 7 | "glibc can fragment" as the `mimalloc` reason | Measure `mimalloc` vs the Windows segment heap | Target is Windows; glibc does not apply. Still measure. |
| 8 | Crates created up front | A crate is created in the milestone that first needs it | No empty crates, no speculative code. |

---

## Phase 0 — Setup (today)

1. Create a dev-mode app in the Spotify developer dashboard. Redirect URI
   `http://127.0.0.1:8898/callback`. Record the Client ID in `.env.example` (no secrets).
2. Install Visual Studio C++ Build Tools; `rustup update`; confirm `rustc >= 1.92` on
   `stable-x86_64-pc-windows-msvc`.
3. Create the workspace skeleton: root `Cargo.toml` with `[workspace]`, `resolver = "3"`,
   `[workspace.package] edition = "2024"`, `[workspace.dependencies]` (exact pins for
   `librespot-*` and `slint`), `[workspace.lints]`, and the release profile
   (`lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `strip = true`).
4. Add `rustfmt.toml`, `.gitignore` (`/target`, `.env`), and a GitHub Actions workflow on
   `windows-latest`: `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`.
5. **Renderer spike** (`crates/ui/examples/renderer_spike.rs`): a Slint window with a
   `ListView` of 5,000 dummy rows, each a 96 px image and two text lines.
   - Measure RSS (Task Manager *Memory* column) and CPU during a flick-scroll, with the
     software renderer and with FemtoVG, maximized and at normal window size.
   - Record the numbers in `docs/measurements.md`.
   - **Exit:** a renderer is chosen. Keep the software renderer if it holds 144 fps under
     ~25% of one core and ~35 MB; otherwise FemtoVG.

---

## M1 — Auth + one hardcoded track

Crates created: `model`, `auth`, `playback`.

1. `model`: `SpotifyId`/`Uri` newtypes, `Error` type, nothing else yet.
2. `auth`: PKCE flow (`oauth2`), loopback listener on `127.0.0.1:8898`, open the browser
   with `open`, exchange code, store the refresh token with `keyring`.
3. `auth`: `TokenProvider` — returns a valid access token, refreshes at 80% of
   `expires_in`, one in-flight refresh, always stores the rotated refresh token.
4. `playback`: log a librespot `Session` in with `Credentials::with_access_token`, let
   librespot's `Cache` (`%LOCALAPPDATA%\Hush\`) keep stored credentials.
5. **Answer the open auth question:** try a token from Hush's own Client ID against the
   access point. If rejected, use `librespot-oauth`'s default client for the session login
   only. Record the answer in `docs/decisions.md`.
6. `playback/examples/m1.rs`: `Player` loads a hardcoded track on the default device,
   logs RSS at the end.
7. Switch output device (speakers → headphones) mid-track; note whether playback follows.

**Exit test:** the example plays the track to the end; a second run plays with no browser
window; a forced refresh (expiry set to now) succeeds; RSS is logged.

---

## M2a — Spotify Connect device

1. `playback`: `Spirc` registered as "Hush", zeroconf discovery **off**.
2. Map librespot `PlayerEvent` → `model::Event` (verify variant names in 0.8). This mapping
   is the only place librespot event types appear.
3. Progress anchor `(position_ms, Instant, is_playing)` in `model`, with one
   `position_at(Instant)` function used everywhere.
4. Example prints events and the interpolated position every 250 ms.

**Exit test:** "Hush" appears in the phone's device list; play, pause, next, seek from the
phone work; logged position stays within 250 ms of the phone over a full track.

## M2b — Audio chain

1. `playback::audio`: `AtomicMixer` implementing librespot's `Mixer` trait.
2. `EqSink` implementing librespot's `Sink`: preamp → 10 biquads/channel → volume →
   soft-knee limiter → f32 → `rtrb` ring buffer (~100 ms) → `cpal` WASAPI stream.
3. Filter coefficients passed via `arc-swap`, ramped over ~20 ms.
4. Force `Bitrate320`, gapless on, normalisation Normal/Auto.

**Exit test:** a gapless album plays with no gap or click; +6 dB at 1 kHz is audible within
150 ms with no click; phone volume slider still works.

---

## M3 — UI shell + now playing

Crates created: `ui`, `app`.

1. `app`: tokio runtime on its own thread (`worker_threads(2)`, `max_blocking_threads(4)`),
   Slint on the main thread, single-instance mutex, dark title bar.
2. `app::controller`: the `Controller` actor — owns app state, receives `Command`s on a
   bounded `mpsc(64)`, pushes view-model diffs via `upgrade_in_event_loop`.
3. `ui`: theme tokens (`theme.slint`: colours, spacing, radii, type scale, durations) —
   every other `.slint` file reads from it.
4. `ui`: window layout (icon rail, content canvas, bottom control strip), the now-playing
   poster page with a placeholder art block, and a full-screen mode.
5. Local progress: 250 ms Slint `Timer` + property animation from the anchor.
6. Transport controls → `Command` → Controller → Spirc.
7. Debug overlay stub (Ctrl+Shift+D): RSS via `memory-stats`, Web API request counter.

**Exit test:** 10 minutes of playback, smooth bar, request counter at 0, RSS < 70 MB.
Renderer choice confirmed with real UI.

---

## M4 — Gateway + cache

Crates created: `kit`, `store`, `webapi`.

1. `kit`: `Req<T>` (reply `oneshot` + `CancellationToken`), `Coalescer<K, V>`,
   `backoff(attempt)`, `ByteLru<K, V>` over `quick_cache`. Unit-tested.
2. `store`: one SQLite connection on a blocking thread; migrations as numbered SQL files
   embedded with `include_str!`; `PRAGMA cache_size=-2048`, WAL, mmap off.
   Tables: `http_cache`, `sync_cursor`, `settings`.
3. `webapi::ApiGateway`:
   1. Four priority lanes (P0–P3) with aging (P3 within ~60 s).
   2. Adaptive token bucket: 20 / 30 s, burst 4; halve for 10 min on 429; +1 per 5 clean min.
   3. Coalescing via `kit::Coalescer` keyed by `(method, url)`.
   4. Drop cancelled items before sending.
   5. 429 rate limit → pause all lanes for `Retry-After` + jitter (default 5 s), retry once.
   6. 429 `QUOTA_EXCEEDED` → circuit breaker (P2/P3 off 1 h), emit `Event::Degraded`.
   7. 401 → one refresh, one retry. 5xx/network → `kit::backoff`, 3 attempts, P0/P1 only.
   8. TTL table from the plan as one `const` array; ETag stored when present.
4. Typed endpoint functions: one generic `get<T: DeserializeOwned>(path, lane, ttl)`; each
   endpoint is a one-line call to it. All model fields `Option<T>`.
5. Metrics (per endpoint per minute, 429s by reason, cache hit rate) into the debug overlay.
6. `wiremock` tests for each exit criterion below.

**Exit test:** 50 concurrent identical GETs → 1 request; `Retry-After: 5` stops every lane
≥ 5 s; `QUOTA_EXCEEDED` opens the breaker; 401 → exactly one refresh; cancelled queued
requests are never sent.

---

## M5 — Library

1. `store`: entity tables (`track`, `album`, `artist`, `playlist`), membership tables
   (`list_item(list_id, pos, entity_key, added_at)`), FTS5 index over titles/artists/albums.
2. `catalog::sync`: one incremental-sync function parameterised by endpoint — newest first,
   `limit=50`, stop at the first known `(id, added_at)`. Used for Liked Songs **and** saved
   albums.
3. Playlist sync: page `/me/playlists`, compare `snapshot_id`, refetch only changed owned
   playlists.
4. Removal detection: first-page `total` vs local count → queue full reconciliation (P3).
5. Triggers: launch if > 24 h, 24 h timer (re-checked on resume), Sync now / Ctrl+R.
6. `ui`: one generic `KeyListModel` (`slint::Model`) — `Vec<u32>` keys + 300-row LRU loaded
   from SQLite in pages of 100. Every list view (songs, albums, playlist items, search
   results) uses it.
7. Playlists page reached from the icon rail; "Syncing library… n / total" progress.
8. Visible-range reporting: `Command::RowsVisible`, coalesced every 100 ms.

**Exit test:** full Liked Songs scrolls at 144 fps; unchanged daily sync ≤ 3 requests;
second launch ≤ 2 requests; RSS < 90 MB with the list open.

---

## M6 — Catalog via librespot

Crate extended: `catalog`.

1. `trait CatalogSource` with `SpclientSource` and `WebApiSource`; config switch to force
   Web API.
2. `CatalogService`: SQLite → spclient → Web API, in that order, in one function.
   Concurrency cap 6 (`tokio::sync::Semaphore`), dedupe via `kit::Coalescer`.
3. Tracks/albums cached forever; artists refreshed weekly.
4. Album, artist (with top tracks) and followed/editorial playlist pages.
5. Play from Hush: `Command::Play { context, start }` (album, playlist, artist or Liked
   Songs, optionally from one track; shuffle as a flag) → Controller → `Spirc::load`. If
   Hush is not the active device, it activates itself first. Play and Shuffle on every page
   and double-click on a track row use it.
6. Spike: batched `extended-metadata` via `SpClient` raw requests. Record result in
   `docs/decisions.md`.

**Exit test:** browsing 20 albums + 5 artists → 0 Web API requests; with spclient disabled,
the same pages load via the fallback; Play on a playlist and on an album starts it on Hush
with no other Spotify app open, at 0 Web API requests.

---

## M7 — Search

1. Debounce 300 ms, Enter fires immediately, minimum 2 characters.
2. One request: `type=track,artist,album,playlist&limit=10`; newer query cancels older.
3. In-memory LRU (200 queries, 10 min) — reuse `kit::ByteLru` or a count-bounded variant,
   not a new cache.
4. "Show more" per type pages with `offset`.
5. Local library search via FTS5 shown first, 0 requests.

**Exit test:** typing "radiohead" at normal speed → exactly 1 request; repeat within 10 min → 0.

---

## M8 — Images + polish

Crate created: `images`.

1. Size picker: smallest of 64/300/640 ≥ target pixels.
2. Disk cache (300 MB LRU, never revalidated) under `%LOCALAPPDATA%\Hush\images`.
3. Decode in `spawn_blocking` (semaphore 2), `fast_image_resize` to exact size,
   `SharedPixelBuffer<Rgba8Pixel>`; drop encoded bytes immediately.
4. RAM `kit::ByteLru` capped at 16 MB, key includes target size.
5. Same decode pass: average colour → `store`; 32×32 copy for the blurred backdrop.
6. Cancel decodes for rows scrolled away.
7. Art-led, immersive views (plan, section 2): album grid, album page, Recently Added home, full-screen now
   playing, hover/press motion at 200–250 ms ease-out.
8. Windows: `souvlaki` media overlay + media keys.
9. Settings → Playback: EQ (presets, 10 sliders, preamp, bypass), normalisation, gapless,
   output device list (`cpal`), persisted in `store.settings`.

**Exit test:** after 30 min of mixed browsing, decoded image bytes ≤ 16 MB, peak RSS
≤ 100 MB, steady RSS ≤ 85 MB.

---

## M9 — Remote devices + resilience

1. Device picker + transfer via Web API player endpoints.
2. Remote polling policy: on focus, ~1 s after each command, at track end + 1.5 s, 60 s
   heartbeat only while visible; none while minimized. Same anchor interpolation as M2a.
3. Degraded modes: librespot down → browse from cache; quota exceeded → cached data +
   banner. Both driven by one `Event::Degraded(reason)`.
4. Network loss recovery: reconnect session, resume gateway lanes.
5. Scripted 24 h soak (example binary) logging 429s and RSS.
6. Decide `mimalloc` vs system allocator from soak data.

**Exit test:** 24 h soak with zero 429s and < 5 MB RSS drift; 5 min network kill recovers
without restart.

---

## Every milestone

- RSS and Web API request count recorded in `docs/measurements.md`.
- `cargo fmt`, `clippy -D warnings` clean; exit criteria checked by hand.
- Delete spike code that the milestone made obsolete.
- Re-check librespot issue [#1771](https://github.com/librespot-org/librespot/issues/1771)
  and the Spotify Web API changelog before starting.

## Stretch (not scheduled)

- Push-based remote state via librespot dealer / Connect cluster updates.
- Native title bar coloured to match the canvas (`DWMWA_CAPTION_COLOR`).
- Zeroconf discovery, only if the device fails to appear without it.

## Dependency order

```
Phase 0 ─► M1 ─► M2a ─► M2b
                  │
                  └──► M3 ─► M4 ─► M5 ─► M6 ─► M7 ─► M8 ─► M9
```

M2b (audio chain) can run in parallel with M3 once M2a is done; everything from M4 on is
sequential because each milestone builds on the previous crate.
