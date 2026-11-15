//! Scene and layer notes for Movie Explorer. Notes live in the document model, not shell state.
//! Export is a small, versioned JSON tree; merge preserves existing artwork and layer ids.

use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{Document, Layer, LayerId, LayerKind, MAX_LAYER_DEPTH, MAX_SCENES, Scene, Timeline};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

const MAX_NOTE_BYTES: usize = 8 * 1024 * 1024;
const MAX_SCENE_LAYERS: usize = 1000;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct NotesFile {
    version: u32,
    scenes: Vec<SceneNotes>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SceneNotes {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    layers: Vec<LayerNotes>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LayerNotes {
    name: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    layers: Vec<LayerNotes>,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec { id: "description.set", label: "Edit Description", menu: "", shortcut: None, params: "{scene: index, layer?: id, text: string}", mutates: true, run: set },
        CommandSpec { id: "description.export", label: "Export Descriptions", menu: "", shortcut: None, params: "{} -> {version:1, scenes:[...]} nested scene and layer descriptions", mutates: false, run: export },
        CommandSpec { id: "description.merge", label: "Import Descriptions", menu: "", shortcut: None, params: "{version:1, scenes:[...]} merge by scene and hierarchical layer names, never delete existing work", mutates: true, run: merge },
    ]
}

fn scene_index(s: &Session, p: &Value) -> Result<usize> {
    let raw = p.get("scene").and_then(Value::as_u64).ok_or_else(|| EngineError::Param("'scene' must be a zero-based scene index".into()))?;
    usize::try_from(raw).ok().filter(|&i| i < s.doc.scenes.len()).ok_or_else(|| EngineError::State("scene no longer exists".into()))
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let index = scene_index(s, p)?;
    let text = p.get("text").and_then(Value::as_str).ok_or_else(|| EngineError::Param("'text' must be a string".into()))?;
    if text.len() > MAX_NOTE_BYTES {
        return Err(EngineError::Param("a description cannot exceed 8 MiB".into()));
    }
    let layer_id = match p.get("layer") {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.as_u64().ok_or_else(|| EngineError::Param("'layer' must be an integer layer id".into()))?),
    };
    let scene = s.doc.scenes.get_mut(index).ok_or_else(|| EngineError::State("scene no longer exists".into()))?;
    let scene = Arc::make_mut(scene);
    if let Some(id) = layer_id {
        let layer = scene.timeline.layer_mut(LayerId(id)).ok_or_else(|| EngineError::State("layer no longer exists".into()))?;
        layer.description = text.to_owned();
    } else {
        scene.description = text.to_owned();
    }
    Ok(json!({"scene": index, "layer": layer_id, "bytes": text.len()}))
}

fn kind_label(k: LayerKind) -> &'static str {
    match k {
        LayerKind::Normal => "normal",
        LayerKind::Folder => "folder",
        LayerKind::Mask => "mask",
        LayerKind::Guide => "guide",
        LayerKind::Camera => "camera",
        LayerKind::Armature => "armature",
    }
}

fn parse_kind(text: &str) -> Result<LayerKind> {
    match text {
        "" | "normal" => Ok(LayerKind::Normal),
        "folder" => Ok(LayerKind::Folder),
        "mask" => Ok(LayerKind::Mask),
        "guide" => Ok(LayerKind::Guide),
        "camera" => Ok(LayerKind::Camera),
        "armature" => Ok(LayerKind::Armature),
        _ => Err(EngineError::Param(format!("unknown layer kind '{text}'"))),
    }
}

fn layer_tree(tl: &Timeline, parent: Option<LayerId>) -> Vec<LayerNotes> {
    tl.layers.iter().filter(|layer| layer.parent == parent).map(|layer| LayerNotes {
        name: layer.name.clone(),
        kind: kind_label(layer.kind).into(),
        description: layer.description.clone(),
        layers: layer_tree(tl, Some(layer.id)),
    }).collect()
}

fn snapshot(doc: &Document) -> NotesFile {
    NotesFile {
        version: 1,
        scenes: doc.scenes.iter().map(|scene| SceneNotes {
            name: scene.name.clone(),
            description: scene.description.clone(),
            layers: layer_tree(&scene.timeline, None),
        }).collect(),
    }
}

fn export(s: &mut Session, _p: &Value) -> Result<Value> {
    serde_json::to_value(snapshot(&s.doc)).map_err(|e| EngineError::Format(e.to_string()))
}

fn validate_layer(node: &LayerNotes, depth: usize, count: &mut usize) -> Result<()> {
    *count += 1;
    if *count > MAX_SCENE_LAYERS || depth >= MAX_LAYER_DEPTH {
        return Err(EngineError::Param("description hierarchy exceeds layer count/depth limits".into()));
    }
    if node.name.trim().is_empty() || node.name.chars().count() > 128 || node.description.len() > MAX_NOTE_BYTES {
        return Err(EngineError::Param("invalid layer name or description size".into()));
    }
    let kind = parse_kind(&node.kind)?;
    if !node.layers.is_empty() && !matches!(kind, LayerKind::Folder | LayerKind::Mask | LayerKind::Guide) {
        return Err(EngineError::Param("only folders, masks and guides can contain layers".into()));
    }
    for child in &node.layers {
        if matches!(kind, LayerKind::Mask | LayerKind::Guide) && parse_kind(&child.kind)? != LayerKind::Normal {
            return Err(EngineError::Param("mask/guide children must be normal layers".into()));
        }
        validate_layer(child, depth + 1, count)?;
    }
    Ok(())
}

fn validate(tree: &NotesFile) -> Result<()> {
    if tree.version != 1 || tree.scenes.len() > MAX_SCENES {
        return Err(EngineError::Param("expected descriptions JSON version 1 with at most 1000 scenes".into()));
    }
    for scene in &tree.scenes {
        if scene.name.trim().is_empty() || scene.name.chars().count() > 256 || scene.description.len() > MAX_NOTE_BYTES {
            return Err(EngineError::Param("invalid scene name or description size".into()));
        }
        let mut count = 0;
        for layer in &scene.layers { validate_layer(layer, 0, &mut count)?; }
    }
    Ok(())
}

fn merge_layers(doc: &mut Document, scene_idx: usize, parent: Option<LayerId>, list: &[LayerNotes], created: &mut usize) -> Result<()> {
    for note in list {
        let kind = parse_kind(&note.kind)?;
        let hit = doc.scenes.get(scene_idx).and_then(|scene| scene.timeline.layers.iter()
            .find(|layer| layer.parent == parent && layer.name == note.name && layer.kind == kind).map(|layer| layer.id));
        let id = if let Some(id) = hit {
            id
        } else {
            let current = doc.scenes.get(scene_idx).map_or(0, |scene| scene.timeline.layers.len());
            if current >= MAX_SCENE_LAYERS {
                return Err(EngineError::State("scene has reached the 1000-layer limit".into()));
            }
            let id = LayerId(doc.alloc_id());
            let mut layer = Layer::new(id, note.name.clone());
            layer.kind = kind;
            layer.parent = parent;
            if matches!(kind, LayerKind::Folder | LayerKind::Camera) {
                layer.keyframes.clear();
            }
            let Some(scene) = doc.scenes.get_mut(scene_idx) else { return Err(EngineError::State("scene no longer exists".into())); };
            let timeline = &mut Arc::make_mut(scene).timeline;
            let insert_at = match parent {
                Some(pid) => timeline.layers.iter().position(|layer| layer.id == pid).map_or(timeline.layers.len(), |idx| timeline.group_range(idx).end),
                None => timeline.layers.len(),
            };
            timeline.layers.insert(insert_at, Arc::new(layer));
            *created += 1;
            id
        };
        if let Some(scene) = doc.scenes.get_mut(scene_idx) {
            if let Some(layer) = Arc::make_mut(scene).timeline.layer_mut(id) {
                layer.description.clone_from(&note.description);
            }
        }
        merge_layers(doc, scene_idx, Some(id), &note.layers, created)?;
    }
    Ok(())
}

fn merge(s: &mut Session, p: &Value) -> Result<Value> {
    let input: NotesFile = serde_json::from_value(p.clone()).map_err(|e| EngineError::Param(format!("invalid description hierarchy: {e}")))?;
    validate(&input)?;
    let mut scenes_added = 0;
    let mut layers_added = 0;
    for note in &input.scenes {
        let scene_idx = match s.doc.scenes.iter().position(|scene| scene.name == note.name) {
            Some(idx) => idx,
            None => {
                if s.doc.scenes.len() >= MAX_SCENES { return Err(EngineError::State("maximum number of scenes reached".into())); }
                s.doc.scenes.push(Arc::new(Scene::new(&note.name)));
                scenes_added += 1;
                s.doc.scenes.len() - 1
            }
        };
        if let Some(scene) = s.doc.scenes.get_mut(scene_idx) { Arc::make_mut(scene).description.clone_from(&note.description); }
        merge_layers(&mut s.doc, scene_idx, None, &note.layers, &mut layers_added)?;
        if s.doc.scenes.get(scene_idx).is_some_and(|scene| !scene.timeline.layers.iter().any(|layer| !matches!(layer.kind, LayerKind::Folder | LayerKind::Camera))) {
            let id = LayerId(s.doc.alloc_id());
            if let Some(scene) = s.doc.scenes.get_mut(scene_idx) {
                Arc::make_mut(scene).timeline.layers.push(Arc::new(Layer::new(id, "Layer 1")));
                layers_added += 1;
            }
        }
        if let Some(scene) = s.doc.scenes.get_mut(scene_idx) { Arc::make_mut(scene).timeline.repair_masks(); }
    }
    Ok(json!({"scenesAdded": scenes_added, "layersAdded": layers_added}))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_merges_without_removing_existing_layers() {
        let mut source = Session::default();
        let first = source.doc.scenes[0].timeline.layers[0].id;
        set(&mut source, &json!({"scene":0,"layer":first.0,"text":"layer notes"})).unwrap();
        set(&mut source, &json!({"scene":0,"text":"scene notes"})).unwrap();
        let data = export(&mut source, &json!({})).unwrap();
        let mut target = Session::default();
        let old = target.doc.scenes[0].timeline.layers[0].id;
        merge(&mut target, &data).unwrap();
        assert_eq!(target.doc.scenes[0].timeline.layers[0].id, old);
        assert_eq!(target.doc.scenes[0].timeline.layers[0].description, "layer notes");
        assert_eq!(target.doc.scenes[0].description, "scene notes");
    }

    #[test]
    fn import_creates_nested_layers_and_is_idempotent() {
        let data = json!({"version":1,"scenes":[{"name":"Scene 1","description":"a","layers":[{"name":"Folder","kind":"folder","layers":[{"name":"Inner","description":"b"}]}]}]});
        let mut session = Session::default();
        let original = session.doc.scenes[0].timeline.layers.len();
        merge(&mut session, &data).unwrap();
        assert_eq!(session.doc.scenes[0].timeline.layers.len(), original + 2);
        assert_eq!(session.doc.scenes[0].timeline.layers[2].parent, Some(session.doc.scenes[0].timeline.layers[1].id));
        merge(&mut session, &data).unwrap();
        assert_eq!(session.doc.scenes[0].timeline.layers.len(), original + 2);
    }
}
