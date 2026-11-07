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
