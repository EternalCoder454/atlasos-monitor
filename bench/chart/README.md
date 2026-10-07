# Chart benchmark

Measures what live charts cost on Qt Quick's software backend, the default
renderer (docs/DESIGN.md, Rendering). It settled the chart decision: a C++
`QQuickPaintedItem` drawn with the tricks below. `livechart.cpp` is the
reference for Telamon.Ui's `LiveChart`. It draws what the Go Atlas Monitor's
graph package draws: caption bands, a square grid, fill and line, a border.

Not part of the app build or CI. Build it in the dev container:

```sh
scripts/dev.sh bash -c 'cmake -S bench/chart -B build/chartbench -G Ninja -DCMAKE_BUILD_TYPE=Release && cmake --build build/chartbench'
```

## Running

- **Under Wayland, as AtlasOS runs it:** `bench/chart/run-kwin.sh 1 <args>` starts a
  private virtual KWin on the host (its own D-Bus, never the real desktop) and
  runs `chartbench` in the container against it. It prints one JSON line. KWin's
  `--scale` does not apply to `--virtual`, so for 1.5x pass
  `CHART_ENV="-e QT_SCALE_FACTOR=1.5"`, which renders the same pixels.
- **One chart in a loop:** `chartbench --micro 3000 --width 470 --height 110 --dpr 1.5`
  paints into a `QImage` with hot caches. It gives exact per-paint costs for
  comparing drawing methods. In the app each paint runs once a second with cold
  caches, about 3× slower.
- **Seams:** `chartbench --seam out/x` under `QT_QPA_PLATFORM=offscreen` saves
  the backing store (built up by partial updates) and a fresh full render, for
  `magick compare -metric AE`. The offscreen grab is the top-left window-size
  crop of the device buffer, so crop the fresh render to match.
- **KWin's CPU:** `run-kwin.sh` prints `kwin_cpu_ms` (the whole run, startup
  included) on stderr. Compare against an `--idle` run.

Options: `--charts N --cols C --width W --height H --interval MS --warmup S
--seconds S --idle --list --opaque --gpu --threadpool --grab FILE`, plus
`--micro PAINTS --dpr R` and `--seam PREFIX` above. `--list` feeds the charts
through a `values` list property, as Rust will. `--threadpool` keeps Qt's GUI
thread pool, which `chartbench` otherwise turns off like the app.

Drawing variants (environment, benchmark only): `CHART_LINE=round|miter|bevel|outline|cosmetic|cosmetic3|segments`
(default `segments`), `CHART_FILLAA=1`, `CHART_GRID=lines`, `CHART_NOCLIP=1`,
`CHART_SKIP=grid,fill,line,border,text`.

Live CPU numbers move with whatever else the machine is doing (the same case
read 3.8 and 6.3 ms/s in two batches). Compare cases within one batch, 3 runs
of 30 s each, medians.

## What was found (2026-10-02, i9-14900KF, Qt 6.11.2)

1. **Qt's GUI thread pool.** The raster engine splits every fill of 96 or more
   spans into jobs on a thread pool and waits for them. For chart-sized shapes
   the hand-off costs more than the fill. `QT_NO_GUI_THREADPOOL=1` turns it off
   (about 40% less process CPU in the stress run). It is not slower for big
   repaints either: at 1.5x, where every tick repaints the whole window, a
   frame took 7.6 ms without the pool and 19.5 ms with it (12 charts, medians
   of 3).
2. **Anti-aliased thin shapes.** Qt's anti-aliasing rasterizer keeps its cells
   in a sorted list per scanline (`gray_record_cell`). A long, nearly flat
   1.5 px line puts hundreds of cells in each row: 190 of a 238 µs paint.
   Drawing each segment as its own small quad (extended half a width at both
   ends) is pixel-equivalent to the round-joined stroke: 13 of 116,000 pixels
   differ beyond 8% fuzz. The fill under the line needs no anti-aliasing,
   because the line covers its only sloped edge (0 pixels differ). One chart,
   hot: 238 → 67 µs at 1x, 391 → 90 µs at 1.5x.
3. **Grid and border as filled rectangles.** Translucent 1-px lines go
   through Qt's per-pixel cosmetic line path; 1-device-pixel rectangles go
   through the span blender. On a CPU-page-sized chart (990×360) at 1.5x:
   350 → 272 µs hot, pixel-identical (2 pixels differ, at the border's
   corners). No change on small charts.
4. **Not worth it:**
   - **Forced partial updates:** `QSGSoftwareRenderer` turns them off when the
     device pixel ratio isn't a whole number, so at 1.5x every tick repaints
     and damages the whole window. `QSG_SOFTWARE_RENDERER_FORCE_PARTIAL_UPDATES=1`
     brings the damage back to the charts' rectangles, without seams (0 pixels
     differ after ~60 frames at 1.25, 1.5 and 1.75x). But it saved nothing:
     Atlas Monitor 11.2 vs 11.2 ms/s with 12 charts and 4.5 vs 4.5 with one;
     KWin's CPU was the same within noise. The rest of the window is flat
     rectangles, so repainting it is cheap. *In the app it is not:* the
     sidebar, text and cards made a whole-window repaint 11-13 ms/s on every
     page, and the app now forces partial updates (bench/pages, cdc1b68).
   - **Opaque charts:** no measurable change.
   - **The clip rectangle:** no cost to remove.
   - **The `values` list** instead of reading a ring buffer: costs the same.

Under virtual KWin, 12 charts at 1 Hz, medians of 3 × 30 s (process CPU),
before the grid change in 3:

| | Naive painted item | Tuned |
|---|---|---|
| 12 charts, 1x | 11.6 ms/s | 3.8 ms/s |
| 12 charts, 1.5x | 28.6 ms/s | 10.0 ms/s |
| 32 charts, 1.5x | 34.3 ms/s | 13.3 ms/s |
| Idle (no ticks) | | 0.006 ms/s |
