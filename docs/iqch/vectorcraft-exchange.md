# VectorCraft → AniCraft exchange bridge

> **Last updated:** 2026-11-16. **Status:** One-way object transfer implemented; the source mapping and transfer modes are part of the fork-specific contract.  
> **Caution:** This document describes the observable **behavioral protocol**. It deliberately does not invent an unverified byte-level JSON schema, network framing or undocumented payload keys.

## Goal

An artist can author vector or raster objects in VectorCraft (VC) and send selected content directly to AniCraft (AC), where it becomes editable artwork or a reusable Graphic Symbol. The workflow should preserve as much structure as possible and allow subsequent refreshes from the VC source without destroying AniCraft instance animation.

## Transport and direction

- Default VectorCraft remoting/control service port: `7979`.
- Default AniCraft remoting/control service port: `7980`.
- Direction implemented in this iteration: **VC → AniCraft**. Reverse synchronization into VectorCraft is **not** part of the present feature.
- Sender-side entry point: `anicraft.sendSelection`, also exposed from File → Export → Send Selection to AniCraft.
- Receiver-side import command includes `bridge.import_svg`.
- Geometry is transferred as SVG embedded in a request rather than by a file path that the receiver must open. Raster content may be embedded as PNG/JPEG data when supported by the object conversion.

These are the names used in our development baseline. For exact request/response JSON and framing, inspect the current sender and receiver source when modifying the transport; older examples should not be treated as normative packet formats.

## What is supported

The working bridge was tested with VC vector curves, raster images, and raster-derived Area/Hydra/NURBS region output. SVG paths remain editable after conversion where the AniCraft vector importer supports their shape/paint representation. Raster-bearing content is imported as raster-backed artwork or symbol content. Not every complex VectorCraft-specific feature can be represented faithfully by SVG/AniCraft.

### Transfer modes

| Mode | User-facing command | Intended result |
|---|---|---|
| `update` | Send to AniCraft (Update) | Create on first send or refresh content linked to the same VC source. |
| `copy` | Send Copy to AniCraft | Create an independent duplicate rather than replacing the source-linked definition. |
| `raw` | Send Raw Shapes to AniCraft | Transfer editable shapes without the ordinary per-object symbol wrapping where applicable. |

**Top-level VC groups are a special case across all three modes:** one selected group becomes **one AniCraft Graphic Symbol**, not a collection of unrelated top-level shapes. Its nested vector/raster members and transforms belong inside that symbol. The group-as-one-symbol invariant still applies in `raw` mode.

### Update semantics

AniCraft tracks VC source document/object identifiers in document `user_metadata`, under the namespace:

```text
_vectorcraft_symbol_sources_v1
```

The mapping identifies the AniCraft symbol definition associated with a VC source. When the same VC object/group is sent again in `update` mode, update the **shared symbol content** instead of deleting the symbol and recreating its instances. Preserve:

- AniCraft symbol ID used by existing instances.
- Instance placement/transforms in the scene.
- Existing timeline animation and animation keys of those instances.
- Multiple instances of the same symbol: all should see the refreshed definition.

That is the central difference between a *symbol refresh* and a destructive replace-paste. A `copy` operation must not silently reuse the same update slot and overwrite older copies.

### Structure and transforms

- Keep top-level selection boundaries: an independent VC object maps to an object/symbol; a top-level VC group maps to a single Graphic Symbol.
- Retain nested members inside the group as far as importers can represent them.
- Account for local-to-world transforms, and for bitmap positions and sizes; do not bake a group's transform twice.
- Embedded image data avoids references to temporary local image files on a different machine.
- When encountering unsupported curves, masks, exotic fills or other VC-specific constructs, record a warning or use a deliberate fallback rather than silently claiming full fidelity.

## Implementation and operational notes

The document's source mapping is persistent metadata. Treat the namespace as application-internal data and do not overwrite it with unrelated `anicraft.set_metadata()` calls; use merge/update helpers or reserved-key-aware code when adding other metadata.

Both applications should be running, with AniCraft's control endpoint available. Sending uses VC's selection; no separate user copy/paste is required. Connection refused means the receiver is not listening/reachable; it does not by itself mean that the document content was damaged.

## Suggested regression matrix

1. Single closed/open VC NURBS path → editable AC path.
2. Raster image → embedded raster; check alpha, dimensions, placement.
3. VC Area/NURBS region → converted result; note any fidelity limitations.
4. VC top-level group of mixed vector/raster objects → **exactly one** Graphic Symbol.
5. Send group again with `update` → existing AC instances retain ID, transform and keys; geometry inside the symbol changes.
6. Send same group with `copy` → a new independent item, with no accidental update of old copies.
7. Send group with `raw` → one Graphic Symbol, even though single non-group raw shapes need not be wrapped.
8. Two VC documents with overlapping object IDs → do not collide because document/source identity participates in the mapping.
9. Save and reopen AniCraft; update still locates source-linked symbol.
10. Missing connection, malformed SVG, unsupported mask/paint; clean error without loss of unrelated scene content.

## Limits and next iterations

This is not a general bidirectional document synchronization layer. Reverse AC → VC transport, arbitrary hierarchy reparenting, full fidelity for every VC Area/NURBS feature, live per-object subscriptions and general-purpose Python object/raster access are outside the implemented baseline. Revalidate the transport format before exposing it as a stable public API.
