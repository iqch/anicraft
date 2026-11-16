# Layer time boundaries and scene duration

> **Last updated:** 2026-11-16. **Status:** Implemented in the AniCraft fork and present in `main`.

## Motivation

Layer artwork may be authored for only part of a scene, while a scene has an independent playback duration. We added explicit per-layer treatment of time *before its first authored frame* and *after its last authored frame*. It must be consistent in the stage preview, scrubbing, playback and rendered/exported frames.

## Data model

`Layer` persists two independent properties:

- `pre_visibility`: behavior before the authored range.
- `post_visibility`: behavior after the authored range.

They use `OutsideFrameMode`: `off`, `hold`, `continue`. The default is `off` for both properties. These fields are serialized with the document and handled by existing `.anicraft` loading/saving; older documents deserialize with the defaults. The names are **not** synonyms for the ordinary layer `visible` toggle.

### The three boundary modes

| Mode | Drawing outside authored range | Nested Graphic Symbol timeline |
|---|---|---|
| `off` | Hide | Not shown |
| `hold` | Show first/last authored appearance | Freeze at boundary time |
| `continue` | Show first/last authored appearance | Continue its animation with time |

`hold` and `continue` use the same boundary artwork; their distinction is how time is propagated into animated symbol instances. For a static drawing they can look identical.

The two controls are separate: one may choose `off` before the first frame and `continue` after the last, for example. **Gaps within the authored timeline remain empty**; boundary behavior does not paint over internal gaps.

A layer with no authored keyframes must produce no content, including outside its range. The settings must not manufacture geometry in an empty layer. A folder is an organizational container; do not treat it as a drawable layer.

## UI and commands

In **Layer Properties** (double-click a layer icon, use the context menu, or Modify → Timeline), use **Before first frame** and **After last frame**. The dialog submits `layer.set_properties` once, so adjusting multiple controls is one Undo operation; Cancel leaves the document unchanged.

The command registry defines:

```json
{
  "command": "layer.set_properties",
  "params": {
    "pre_visibility": "off",
    "post_visibility": "continue"
  }
}
```

Here `command`/`params` is an explanatory wrapper for the command ID and its parameters, **not a TCP wire-format specification**. `layer` may be supplied as a numeric layer ID; without it, the current layer is targeted. The command also accepts normal properties such as `visible`, `locked`, `name`, `type`, `color`, `outline`, `height`, and `onion`.

Use `layer.properties` to read the current settings. Accepted values are lowercase `off`, `hold`, `continue`; invalid strings are rejected.

## Independent scene duration

Each `Scene` stores `duration_frames`. It is a **minimum** length, not a destructive trim of its layers. The effective scene length is:

```text
max(timeline.frame_count(), duration_frames, 1)
```

A zero value is the legacy/automatic mode; for positive values, the scene can extend beyond the last authored keyframe. `scene.set_duration` accepts `frames: 0` or a positive integer up to 100000, with an optional scene index. The same effective scene length must be used by playback, preview, animation export and the scene timeline. Changing scene duration does not inject keyframes or silently delete material.

Example parameters:

```json
{"frames": 180, "scene": 0}
```

`scene` is a zero-based index. It is optional if the current scene is intended.

## Regression cases

1. Static layer with keys only in frames 11–30: test both sides with `off`, `hold`, `continue`.
2. Graphic Symbol that animates internally: verify that `hold` freezes its sub-timeline, while `continue` progresses.
3. Blank internal interval: confirm that none of the outside modes fills the gap.
4. Layer without authored keys: confirm a blank result across the scene.
5. Scene duration longer than any timeline: confirm export reaches the requested length and boundary behavior is visible.
6. Change modes with Layer Properties; Undo and Redo should restore both fields.
7. Save, reopen, and repeat the behavior checks.

## Implementation anchors

- `crates/doc/src/lib.rs`: `Layer`, `Scene`, `OutsideFrameMode`, `Scene::frame_count`.
- `crates/engine/src/cmds_layer.rs`: `layer.set_properties`, `layer.properties`.
- `crates/engine/src/cmds_scene.rs`: `scene.set_duration`.
- `src/layer_props.rs`: dropdowns and descriptions.
- Rendering/evaluation: layer time evaluation and nested symbol time behavior.

## Non-goals

These controls do not create more keyframes, replace ordinary show/hide, stretch tween ranges, or make an empty layer visible. Do not conflate scene length with a particular layer's frame span.
