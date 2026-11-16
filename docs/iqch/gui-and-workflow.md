# Desktop GUI and workflow refinements

> **Last updated:** 2026-11-16. **Status:** Fork-specific additions layered on top of upstream AniCraft 0.3.0.

This page groups smaller workflow changes that are important for integration but do not justify changing upstream feature design documents. Larger topics have dedicated pages.

## Configurable default stage color

`ANICRAFT_DEFAULT_STAGE_COLOR` sets the **initial stage/background color** for a new document, and is used when showing the new-document template thumbnails. Accepted values are `#RRGGBB`, `RRGGBB` and `0xRRGGBB` (case insensitive for `0x`). An invalid value or a missing variable falls back to white.

Example Windows CMD:

```bat
set ANICRAFT_DEFAULT_STAGE_COLOR=#202020
scripts\build\build.cmd
```

The environment variable is evaluated by the running AniCraft process when a new document/template is instantiated. It does **not** retroactively recolor every already-saved document.

## Remote/API Control

A compact control widget is available in the upper-right area of the desktop UI. It exposes the control/remoting status and whether the AniCraft endpoint is available. Our VectorCraft exchange workflow uses AniCraft's default port `7980` and VC's default port `7979`. The control surface is a deliberate UI affordance; it does not authorize arbitrary Python execution by remote peers.

When troubleshooting a VC→AC send, first check that the receiver control endpoint is enabled and that the source application is sending to the correct destination. Details of supported object modes are in [vectorcraft-exchange.md](vectorcraft-exchange.md).

## Movie Explorer action icons and note windows

- Scene/layer rows have compact **edit-description** buttons; hover text tells which scene/layer will be edited.
- Export/import actions in the Movie Explorer header open JSON dialogs, with bundled icon fallbacks and optional named asset icons.
- Independent note windows can remain open for multiple rows; text changes apply immediately to document data, with Undo handling.

See [scene-and-layer-descriptions.md](scene-and-layer-descriptions.md) for data and merge semantics.

## Docked Python Console

The embedded Python Console is a dockable panel. It can be placed alongside Timeline or elsewhere using normal dock mechanics. The console is persistent for its interpreter session, with proper multiline REPL behavior, history, separate errors, Unicode and emoji support. It was added without turning AniCraft into a Qt application: Qt remains an optional co-host for external tools.

See [python-embedding.md](python-embedding.md).

## Asset-based icon search

To keep custom/pipeline menu icons separate from built-in GUI assets, `ANICRAFT_ASSETS` supplies additional icon search roots. The same naming convention can decorate Python menu items and chosen compact controls. Supported icon files are PNG/SVG. The search is by **name**, not by unrestricted local paths.

In the development iteration, compact icons were added for dock panel categories: Actions, Movie Explorer, History, Library, Output, Properties, Swatches. These coexist with upstream's general dockable-panel and collapsed-dock machinery; do not rewrite upstream panel behavior as a fork-only feature.

## Layer Properties additions

The Layer Properties dialog includes distinct **Before first frame** / **After last frame** selectors with `Off`, `Hold`, `Continue` modes, plus the original layer controls. Changes are applied as one command/Undo step. Scene duration can independently extend the playback length. Detailed semantics are in [layer-boundaries-and-scene-duration.md](layer-boundaries-and-scene-duration.md).

## Build-time and release workflow

The local Windows Release process builds with the `python` feature and eight jobs, stamps the executable build time in the active build-script implementation, and stages Python runtime files/scripts next to the `.exe`. The working fork was merged from a tested integration branch into the dated `stable_16112026` checkpoint and then into `main`.

Upstream is developing rapidly. Avoid interpreting a change in `.exe` size alone as breakage. Keep integration/verification work in a temporary branch, preserve a dated stable checkpoint, verify the desktop smoke test, and only then merge into `main`.

### Smoke test

1. Start desktop Release and create a document with a custom default stage color.
2. Check that templates show the same default color.
3. Open Layer Properties, toggle the before/after controls and Undo once.
4. Open the Movie Explorer, edit notes in two windows, export/import JSON.
5. Open the Python Console and test a pipeline menu with a custom icon.
6. Start a PySide6 tool window by callback and close it normally without hanging the app.
7. Enable remoting and send a selected VC object; inspect the result and error status.
8. Save/reopen `.anicraft`; check that notes, layer modes, scene duration, and VC source metadata persist.

## Non-goals

The changes here do not promise platform parity for Qt ownership, a cross-platform installer with an independently bundled Python standard library, a generic remote Python execution API, or preservation of every experimental upstream workflow. Those require separate release/CI verification.
