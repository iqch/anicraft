# Scene and layer descriptions (Movie Explorer)

> **Last updated:** 2026-11-16. **Status:** Implemented and stored inside `.anicraft` documents.

## Purpose

A production file needs text notes attached to the scene and layer being worked on: story context, tasks, editorial comments, or hand-off instructions. Notes should travel with the movie, rather than being stored only in a temporary GUI session.

## Persistence and identity

Both `Scene` and `Layer` include a UTF-8 `description: String` field. Empty descriptions are optional in serialized data. The two levels are separate: a scene note does not overwrite its layer notes, and renaming an item does not require moving its description to a different scene/layer.

- A **scene** is selected by its index inside the movie when executing commands.
- A **layer** is selected by its document layer ID. In the editor the title helps identify it by `Scene` or `Scene.Layer`.
- Movie Explorer presents the scene/layer hierarchy in document order, as well as nested symbol definitions, keyframes and instances.

Names alone are insufficient to identify layers in code because different scenes may have identically named layers.

## Editing in Movie Explorer

Each scene and layer row has an edit icon. The icon opens a dedicated note editor. Several note windows may stay open independently, allowing comparisons and copy/paste between unrelated scenes and layers.

Editors support Undo/Cut/Copy/Paste using the conventional shortcuts (`Ctrl+Z`, `Ctrl+X`, `Ctrl+C`, `Ctrl+V`). Text edits are applied to the document immediately; focus/gesture handling combines a continuous edit into a sensible document Undo step. The status line and tooltips emphasize that descriptions are saved without a separate Apply button. The windows themselves are transient UI state; the descriptions are persistent document data.

Opening the same scene/layer repeatedly targets the corresponding note editor rather than creating independent copies of one buffer.

## Engine commands

| Command ID | Function |
|---|---|
| `description.set` | Set the text for a scene or a particular layer (scene index and optional layer ID). |
| `description.export` | Return the scene/layer note hierarchy as JSON data. |
| `description.merge` | Merge a descriptions JSON hierarchy into the current movie. |

For `description.set`, source usage in `src/movie_explorer.rs` establishes this parameter form:

```json
{"scene": 0, "layer": 42, "text": "Animation reference and cleanup notes"}
```

Omit `layer` to target the scene description. Here `42` is an example ID, **not** a position in the displayed layer list.

The same command registry can be accessed through embedded Python's `anicraft.execute()`; the current Python API intentionally does not hand out mutable Rust document objects.

## Export and import of descriptions

The Movie Explorer header includes JSON export/import controls:

- **Export** (`description.export`): save the current complete scene/layer note hierarchy, with a default filename of `scene-descriptions.json`.
- **Import** (`description.merge`): read JSON and merge it into the current movie. Existing scenes are **not** deleted or replaced. Missing scenes/layers can be added in their hierarchy, and existing notes can be matched/updated according to the merge command's rules.

The import UI rejects a JSON file larger than 64 MiB, parses the file before submission and reports how many scenes and layers were added (`scenesAdded`, `layersAdded`). It should not clear existing animation content as a side effect of importing notes.

The **authoritative format** is the JSON returned by the currently running `description.export` command. Round-trip an exported file rather than constructing undocumented keys by guesswork; the schema may evolve during upstream integration.

## Recommended workflow

1. Open a working `.anicraft` movie and select Window → Movie Explorer.
2. Add a scene-level description for sequence goals; layer-level descriptions for artist- or shot-specific work.
3. Export `scene-descriptions.json` for sharing, revision, or review.
4. Import into an existing or newly extended movie to merge the hierarchy and notes.
5. Save the `.anicraft` document to persist the result.

## Regression cases

- Two note editors open at once, one for a scene and one for a layer; edits do not cross-contaminate.
- Scene and layer with the same name; notes stay attached to the correct ID.
- Empty and Unicode/emoji text; save/reopen round-trip.
- Export followed by merge into the same file; no duplicate hierarchy.
- Merge into a movie with pre-existing animated scenes; existing scene data and keyframes remain.
- Undo/Redo during continuous note edits.
- Document change while a note editor is open; stale editor must not mutate another item's description.

## Implementation anchors

- `crates/doc/src/lib.rs`: `Scene::description`, `Layer::description`.
- `src/movie_explorer.rs`: editor windows, edit icon, `description.set`, JSON file dialogs, status summaries.
- Engine command registry: `description.set`, `description.export`, `description.merge`.
