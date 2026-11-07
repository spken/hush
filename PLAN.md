# Hush — Implementation Plan

Nov 7, 2026 · @spken

## Summary

Build it with **Slint (software renderer) + tokio actors + librespot 0.8 as the Connect device**, and treat the Web API as a scarce, metered resource rather than your main data source. The key change since your Spotifast-style attempt: since March 2026, dev-mode apps have lost every multi-ID batch endpoint and can only read the contents of playlists you own or collaborate on, so "batching via multi-ID endpoints" is no longer possible. The plan therefore pulls catalog metadata (tracks, albums, artists, followed and editorial playlists) through librespot's spclient, uses the Web API only for library lists, search, top items and controlling other devices, and sends every Web API call through one gateway with a persistent SQLite cache, request coalescing, cancellation and an adaptive budget. Playback position comes from librespot's own player events, so there is zero polling while this app is the active device; a normal hour of local listening should cost 0 Web API calls. The target is Windows on a 1920×1080, 144 Hz display with an Apple Music–style dark design, and a large library synced daily or on demand. Estimated steady-state memory is 70–90 MB, with the UI renderer as the biggest variable, which is why a renderer spike is on today's list.

## 6 · Web API dev-mode restrictions (verified Nov 7, 2026)

A new dev-mode Client ID today gets the full February 2026 restrictions, with one revert in March and quota changes in July. Checked against Spotify's [February 2026 changelog](https://developer.spotify.com/documentation/web-api/references/changes/february-2026), [migration guide](https://developer.spotify.com/documentation/web-api/tutorials/february-2026-migration-guide), [March 2026 changelog](https://developer.spotify.com/documentation/web-api/references/changes/march-2026) and [July 2026 changelog](https://developer.spotify.com/documentation/web-api/references/changes/july-2026). Spotify postponed endpoint changes for some *existing* dev apps, but a newly created Client ID is restricted, so assume the table below.

| Area | Dev-mode status now | How the design copes |
| --- | --- | --- |
| Batch fetch (`GET /tracks`, `/albums`, `/artists`, `/episodes`, `/shows`, `/audiobooks`, `/chapters` with `ids=`) | **Removed.** Single-item GETs only. | Catalog hydration goes through librespot spclient; single Web API GETs are a low-priority fallback only. |
| Playlist contents (`GET /playlists/{id}/items`, renamed from `/tracks`) | Only for playlists you own or collaborate on; others return metadata without `items`. | Own playlists: Web API, cached per `snapshot_id`. Followed and editorial playlists: librespot playlist metadata. |
| Search | `limit` max 10 (was 50), default 5. | One request per query with `type=track,artist,album,playlist&limit=10`; explicit "More" uses `offset`. |
| Artist top tracks, new releases, browse categories | **Removed.** | Top tracks from librespot artist metadata. Home screen built from `/me/top`, recently played, saved albums and playlists. |
| Other users (`/users/{id}`, `/users/{id}/playlists`), `/markets` | **Removed.** | Not supported. Country comes from the librespot session. |
| Library writes and checks | Per-type save/remove/follow replaced by `PUT/DELETE /me/library` and `GET /me/library/contains`, using URIs. | One `LibraryService` that batches URIs of mixed types into one call. |
| Library reads | `/me/tracks`, `/me/albums`, `/me/playlists`, `/me/following`, `/me/shows`, `/me/episodes` still available. | Incremental sync into SQLite (section 3). |
| Player | All player endpoints still available (state, devices, queue, play, pause, seek, next, transfer…). | Used only when another device is active. Local control goes through librespot Spirc. |
| Personalisation | `/me/top/{type}` and recently played available. Recommendations, related artists and audio features have been gone since Nov 2024. | No "radio" or recommendation features via Web API; autoplay is handled by librespot's Connect autoplay. |
| Removed fields | Track: `popularity`, `available_markets`, `linked_from`. Album: `label`, `popularity`, `album_group`, `available_markets`. Artist: `followers`, `popularity`. `/me`: `country`, `email`, `product`, `explicit_content`. `external_ids` was restored in March. | No popularity sorting. Label and product tier come from librespot. All model fields are `Option<T>`. |
| Account and app limits | Owner must have Premium; 5 users per app; 25 Client IDs per developer since July. | Fine for personal use. |
| Quota | Since July, counted **per developer account**, shared across all your Client IDs. Quota exhaustion returns 429 with `"reason": "QUOTA_EXCEEDED"`; ordinary rate limits use a rolling 30-second window with unpublished thresholds. | Gateway treats the two 429 kinds differently (section 3). Don't run other dev apps on the same account while testing. |

## 1 · Architecture

One Cargo workspace, a Slint UI on the main thread, and a small set of tokio actors that each own one resource and talk only through bounded channels. Nothing but the gateway touches `api.spotify.com`, and nothing but the playback actor touches librespot.

&#91;embedded content: actor architecture · UI, controller, 4 services, 3 external endpoints\]

Local playback and its progress bar never touch the Web API; catalog pages try the SQLite cache, then spclient, and only then the metered gateway.

### Crate layout

| Crate | Owns | Depends on |
| --- | --- | --- |
| `app` (bin) | Startup, config, wiring actors to the UI, release profile | all below |
| `model` | `SpotifyId` wrappers, domain structs (all optional fields), `Command` / `Event` enums. No I/O. | serde |
| `auth` | PKCE flow, loopback callback, keyring storage, proactive refresh | model |
| `webapi` | `ApiGateway` actor: queue, limiter, coalescing, HTTP cache, typed endpoint fns | model, auth, store |
| `playback` | librespot `Session`, `Spirc`, `Player`; translates `PlayerEvent` into `model::Event` | model, librespot-\* |
| `catalog` | `CatalogSource` trait with two impls (librespot spclient first, Web API fallback) plus the entity cache | model, webapi, playback, store |
| `store` | SQLite: HTTP cache, entities, list membership, sync cursors | rusqlite |
| `images` | CDN fetch, disk cache, decode/resize pool, byte-weighted RAM LRU | model |
| `ui` | `.slint` files, theme, custom `slint::Model` adapters, view state | slint, model |

### Runtime

- Slint's event loop runs on the main thread. tokio runs on a separate thread with `worker_threads(2)` and `max_blocking_threads(4)`: few threads means less stack and allocator-arena memory.
- Image decode runs in `spawn_blocking`, capped at 2 concurrent by a semaphore.

### Messaging

- **UI → core:** every button or scroll becomes a `Command` (e.g. `Play(Uri)`, `OpenAlbum(Id)`, `RowsVisible{list, range}`, `Search(String)`) sent on a bounded `mpsc` (capacity 64) to the `Controller` actor.
- **Controller:** single owner of app state (current view, now-playing anchor, library list keys). It fans out to `PlaybackActor`, `CatalogService`, `ApiGateway`, `ImageService` and never blocks.
- **Request/response:** services take a `oneshot::Sender` plus a `CancellationToken`. When a view closes, its token is cancelled and queued work is dropped before it costs a request.
- **Core → UI:** the controller produces small view-model diffs and applies them with `slint::Weak::upgrade_in_event_loop`. Rows are never pushed wholesale; the UI's model adapter asks for rows (section 5).
- **Playback events:** `PlayerEvent`s go to the controller over a `broadcast` channel. The controller stores a progress anchor `(position_ms, Instant, is_playing)`, and a UI-side 250 ms Slint `Timer` plus a property animation interpolates the bar locally.

### Auth

- **Web API token:** PKCE with your own Client ID, redirect `http://127.0.0.1:8898/callback` (Spotify requires a loopback IP literal, not `localhost`), scopes `user-library-read user-library-modify playlist-read-private playlist-read-collaborative playlist-modify-private user-read-playback-state user-modify-playback-state user-read-recently-played user-top-read`. The refresh token lives in the OS keychain. Refresh at 80% of `expires_in` with a single in-flight refresh, and always store the rotated refresh token Spotify returns.
- **librespot session:** log in with an access token that has the `streaming` scope (`Credentials::with_access_token`), then let librespot's `Cache` keep the reusable stored credentials the access point returns. Later starts use those and need no browser.
- **Open question for M1:** whether a token from *your* Client ID is accepted by the access point. If not, use `librespot-oauth`'s default client for the session login only. Neither path needs a client secret.

### Windows specifics

- **Toolchain:** `stable-x86_64-pc-windows-msvc` plus the Visual Studio C++ Build Tools.
- **Audio:** librespot's rodio backend, which uses cpal and WASAPI shared mode. Test switching output devices (speakers to headphones) mid-track in M1; if playback doesn't follow the new default device, rebuild the sink on device-change notifications.
- **Spotify Connect:** the session registers the device with Spotify's servers, so your phone should list it without local network discovery. Start with zeroconf discovery disabled: no mDNS socket, no Windows Firewall prompt, less memory. Enable it only if the device fails to appear.
- **Credentials:** `keyring` stores the refresh token in Windows Credential Manager.
- **Paths and the 500 MB disk budget:** everything under `%LOCALAPPDATA%\Hush\`: image cache 300 MB, librespot audio cache 150 MB (librespot's `Cache` takes a size limit), SQLite about 50 MB.
- **Media keys and the Windows media overlay:** `souvlaki` drives System Media Transport Controls (title, artist, art and play/pause in the volume flyout and on the lock screen). It needs the window's HWND, which you can get through Slint's winit window handle.
- **Window chrome:** native title bar switched to dark mode with `DwmSetWindowAttribute(DWMWA_USE_IMMERSIVE_DARK_MODE)`.
- **Single instance:** a named mutex, so a second launch focuses the running window instead of starting another Connect device.
- **What "memory" means:** measure the Task Manager *Memory* column (private working set). The 100 MB target applies to that number.

## 2 · UI toolkit: Slint

Use **Slint (1.17.x)** with its software renderer by default and the FemtoVG (OpenGL) renderer behind a feature flag. Slint is the only option of the three that gives you a virtualized `ListView`, a declarative styling language suited to a polished design, and a GPU-free rendering path, all out of the box. The GPU path matters for RAM: profiling of an iced app found a bare wgpu application alone uses more than 50 MB RSS ([ashell notes](https://malpenzibo.github.io/ashell/dev-guide/architecture/known-limitations.html)), and a developer who moved a Rust GUI from egui to Slint's software renderer reported RSS falling from 135 MB to 30 MB ([write-up](https://dev.to/trystan_sarrade/how-i-took-my-rust-gui-from-135-mb-to-30-mb-by-ditching-the-gpu-2i7d)). Treat both as anecdotes and confirm on your machine in today's spike.

| Criterion | egui | iced (0.14) | Slint (1.17) |
| --- | --- | --- | --- |
| Typical RSS for this app | Low–medium with `glow`; higher with wgpu | High on wgpu (wgpu alone >50 MB); lower on tiny-skia | Lowest with software renderer |
| Polished look (typography, art, spacing) | Hardest; immediate-mode look, limited text layout | Good, but you build most custom widgets | Good; CSS-like DSL, embedded fonts, animations, live preview |
| Smooth scrolling of 5k rows | `ScrollArea::show_rows`, repaints whole frame | No built-in virtualized list; you write one | Built-in `ListView` instantiates only visible rows |
| Fit with actor architecture | Polling state each frame | Excellent (Elm messages) | Good (callbacks + `upgrade_in_event_loop`) |
| Idle CPU | Repaints on any input | Reactive rendering since 0.14 | Redraws only dirty regions |
| Main costs | Look and feel | RAM, more widget work | DSL to learn; software rendering costs CPU on big HiDPI windows |

**Trade-off to watch:** software rendering moves work to the CPU. Your MAG27C (1920×1080 at 144 Hz, 100% scaling) keeps the framebuffer small, about 8 MB, but a smooth scroll means up to 144 repaints a second. If the spike shows more than \~25% of one core during a flick-scroll, or dropped frames, switch to FemtoVG and accept roughly 15–30 MB more memory (an estimate to verify).

**Licensing:** Slint is GPLv3, or a free royalty-free license for desktop apps, or commercial. Personal use is fine under either free option.

### Design direction: art-led, immersive, dark

Inspired by Apple Music's calm (large art, lots of space) but with its own look: the current album art sets the mood of the whole window, chrome nearly disappears, and type is set like a poster. No ambient glows and no glass panels. It works within the software renderer's limits, measured in M3: no blur, no drop shadows, and gradients cannot have rounded corners. Sizes assume 100% scaling.

| Element | Spec |
| --- | --- |
| Canvas | One full-window colour field behind everything. Until art loads it is a neutral near-black; from M8 it is flooded by the current art: an upscaled 32×32 copy (which reads as a heavy blur) darkened under the art's average colour, cross-faded on track change. |
| Chrome | No panels, cards or borders. A slim icon rail on the left (icons only, the selected page marked by a soft disc; connection status as a dot at the bottom) and one borderless control strip at the bottom: transport, timeline, volume. Both sit directly on the canvas. |
| Now playing | The home of the app and a poster: art as large as the window allows on the left, an oversized title (56–72 px, bold, tight tracking) and the artist beside it, a white Play pill and a quiet Full Screen action. Full screen hides the rail and makes the window full screen; Esc leaves it. |
| Album grid | Square covers \~180 px, 8 px radius, title (600 weight) and artist (secondary) under each, 24 px gutters, directly on the canvas. Hover brightens the cover and reveals a round play button. |
| Album page | A poster like now playing: cover \~300 px, oversized title, artist, year and track count; Play and Shuffle pills; a plain track list with hover rows. |
| Recently Added | The home view once the library exists, fed by the daily sync. |
| Typography | Segoe UI Variable (built into Windows 11), nothing bundled. Poster scale: display titles 56–72 px bold with tight tracking, body 13–14 px, secondary text at \~60% white, small uppercase labels only where they carry meaning. |
| Colour | Neutral near-black canvas, text at 96/60/35% white. The accent is white until art provides one; from M8 it is taken from the current art. Status colours only for status. |
| Motion | 200–250 ms ease-out for page and hover transitions, cross-fade when the art or canvas changes, kinetic scrolling. Everything must hold 144 fps. |
| Stretch | Native title bar coloured to match the canvas (`DWMWA_CAPTION_COLOR`), so the window reads as one surface. |

## 3 · Rate-limit-proof request layer

The rule is: the cheapest request is the one never sent. Freshness TTLs, librespot-first metadata and event-driven playback remove most calls; the gateway then meters what is left. Spotify publishes no thresholds, only that limits use a rolling 30-second window and that 429 responses usually carry `Retry-After` in seconds ([rate limits](https://developer.Spotify.com/documentation/web-api/concepts/rate-limits)).

### Target request budget

| Scenario | Web API requests |
| --- | --- |
| Launch with warm cache | 0–2 (`/me/playlists` only if the last sync is over 24 h old) |
| First launch, empty cache, \~10,000 liked songs | \~210, spread over \~10 min in the background lane; the library fills in as pages land |
| Open any album or artist page | 0 (librespot) |
| Open your own unchanged playlist | 0 (`snapshot_id` matches) |
| Type "radiohead" into search | 1 |
| Play, pause, skip, seek, volume on this device | 0 (Spirc) |
| One hour of local listening | 0 |
| One hour while another device plays, window visible | \~75–90 (heartbeats plus track ends) |

### The gateway actor (`webapi::ApiGateway`)

1. **One queue, four priority lanes.** P0 user actions (remote play, library save), P1 visible content, P2 prefetch, P3 background sync. Strict priority, with aging so P3 still progresses within \~60 s.
2. **Adaptive token bucket.** Start at 20 requests per 30 s with a burst of 4. On any 429, halve the rate for 10 minutes. After 5 clean minutes, add 1 request per 30 s back, up to the starting value. These are conservative starting numbers, not known limits.
3. **Coalescing.** An in-flight map keyed by `(method, url)` holds waiters; 10 identical GETs cost 1 request.
4. **Cancellation.** Each queued item carries a `CancellationToken`. Items whose view is gone are dropped *before* sending. Requests already sent complete and fill the cache.
5. **429, rate limit.** Pause *all* lanes until `Retry-After` + 0–1 s jitter (default 5 s if the header is missing). Retry the request once, then fail it to the caller.
6. **429, `QUOTA_EXCEEDED`.** Open a circuit breaker: P2/P3 stop for 1 hour, P0/P1 continue only after `Retry-After`, and the UI shows a quiet "Using cached data" banner. Log the body and timing so you learn the real window.
7. **401.** One token refresh, then one retry. **5xx and network errors:** backoff 1, 2, 4 s with jitter, max 3 attempts, P0/P1 only.
8. **Metrics from day one.** Requests per endpoint per minute, 429s by reason, cache hit rate, shown in a debug overlay (Ctrl+Shift+D) and logged.

### Caching

- **Freshness TTLs are what save budget.** A 304 Not Modified most likely still counts as a request, so ETags save bandwidth and parsing, not quota. Store `ETag` when Spotify sends one and send `If-None-Match` on refetch; record in M4 which endpoints actually return ETags.
- Storage: an SQLite `http_cache` table (url key, etag, body, fetched\_at, ttl) plus normalized entity tables that survive restarts.

| Endpoint | Policy |
| --- | --- |
| `/me` | 24 h |
| `/me/playlists` | Daily, plus Sync now (see Library sync). Compare each playlist's `snapshot_id`; only changed ones are refetched. |
| `/playlists/{id}/items` (own) | Keyed by `snapshot_id`, so immutable; cached indefinitely |
| `/me/tracks`, `/me/albums` | Incremental: fetch newest pages (limit 50) until a known `(id, added_at)` appears. Full resync weekly in P3 to catch removals. |
| `/me/following`, `/me/top/*` | 24 h |
| `/me/player/recently-played` | 5 min, only while Home is visible |
| `/artists/{id}/albums` (fallback only) | 24 h |
| `/search` | In-memory LRU of 200 queries, 10 min |

### Library sync: daily and on demand

A daily sync of a library that grew by a few dozen items costs about 3–8 requests, because each list is read newest-first and stops at the first item already known.

- **Triggers:** on launch if the last sync is over 24 h old; a 24 h timer while the app runs (re-checked on resume from sleep); a Sync now button and Ctrl+R. Saves and edits made in this app update SQLite directly and never trigger a fetch.
- **Liked Songs, saved albums:** `limit=50`, newest first, stop at the first known `(id, added_at)`. 20 new songs = 1 request.
- **Playlists:** page `/me/playlists` (about 1 request per 50 playlists) and compare each `snapshot_id`. Only changed playlists you own are refetched; followed playlists refresh through librespot when opened.
- **Removals:** an incremental sync can't see songs you unliked on your phone. Compare the `total` on the first page with the local count; if they disagree, queue a full reconciliation in the background lane. Also run one weekly, and offer Full resync in settings.
- **Feedback:** "Syncing library… 3,200 / 10,400" on the library page, and the library stays usable throughout.
- **Searching your own library** uses SQLite FTS5 over titles, artists and albums. It is instant and costs 0 requests; the Web API search is only for the wider catalog.

### Search

- Debounce 300 ms after the last keystroke; Enter fires immediately. Minimum 2 characters.
- One request covers all types (`type=track,artist,album,playlist&limit=10`). A newer query cancels the older one's queued request and discards its late result.
- "Show more" for one type is an explicit click that pages with `offset`; no infinite scroll on search.

### Lazy loading

- The list model reports which rows are on screen. The controller coalesces those ranges every 100 ms and asks `CatalogService` only for rows not yet in the entity cache, plus one screen of overscan.
- Prefetch beyond that is P2 and only runs when the queue is idle.

### Playback state

- **This app is the active device:** no polling at all. librespot `PlayerEvent`s (playing, paused, seeked, position correction, track changed, end of track, volume, shuffle, repeat) update the anchor, and the bar is interpolated locally. Check the exact variant names in librespot 0.8.
- **Another device is active:** `GET /me/player` on window focus, \~1 s after each user command, at the estimated track end (+1.5 s), and a 60 s heartbeat only while the window is visible. Nothing while minimized. Interpolate between polls the same way.
- **Stretch goal:** read Connect cluster updates from librespot's dealer connection, which would make remote-device state push-based too. Its API exposure is unverified, so it is not in the critical path.

## 4 · librespot's session as the metadata source

Make librespot spclient the *primary* catalog source and the Web API the fallback. In librespot 0.7/0.8, most metadata and autoplay contexts are fetched through spclient, the HTTP API the official clients use, and `SpClient` exposes raw request methods for anything not wrapped ([releases](https://github.com/librespot-org/librespot/releases)). Since your session is already authenticated for playback, these calls cost no Web API budget.

### What to take from librespot

| Need | librespot source (verify names on docs.rs for 0.8) | Web API fallback |
| --- | --- | --- |
| Track title, artists, duration, album, explicit | `librespot_metadata::Track` | `GET /tracks/{id}` (P2) |
| Album with discs, track ids, date, label, cover file ids | `Album` | `GET /albums/{id}` |
| Artist with top tracks by country and album groups | `Artist` | `GET /artists/{id}`, `/artists/{id}/albums` (top tracks: none) |
| Followed and editorial playlists' contents | `Playlist` (playlist v2 via spclient) | none in dev mode |
| Episodes and shows | `Episode`, `Show` | single GETs |
| Cover images | file id → CDN URL (`https://i.scdn.co/image/{file_id}`) | `images[]` URLs |
| Country, Premium status | session / product info | none (fields removed from `/me`) |

### Rules

- `CatalogService` checks the SQLite entity cache first. Track and album metadata is effectively immutable, so cache it with no TTL; refresh artists weekly.
- librespot fetches are per item. Cap them at 6 concurrent, dedupe in-flight ids and coalesce row requests every 100 ms.
- An album page needs the album plus N tracks (about 12 requests for a typical album) on first view, then 0 forever. Spike the batched `extended-metadata` endpoint in M6: its protobufs ship in `librespot-protocol` and could be called through `SpClient`'s raw request API, but this is unverified.
- Keep everything behind `trait CatalogSource` with `SpclientSource` and `WebApiSource`, plus a config switch to force Web API mode when librespot breaks.

### Risks of leaning on the session

- **Private, undocumented API.** Spotify can change it without notice. Example: since Sep 29, 2026 some users see librespot 0.8 fail with a 503 from login5 right after access-point login, and the issue is still open ([#1771](https://github.com/librespot-org/librespot/issues/1771)). The entity cache means an outage degrades to cached browsing instead of a blank app.
- **Breaking API changes between librespot versions.** 0.7 removed album and artist genres and rewrote Spirc. Pin an exact version and upgrade deliberately.
- **Unknown spclient limits.** Stay polite: concurrency cap, no crawling, and fetch only what is on screen or about to be.
- **Terms of service.** Unofficial clients are outside Spotify's terms. Personal, low-volume use keeps the account risk low but not zero.

## 5 · Memory budget

Aim for \~75 MB steady and a 100 MB hard ceiling while playing. Every figure below is an estimate to replace with measurements from M1 and M3; the image cache is the one budget you set yourself.

| Component | Budget (MB, est.) | How it is kept there |
| --- | --- | --- |
| Slint runtime + software framebuffer | 20–30 | At 1920×1080 the RGBA buffer is \~8 MB; partial redraws; no GPU driver mappings |
| librespot session, player, decoder, audio buffers | 12–20 | Audio file cache on disk, not RAM; one sink; default prefetch |
| tokio (2 workers), reqwest + rustls, HTTP/2 | 6–10 | Few threads; one connection pool |
| SQLite | 2–4 | `PRAGMA cache_size=-2048` (2 MB), WAL, mmap off |
| Decoded images (LRU) | 16 cap | Byte-weighted LRU, details below |
| Fonts | 2–4 | One variable font, plus a fallback only if needed |
| Domain data and visible rows | 1–5 | Ids in RAM, row text on demand |
| **Total** | **\~60–90** |  |

### Album art pipeline

1. **Pick the smallest source size ≥ the target pixels.** Spotify serves 640, 300 and 64 px. At 100% scaling, 40 px row thumbnails use the 64 px image as-is, grid tiles (\~180 px) and the album header (\~270 px) use the 300 px image, and full-screen now playing uses 640 px.
2. **Fetch from the CDN** (`i.scdn.co`), which does not count against the Web API. Image URLs are content-addressed by file id and never change, so the disk cache never revalidates. Cap disk use at 300 MB with LRU eviction.
3. **Decode off the UI thread** (2 workers). Resize with SIMD to the exact target pixel size, then convert to a `slint::SharedPixelBuffer<Rgba8Pixel>`. Encoded bytes are dropped right after decode.
4. **RAM LRU weighted by bytes, 16 MB cap.** A 64 px thumbnail is 16 KB, a 180 px tile 130 KB and a 640 px cover 1.6 MB, so the cap holds the full-screen cover plus about 60 grid tiles and 300 row thumbnails. Keys include target size, so the same cover at two sizes counts twice.
5. **Cancel decodes for rows that scrolled away** before they start.
6. **Placeholders cost nothing.** During decode, compute the average colour and store it in SQLite; rows show that colour block until the image lands. The same pass makes a 32×32 copy for the blurred now-playing backdrop, upscaled with smooth filtering.

### Large track lists

- Never keep API JSON or full track structs for a whole list. List membership lives in SQLite. In RAM, a list is a `Vec<u32>` of interned keys: even 20,000 liked songs cost 80 KB.
- Implement a custom `slint::Model`. `row_count()` returns the key count; `row_data(i)` reads from a 300-row LRU of display structs loaded from SQLite in pages of 100. `ListView` only instantiates visible rows, so `SharedString`s exist only for those.
- Sorting and filtering run as SQL queries that return a new key vector.

### General hygiene

- Release profile: `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `strip = true`.
- Measure `mimalloc` against the system allocator (glibc can fragment with several threads). Keep whichever gives lower steady RSS after 30 minutes.
- Show live RSS in the debug overlay and add a `dhat` cargo feature for heap profiling when a budget is exceeded.

## Audio pipeline and playback settings

Playback runs at the highest quality librespot can get: 320 kbps Ogg Vorbis (Spotify's "Very high"), decoded and processed in floating point, gapless, with optional normalisation and a 10-band EQ. **Lossless is not possible**: librespot's maintainers have marked Spotify lossless as "will not be supported" ([#1583](https://github.com/librespot-org/librespot/issues/1583)), because the FLAC streams aren't served to third-party Connect devices. If that changes, the pipeline below takes FLAC without changes after the decoder.

### Signal chain

1. **Decode:** librespot fetches the 320 kbps file (`Bitrate::Bitrate320`) and decodes to 64-bit float samples, 44.1 kHz stereo.
2. **Normalise:** librespot applies Spotify's ReplayGain values from the stream, before any sample conversion, so no dynamic range is lost ([options](https://github.com/librespot-org/librespot/wiki/Options)).
3. **Custom sink (`EqSink`)**, which replaces librespot's built-in rodio sink:
   - EQ preamp, then 10 biquad filters per channel.
   - Volume, read from a custom `AtomicMixer` (see below).
   - A soft-knee safety limiter just under 0 dBFS.
   - Conversion to f32.
4. **Output:** a lock-free ring buffer (\~100 ms) feeds a `cpal` WASAPI shared-mode stream.

Why a custom sink and mixer:

- **Volume after the EQ.** librespot normally applies volume before the sink. Implementing librespot's `Mixer` trait as an atomic that `EqSink` reads keeps volume after the EQ, so EQ boosts can't clip at full volume, and your phone's volume slider still works over Connect.
- **Gapless survives.** The sink stays open between tracks, and filter state carries over, so gapless playback stays seamless with the EQ on.
- **It costs almost nothing.** 20 biquads at 44.1 kHz use well under 1% of one core, and the buffers take less than 100 KB.

### Settings page: Playback

| Setting | Options | Default | Implementation |
| --- | --- | --- | --- |
| Streaming quality | Very high (320 kbps), shown as fixed | Very high | `Bitrate320`; lossless shown as unavailable, with the reason |
| Gapless playback | On / Off | On | librespot gapless mode (on by default); Off closes the sink between tracks |
| Normalise volume | Off / Quiet / Normal / Loud | Normal | Pregain −9 / 0 / +3 dB, the levels librespot documents as matching the official client |
| Normalisation basis | Auto / Track / Album | Auto | Auto uses album gain inside an album context and track gain elsewhere, so album dynamics stay intact |
| Limiter | Dynamic / Basic | Dynamic | librespot's dynamic limiter (threshold, attack, release and knee under Advanced) |
| Equalizer | On / Off, presets (Flat, Bass, Treble, Vocal, Loudness, Custom), 10 sliders at 31 Hz–16 kHz (±12 dB), preamp slider, bypass for A/B listening | Off (Flat) | Low shelf at 31 Hz, peaking filters 62 Hz–8 kHz, high shelf at 16 kHz. Auto preamp = minus the largest boost. |
| Output device | List of Windows output devices, plus "System default" | System default | Enumerated with `cpal`; the stream is rebuilt when the device changes |

**Applying changes live:**

- **No clicks:** slider moves recompute coefficients on the controller and pass them to the sink lock-free (`arc-swap`). The sink ramps toward the new coefficients over \~20 ms.
- **Fast response:** with a \~100 ms ring buffer, an EQ change is heard within about 100–150 ms.
- **Persistence:** settings are stored in SQLite and applied at startup before the first sample plays.

**Windows output note:** WASAPI shared mode resamples to the device's format. To avoid resampling 44.1 kHz audio, set the output device's default format to 24-bit, 44100 Hz in Windows Sound settings. Bit-perfect exclusive mode is out of scope for v1.

## 7 · Milestones in build order

Each milestone ends with a test you can run, and the riskiest unknowns (librespot login, renderer RAM) are settled in the first two.

1. **M1 — Auth + one hardcoded track.** PKCE login with your Client ID, refresh token in the keychain, librespot session login, `Player` loads a hardcoded track URI to the default audio device.
   - Done when: `cargo run --bin m1` plays the track to the end; a second run plays with no browser window; a forced refresh (set expiry to now) succeeds; RSS is logged at the end of playback.
2. **M2 — Spotify Connect device.** `Spirc` registered as "Hush", events printed.
   - Done when: the device appears in your phone's device list; play, pause, next and seek from the phone work; logged `position_ms` stays within 250 ms of the phone's display over a full track; audio now runs through `EqSink` and `AtomicMixer` at 320 kbps; a gapless album plays its transitions with no gap or click; a +6 dB change at 1 kHz is audible within 150 ms with no click.
3. **M3 — UI shell + now playing (renderer decision).** Dark-theme Slint window, now-playing bar and view with large art, local progress interpolation, transport controls via Spirc.
   - Done when: 10 minutes of playback shows a smooth bar with the Web API request counter at 0; RSS under 70 MB; renderer chosen from measured RSS and CPU.
4. **M4 — Gateway + cache.** `ApiGateway`, SQLite cache, metrics overlay, tests against a mock server.
   - Done when: 50 concurrent identical GETs produce 1 network request; a 429 with `Retry-After: 5` stops every lane for at least 5 s; `QUOTA_EXCEEDED` opens the breaker; a 401 triggers exactly one refresh; cancelled queued requests are never sent.
5. **M5 — Library.** Playlists page with `snapshot_id` diffing, incremental Liked Songs sync, virtualized list backed by SQLite.
   - Done when: your full Liked Songs list scrolls at 144 fps without visible stutter; a daily sync with no library changes costs ≤ 3 requests; second launch makes ≤ 2 Web API requests; RSS under 90 MB with the list open.
6. **M6 — Catalog via librespot.** Album, artist (with top tracks) and followed or editorial playlist pages from spclient; spike batched extended metadata.
   - Done when: browsing 20 albums and 5 artists produces 0 Web API requests; with spclient disabled by config, the same pages still load through the Web API fallback.
7. **M7 — Search.** Debounced single-request search with cancellation and caching.
   - Done when: typing "radiohead" at normal speed sends exactly 1 request; repeating it within 10 minutes sends 0.
8. **M8 — Images + polish.** Art-led, immersive views (section 2), full art pipeline, average-colour placeholders, blurred backdrop, animations, Windows media overlay and media keys, and the Playback settings page (EQ, normalisation, gapless, output device).
   - Done when: after 30 minutes of mixed browsing, decoded image bytes stay ≤ 16 MB, peak RSS ≤ 100 MB and steady RSS ≤ 85 MB.
9. **M9 — Remote devices + resilience.** Device picker and transfer to your phone and speakers, with the polling policy, plus degraded modes (librespot down: browse from cache; quota exceeded: cached data, banner).
   - Done when: a scripted 24-hour soak session shows zero 429s and less than 5 MB RSS drift; killing the network for 5 minutes recovers without restarting the app.

## 8 · Risks and unknowns

The two that can sink the project, librespot login and renderer memory, are tested in M1–M3 before any feature work.

| Risk | Impact | De-risk early |
| --- | --- | --- |
| Spotify changes break librespot (login5 503 reports since Sep 29, 2026) | No playback | M1 first, on your real account and network. Pin the version, watch the issue tracker, keep browsing working when the session is down. |
| AP rejects a token from your own Client ID | Two login flows | Test on day 1. Fallback: librespot-oauth's client for the session login only. |
| Unknown dev-mode thresholds; quota shared across your account | Throttling returns | Gateway metrics and conservative bucket from M4. Log every 429 body. No other dev apps on this account. |
| More Web API cuts (Nov 2024, Feb 2026 precedent; rspotify heading to maintenance mode) | Features vanish | librespot-first catalog; thin in-house Web API client; all fields optional. |
| spclient per-item metadata is chatty or limited | Slow first views | Concurrency cap, permanent entity cache, extended-metadata spike in M6. |
| Software rendering can't hold 144 fps while scrolling | Janky scroll or high CPU | Renderer spike today; FemtoVG feature flag. |
| Audio backend quirks per OS | No sound | M1 on Windows with the rodio/WASAPI backend; switch output devices mid-track as part of the test. |
| ToS / account action | Account risk | Personal use, no crawling, behave like a normal client. |
| librespot breaking API changes | Rework | Pin exact versions; isolate librespot inside the `playback` and `catalog` crates. |

## 9 · Recommended crates

Pin exact versions for librespot and Slint; float the rest within semver. librespot 0.8 needs Rust 1.85 or newer.

| Crate | Why |
| --- | --- |
| `librespot-core`, `-playback`, `-connect`, `-metadata` 0.8 | Session, audio, Spotify Connect and spclient metadata in one stack |
| `librespot-oauth` 0.8 | Ready-made PKCE flow; the fallback for session login |
| `slint` 1.17 | Virtualized lists, styling DSL, software renderer |
| `tokio` + `tokio-util` | Async runtime; `CancellationToken` for dropping stale requests |
| `reqwest` (rustls, http2, gzip) | One pooled HTTP/2 client for the Web API and the image CDN |
| `serde`, `serde_json` | Typed Web API responses with optional fields |
| `oauth2` | PKCE code verifier, challenge and token exchange without a secret |
| `keyring` | Refresh token in the OS keychain, not a plain file |
| `rusqlite` (bundled) | HTTP cache, entities, list membership, sync cursors |
| `governor` | Token-bucket limiter (or \~50 lines of your own) |
| `quick_cache` | LRU with a byte weigher for decoded images |
| `zune-jpeg` / `image` | Fast JPEG decode, PNG fallback |
| `fast_image_resize` | SIMD downscaling to exact target pixels |
| `tracing`, `tracing-subscriber` | Logs plus per-endpoint request metrics |
| `memory-stats`, `dhat` | Live RSS in the overlay; heap profiling on demand |
| `mimalloc` | Optional allocator, kept only if it measurably lowers RSS |
| `wiremock` | Mock Web API for 429, 401 and coalescing tests |
| `directories` | Correct per-OS config and cache paths |
| `thiserror`, `anyhow` | Typed errors in libraries, context in the binary |
| `open` | Open the system browser for the PKCE login |

Windows additions: `souvlaki` for the media overlay and media keys (System Media Transport Controls), and `windows` for the dark title bar and the single-instance mutex.

Audio additions: `cpal` for direct WASAPI output from `EqSink`, `rtrb` for the lock-free ring buffer, `biquad` for the EQ filters (or \~40 lines of the RBJ cookbook formulas), and `arc-swap` for passing new filter coefficients to the audio side without locks.

Skip `rspotify`: its maintainer said it enters maintenance mode after the February 2026 patch, and you need full control of the request pipeline anyway ([rspotify #550](https://github.com/ramsayleung/rspotify/issues/550)).

## First 3 tasks for today

- [ ] **Register the app.** Create a dev-mode app in the Spotify developer dashboard with redirect URI `http://127.0.0.1:8898/callback`, note the Client ID, install the Visual Studio C++ Build Tools, and confirm Rust ≥ 1.85 (`rustup update`).
- [ ] **M1 spike: play one track.** New workspace with librespot 0.8. Run the `librespot-oauth` async example to get a token, log a `Session` in with it, and play a hardcoded track through `Player` on the default backend. Then repeat with a token from your own Client ID to answer the auth question. Log RSS at the end.
- [ ] **Renderer spike.** A Slint window with a `ListView` of 5,000 dummy rows, each with a 96 px image and two text lines. Record RSS and CPU while flick-scrolling with the software renderer and with FemtoVG on the MAG27C at 144 Hz, both maximized and at your usual window size. Keep the faster one that stays under \~35 MB.

## Decisions (Nov 7)

| Topic | Decision |
| --- | --- |
| Name | Hush (app, window title, Connect device name, data folder) |
| Platform | Windows, MSVC toolchain |
| Display | MSI MAG27C: 27", 1920×1080, 144 Hz; 100% scaling assumed |
| Playback scope | Local playback first; phone and speaker control in M9 |
| Design | Art-led and immersive: the album art floods the window, minimal chrome, poster type, dark; inspired by Apple Music, not a copy (section 2) |
| Audio | 320 kbps Vorbis (lossless unavailable through librespot), gapless on, normalisation Normal/Auto, 10-band EQ in Settings |
| Disk cache | 500 MB total: images 300, audio 150, SQLite \~50 |
| Library | Large and growing daily: daily plus on-demand sync, local search via FTS5. Sizes in this plan assume \~10,000 liked songs. |

## Sources

- [Web API changelog, February 2026](https://developer.spotify.com/documentation/web-api/references/changes/february-2026)
- [February 2026 migration guide](https://developer.spotify.com/documentation/web-api/tutorials/february-2026-migration-guide)
- [Web API changelog, March 2026](https://developer.spotify.com/documentation/web-api/references/changes/march-2026)
- [Web API changelog, July 2026](https://developer.spotify.com/documentation/web-api/references/changes/july-2026)
- [Web API quota updates for Development Mode (July 23, 2026)](https://developer.spotify.com/blog/2026-07-23-web-api-quota-updates)
- [Spotify Web API rate limits](https://developer.Spotify.com/documentation/web-api/concepts/rate-limits)
- [February 2026 developer update thread (postponement for existing apps)](https://community.spotify.com/t5/Spotify-for-Developers/February-2026-Spotify-for-Developers-update-thread/td-p/7330564)
- [librespot releases (0.8.0 notes)](https://github.com/librespot-org/librespot/releases)
- [librespot issue #1771: login5 503](https://github.com/librespot-org/librespot/issues/1771)
- [rspotify issue #550: Feb 2026 changes, maintenance mode](https://github.com/ramsayleung/rspotify/issues/550)
- [iced 0.14.0 release notes](https://github.com/iced-rs/iced/releases/tag/0.14.0)
- [Slint releases](https://github.com/slint-ui/slint/releases)
- [ashell known limitations (iced/wgpu memory)](https://malpenzibo.github.io/ashell/dev-guide/architecture/known-limitations.html)
- [Rust GUI from 135 MB to 30 MB with Slint](https://dev.to/trystan_sarrade/how-i-took-my-rust-gui-from-135-mb-to-30-mb-by-ditching-the-gpu-2i7d)
