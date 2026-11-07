# AGENTS.md

Instructions for anyone — human or AI agent — writing code in this repository.
Read this before writing code. The build order is in [ROADMAP.md](ROADMAP.md).

Hush is a native Windows Spotify client in Rust: Slint UI, tokio actors, librespot as the
Connect device and metadata source, and a metered Web API gateway.

---

## 1 · Prime directive: the least code that works

1. **Smallest correct change.** Write the code the current milestone needs, nothing for a
   future one. No speculative traits, generics, config flags, or "in case" parameters.
2. **One home per concept.** Every type, constant, query, and algorithm exists in exactly
   one place. Before writing anything, search for it (`rg`). If it exists, use it. If it
   almost exists, change it to fit both callers.
3. **No duplicate code.** The second time logic appears, it moves to its shared home
   (table in §3). Two similar functions differing by a value become one function taking
   that value.
4. **No unnecessary code paths.** One way to do each thing. No dead branches, no unused
   `pub` items, no commented-out code, no `TODO` without a milestone tag (`TODO(M6)`).
5. **Keep it stupid simple.** Prefer a function over a trait, a struct over a builder,
   a `match` over a state-machine crate, a `const` table over a config file. Add an
   abstraction only when a second real implementation exists (e.g. `CatalogSource`).
6. **Delete freely.** Code made obsolete by a change is removed in the same change.

---

## 2 · Architecture

### Shape

```
            Slint UI (main thread)
                 │ Command            ▲ view-model diffs
                 ▼                    │ (upgrade_in_event_loop)
          ┌──────────── Controller (app) ────────────┐
          │      single owner of app state           │
          ▼            ▼             ▼               ▼
   PlaybackActor  CatalogService  ApiGateway    ImageService
   (librespot)    (store→spclient (Web API,     (CDN, decode,
                   →webapi)        only caller)  RAM LRU)
          │            │             │               │
          └────────────┴──── store (SQLite) ─────────┘
```

- Each actor owns exactly one resource and is the only code that touches it.
  - Only `webapi` talks to `api.spotify.com`.
  - Only `playback` (and `catalog` via a passed-in `Session`) touches librespot.
  - Only `store` runs SQL.
- Actors communicate only through bounded channels. No shared mutable state, no
  `Arc<Mutex<AppState>>`.
- The Controller is the only writer of app state. Services return data; they do not
  update the UI.

### Crates

| Crate | Owns | May depend on |
| --- | --- | --- |
| `model` | IDs, domain structs, `Command`, `Event`, progress anchor, shared `Error`. Pure data, no I/O. | `serde` |
| `kit` | Generic building blocks used by ≥ 2 crates: `Req<T>` + `call`, `Coalescer<K, V>`, `backoff`, `ByteLru`. | `model`, `tokio`, `tokio-util`, `quick_cache` |
| `store` | SQLite connection, migrations, every SQL query. | `model` |
| `auth` | PKCE, loopback callback, keyring, `TokenProvider`. | `model` |
| `webapi` | `ApiGateway` actor and typed endpoint fns. | `model`, `kit`, `auth`, `store` |
| `playback` | librespot `Session`/`Spirc`/`Player`, `EqSink`, `AtomicMixer`, event mapping. | `model`, `kit`, librespot |
| `catalog` | `CatalogSource` trait, spclient + Web API sources, library sync. | `model`, `kit`, `store`, `webapi`, `librespot-core`, `librespot-metadata` |
| `images` | CDN fetch, disk cache, decode/resize, RAM LRU. | `model`, `kit`, `store` |
| `ui` | `.slint` files, theme tokens, `slint::Model` adapters. | `model`, `slint` |
| `app` (bin) | `main`, runtime setup, Controller, wiring, Windows glue. | everything |

**Rules**

- The graph is a DAG pointing down the table. A lower crate never imports a higher one.
  If two crates need the same thing, it moves down (usually to `model` or `kit`).
- `model` and `ui` never perform I/O. `ui` never imports a service crate.
- Third-party types do not cross crate boundaries. librespot types stay inside `playback`
  and `catalog`; `rusqlite` types stay inside `store`; `reqwest` types stay inside
  `webapi`/`images`. Convert to `model` types at the edge.
- A crate is created in the milestone that first needs it, never earlier.
- Shared singletons (`reqwest::Client`, `Session`, `Store` handle, `TokenProvider`) are
  built once in `app` and passed in. No globals, no `lazy_static`/`OnceLock` for services.

### Shared homes — where reusable code goes

| If you are writing… | It goes in | Not in |
| --- | --- | --- |
| A struct describing Spotify data, a `Command` or `Event` variant | `model` | the service that first needs it |
| Request/reply with cancellation | `kit::Req<T>` | ad-hoc `oneshot` + token pairs |
| "Many callers want the same in-flight thing" | `kit::Coalescer` | a local `HashMap<_, Vec<Sender>>` |
| Retry delay with jitter | `kit::backoff` | inline `sleep(2^n)` |
| A memory-bounded cache | `kit::ByteLru` | a new LRU type |
| Any SQL | a function in `store` | string SQL in another crate |
| A Web API endpoint | one line calling `webapi::get`/`send` | a new HTTP code path |
| A colour, size, radius, font size, duration | `ui/theme.slint` | a literal in a component |
| A repeated Slint widget (button, row, art tile) | `ui/components/` | copy-pasted markup |
| A list backed by IDs | `ui::KeyListModel` | a new `slint::Model` impl |
| A tunable number (TTL, budget, cap) | a `const` next to its only user, or `model::limits` if shared | a magic number inline |

### Actor template

Every actor follows the same shape. Do not invent variations.

```rust
pub struct Handle { tx: mpsc::Sender<Msg> }

enum Msg {
    Album(Req<Album>, AlbumId),
}

impl Handle {
    pub fn spawn(deps: Deps) -> Self {
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(run(deps, rx));
        Self { tx }
    }

    pub async fn album(&self, id: AlbumId, cancel: CancellationToken) -> Result<Album> {
        kit::call(&self.tx, cancel, |req| Msg::Album(req, id)).await
    }
}

async fn run(deps: Deps, mut rx: mpsc::Receiver<Msg>) {
    while let Some(msg) = rx.recv().await {
        match msg { /* never block here; spawn long work */ }
    }
}
```

- Channels are bounded (`64` unless measured otherwise).
- Every request carries a `CancellationToken`. Check it before doing expensive work.
- An actor's `run` loop never `.await`s on another actor's reply inline if that could
  deadlock; spawn the call and send the result back as a message.

### Data flow rules

- Catalog lookups: `store` → spclient → Web API, in that order, in one function.
- Playback position is never polled while Hush is the active device. It is derived from
  the anchor with `model::Anchor::position_at`.
- Lists are `Vec<u32>` of keys in RAM; row text is loaded on demand. Never hold a whole
  list of API JSON or full structs.
- Settings are read from `store` at startup and changed only through `Command`s.

---

## 3 · Rust language guide

### Toolchain and workspace

- Rust stable ≥ 1.85, `x86_64-pc-windows-msvc`, **edition 2024**, `resolver = "3"`.
- All dependency versions are declared once in root `[workspace.dependencies]`; member
  crates use `dep = { workspace = true }`. Pin `librespot-*` and `slint` with `=x.y.z`.
- Enable only the features you use (`default-features = false` where it saves size).
- Lints are set once in the root:

```toml
[workspace.lints.rust]
unsafe_code = "deny"            # allowed only in app::windows, with a SAFETY comment
unused_must_use = "deny"
missing_debug_implementations = "warn"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
module_name_repetitions = "allow"
must_use_candidate = "allow"
missing_errors_doc = "allow"
unwrap_used = "warn"
expect_used = "warn"
dbg_macro = "warn"
todo = "warn"
print_stdout = "warn"
```

- Before every commit: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`.

### Layout and naming

- One module per concept; files stay short (aim < 300 lines). `lib.rs` re-exports the
  public surface and contains little else.
- Default to private. `pub(crate)` before `pub`. Every `pub` item must have a caller
  outside its module.
- Names: `snake_case` functions/modules, `UpperCamelCase` types, `SCREAMING_SNAKE` consts.
  No `Manager`, `Helper`, `Utils`, `Impl` suffixes. A function name is a verb; a type name
  is a noun.
- Newtypes for IDs (`TrackId(SpotifyId)`), never raw `String`s for identity.

### Types and data

- Model fields from the Web API are `Option<T>` — Spotify removes fields without notice.
  Use `#[serde(default)]` rather than custom deserializers.
- Prefer `Box<str>`/`Arc<str>` for immutable strings held long-term; `slint::SharedString`
  only at the UI boundary.
- Use `enum`s for closed sets (`Lane::{P0, P1, P2, P3}`), not integers or strings.
- Derive what is needed: `Debug` always; `Clone`, `PartialEq`, `Eq`, `Hash` only when used.
- Prefer borrowing (`&str`, `&[T]`) in parameters; take ownership only when you store it.
- Avoid `.clone()` to satisfy the borrow checker — restructure first. Cloning an `Arc` or a
  small `Copy` type is fine.

### Errors

- Library crates: one `thiserror` enum per crate (`webapi::Error`), converted into
  `model::Error` at the crate edge if the Controller needs to match on it.
- `app`: `anyhow::Result` with `.context("…")`.
- No `unwrap()`/`expect()` outside tests and `main` startup. If a panic is truly
  impossible, use `expect("why this cannot fail")`.
- Never silently discard an error. Either handle it, return it, or `tracing::warn!` it.
  `let _ = tx.send(..)` is allowed only for a receiver that may have legitimately gone away
  (closed view), with a comment.
- Release builds use `panic = "abort"`: a panic kills the app. Treat panics as bugs.

### Async and concurrency

- tokio only. Runtime: 2 worker threads, 4 blocking threads, created in `app`.
- Never block a tokio worker: SQLite, image decode, file I/O and keyring calls go through
  `spawn_blocking` (or the owning actor's blocking thread).
- Never hold a `std::sync::Mutex` guard across `.await`. Prefer message passing; use a
  mutex only for tiny, non-async critical sections.
- Every `tokio::spawn` is either owned by an actor or tied to a `CancellationToken`.
  No fire-and-forget tasks that outlive their purpose.
- Bounded concurrency via `tokio::sync::Semaphore` (catalog 6, decode 2).
- Real-time audio thread (`EqSink` → `cpal` callback): no allocation, no locks, no
  logging, no syscalls. Communicate via `rtrb` and `arc-swap` only.
- UI thread: never `.await`, never block. Reach it only via `slint::Weak::upgrade_in_event_loop`.

### Memory and performance

Memory is a product requirement (≈ 75 MB steady, 100 MB ceiling).

- Measure before optimising; record numbers in `docs/measurements.md`.
- No unbounded collections. Every cache has a byte or count cap; every channel is bounded.
- Drop large buffers as soon as they are consumed (encoded image bytes after decode, HTTP
  bodies after parse).
- Avoid intermediate `Vec`s: iterate, use `extend`, `with_capacity` when size is known.
- No `serde_json::Value` for known shapes; deserialize straight into typed structs.

### Unsafe and platform code

- `unsafe` is denied workspace-wide. The only exception is `app::windows` (DWM dark title
  bar, single-instance mutex, HWND access), with `#[allow(unsafe_code)]` on that module and
  a `// SAFETY:` comment on every block.
- Platform-specific code lives behind `#[cfg(windows)]` in that one module.

### Logging and metrics

- `tracing` everywhere; never `println!`. Use structured fields:
  `tracing::info!(endpoint, status, "request done")`.
- Levels: `error` = user-visible failure, `warn` = degraded but handled (429, fallback),
  `info` = lifecycle, `debug` = per-request, `trace` = per-sample/row (off by default).
- Never log tokens, refresh tokens, or authorization headers.

### Security

- Secrets (refresh token) only in Windows Credential Manager via `keyring`. Never in files,
  logs, or SQLite.
- The loopback server binds `127.0.0.1` only and shuts down after one callback.
- The Client ID is not a secret but lives in config, not in code.

### Testing

- Unit tests sit beside the code in `#[cfg(test)] mod tests`. Test behaviour, not
  implementation.
- Integration tests for the gateway use `wiremock`; never call real Spotify in tests.
- `store` tests use an in-memory SQLite database with the real migrations.
- Each milestone exit test in ROADMAP.md is either an automated test or an `examples/`
  program with documented steps.
- No mocking frameworks. If code is hard to test, pass the dependency in as a value.

### Slint

- `.slint` files describe layout and style only; all logic lives in Rust.
- Every visual constant comes from `theme.slint`. No hex colours or pixel sizes elsewhere.
- Reusable components live in `ui/components/`; a page composes components, it does not
  redefine them.
- Lists use `ListView` + `KeyListModel`. Never push a full list into a `VecModel`.
- Animations: 200–250 ms ease-out, defined once as theme durations.
- Design bar: enterprise-grade, Apple Music–style dark theme (see the plan, section 2).
  Consistent spacing grid, one accent colour, clear hierarchy, no visual noise.

### Dependencies

- Adding a crate requires a reason: it replaces more code than it costs in compile time
  and binary size. Prefer the crates listed in the implementation plan.
- No `rspotify`. No GUI or async runtime other than Slint and tokio.
- ~50 lines of clear code beats a dependency for a single small function.

### Comments and docs

- `///` on every `pub` item: one line saying what it is for. No restating the signature.
- Inline comments explain *why*, never *what*.
- Decisions with trade-offs go in `docs/decisions.md` (date, decision, reason), not in code.

### Commits

- One logical change per commit, imperative subject (`Add gateway coalescing`), with the
  milestone in the body when relevant.
- A commit leaves the workspace building, formatted, lint-clean, and tested.
