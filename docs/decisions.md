# Decisions

| Date | Decision | Reason |
| --- | --- | --- |
| 2026-11-07 | Slint `=1.18.1` instead of the plan's 1.17.x; workspace `rust-version = "1.92"` | 1.18.1 is the current stable patch (Sep 21); it requires Rust 1.92, above AGENTS.md's 1.85 floor. |
| 2026-11-07 | Software renderer by default; FemtoVG stays behind the `ui/femtovg` feature | Spike: FemtoVG costs ~43 MB more private working set for a modest CPU saving. Numbers in `measurements.md`. Confirm in M3 with the real UI. |
| 2026-11-07 | The librespot session logs in with Spotify's desktop client ID (`streaming` scope, redirect `127.0.0.1:8898/login`); Hush's own Client ID is used for the Web API only | A token from Hush's Client ID passes the access point, but clienttoken rejects Hush's ID (HTTP 400) and login5 rejects the stored credential it yields under the default client ID (`INVALID_CREDENTIALS`), so no track loads. The session login runs once; librespot then reuses its cached credentials. |
| 2026-11-07 | Session login reuses `auth`'s PKCE code instead of `librespot-oauth` | `librespot-oauth`'s async login blocks a tokio worker on a std `TcpListener` and builds its own `reqwest::Client`. |
| 2026-11-07 | `vergen` locked to 9.0.6 in `Cargo.lock` | `librespot-core` 0.8.0's build script fails with vergen 9.1 (incompatible `vergen-lib`). |
| 2026-11-07 | Volume uses librespot's `SoftMixer`; the player gets `NoOpVolume` and `EqSink` reads the mixer's `VolumeGetter`, instead of a custom `AtomicMixer` | `SoftMixer` already keeps the mapped volume in an atomic and Spirc drives it; a custom mixer would duplicate it. Volume still lands after the EQ. |
| 2026-11-07 | `EqSink` streams 44.1 kHz to cpal 0.18 and lets Windows resample, instead of a resampler crate | cpal 0.18 opens WASAPI shared-mode output with `AUTOCONVERTPCM` and `SRC_DEFAULT_QUALITY`, so the audio engine converts to the device rate (usually 48 kHz). cpal 0.16 did not, which would have needed `rubato`. |
| 2026-11-07 | The safety limiter is a static soft-knee curve (knee at −1 dBFS, `tanh` above it); normalisation keeps librespot's dynamic limiter | It only has to catch EQ boosts that pass full scale, so it needs no attack/release state and cannot pump. |
