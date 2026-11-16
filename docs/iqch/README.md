# AniCraft fork extensions — engineering notes

> **Last updated:** 2026-11-16. **Baseline:** `iqch/anicraft`, `main` after the verified `stable_16112026` integration (AniCraft 0.3.0).  
> **Scope:** Our fork-specific behavior, not a replacement for the upstream `docs/guide` and ADR documentation.

These notes document the additions developed alongside the VectorCraft integration. Keep them separate from upstream documentation so that routine upstream merges do not overwrite the explanations or blur upstream and fork features. Descriptions reflect the implemented 2026-11-16 code when independently checked; transport internals not established by source inspection are explicitly distinguished from behavioral contracts.

| Document | Subject | Principal implementation |
|---|---|---|
| [layer-boundaries-and-scene-duration.md](layer-boundaries-and-scene-duration.md) | Before/after layer time range, no authored content, independent scene duration | `crates/doc/src/lib.rs`, `crates/engine/src/cmds_layer.rs`, `crates/engine/src/cmds_scene.rs`, `src/layer_props.rs` |
| [scene-and-layer-descriptions.md](scene-and-layer-descriptions.md) | Notes, Movie Explorer editors, JSON export/merge, commands | `crates/doc/src/lib.rs`, `src/movie_explorer.rs` |
| [vectorcraft-exchange.md](vectorcraft-exchange.md) | One-way VC → AniCraft SVG/raster and symbol exchange; update/copy/raw behavior | bridge implementation, document user metadata, symbol library |
| [python-embedding.md](python-embedding.md) | Python 3.14, in-process API, REPL, menus, icons, PySide6 and Qt | `src/python.rs`, `scripts/build/build.cmd` |
| [gui-and-workflow.md](gui-and-workflow.md) | Default stage color, controls, docks, icons, run/build and other desktop polish | `crates/doc/src/lib.rs`, `src/movie_explorer.rs`, `src/python.rs`, desktop GUI |

## Compatibility and maintenance

- The parent repository is [storytold/anicraft](https://github.com/storytold/anicraft); these files cover only our additions in [iqch/anicraft](https://github.com/iqch/anicraft).
- Behavior is intended for the desktop Release build. Browser/WASM and non-Windows native Qt behavior may differ; see each page.
- A manual Windows Release smoke test confirmed the integrated build and the principal fork features. This is **not** an assertion that all CI jobs pass on all platforms.
- After upstream merges, recheck the commands, JSON fields, paths and GUI labels against the new source; update the date at the top of the affected document.
- In case of disagreement, implementation code and the active command registry are authoritative. Do not change serialized keys or protocol behavior solely to match prose.

## Quick checks

1. Confirm stage default via `ANICRAFT_DEFAULT_STAGE_COLOR` and a new document.
2. Open Layer Properties and inspect both outside-frame controls; test one Graphic Symbol spanning the boundary.
3. Open two independent scene/layer note windows in Movie Explorer, then export and merge JSON.
4. Send VectorCraft selection in each transfer mode and verify that an updated Graphic Symbol retains existing instances.
5. Open the Python Console, run `import anicraft`, check a menu from `userSetup.py`, and exercise a PySide6 window without running `QApplication.exec()`.

This directory is intentionally documentation-only. It contains no Rust changes or configuration overrides.
