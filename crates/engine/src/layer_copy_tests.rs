//! Focused invariants for the non-destructive layer tree clipboard.
use crate::Session;
use anicraft_doc::{Element, ElementId, ElementKind, Keyframe, Layer, LayerId, LayerKind, Paint, Rgba, Shape};
use serde_json::json;
use std::sync::Arc;

fn run(s: &mut Session, cmd: &str, args: serde_json::Value) -> serde_json::Value {
    s.execute(cmd, &args).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

#[test]
fn duplicate_keeps_content_but_allocates_new_element_ids() {
    let mut s = Session::default();
    let original = s.layer;
    let element_id = ElementId(s.doc.alloc_id());
    let shape = Element { id: element_id, name: "drawing".into(), transform: Default::default(), color: Default::default(), kind: ElementKind::Shape(Shape::filled(anicraft_geom::Path::rect(0.0, 0.0, 10.0, 10.0), Paint::Solid(Rgba::hex(0x88aaee)))) };
    let layer = s.doc.timeline_mut(s.scene).layer_mut(original).expect("layer");
    layer.description = "note".into();
    layer.keyframes = vec![Arc::new(Keyframe { elements: vec![Arc::new(shape)], ..Keyframe::new(0, 2) })];
    let id = LayerId(run(&mut s, "layer.duplicate", json!({"layer": original.0}))["layer"].as_u64().expect("new layer"));
    let src = s.timeline().layer(original).expect("source");
    let copied = s.timeline().layer(id).expect("copy");
    assert_eq!(copied.description, "note");
    assert_eq!(copied.keyframes[0].duration, src.keyframes[0].duration);
    assert_ne!(copied.keyframes[0].elements[0].id, src.keyframes[0].elements[0].id);
    assert_eq!(copied.keyframes[0].elements[0].name, "drawing");
    assert_eq!(s.timeline().layers.len(), 2);
}

#[test]
fn copy_paste_is_independent_of_edit_undo() {
    let mut s = Session::default();
    let original = s.layer;
    assert_eq!(run(&mut s, "layer.copy", json!({"layer": original.0}))["copied"], 1);
    assert_eq!(run(&mut s, "layer.paste", json!({"layer": original.0}))["layers"], 1);
    assert_eq!(s.timeline().layers.len(), 2);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.timeline().layers.len(), 1);
    assert!(s.layer_clipboard.is_some());
}

#[test]
fn folder_copy_keeps_descendant_parent_links() {
    let mut s = Session::default();
    let folder_id = LayerId(s.doc.alloc_id());
    let child_id = LayerId(s.doc.alloc_id());
    let mut folder = Layer::new(folder_id, "Folder");
    folder.kind = LayerKind::Folder;
    folder.keyframes.clear();
    let mut child = Layer::new(child_id, "Inside");
    child.parent = Some(folder_id);
    let tl = s.doc.timeline_mut(s.scene);
    tl.layers.splice(0..0, [Arc::new(folder), Arc::new(child)]);
    tl.repair_masks();
    assert_eq!(run(&mut s, "layer.duplicate", json!({"layer": folder_id.0}))["layers"], 2);
    let tl = s.timeline();
    assert_eq!(tl.layers.len(), 5);
    assert_eq!(tl.layers[0].kind, LayerKind::Folder);
    assert_eq!(tl.layers[1].parent, Some(tl.layers[0].id));
    assert_ne!(tl.layers[0].id, folder_id);
    assert_ne!(tl.layers[1].id, child_id);
}
