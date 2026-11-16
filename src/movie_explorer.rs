//! Window › Movie Explorer (ADR 0058), a tab of the Library's dock: the movie as a tree of
//! scenes, layers, keyframes and what is on them, instances opening into their symbols, and the
//! Library's symbol definitions. The Show buttons hide kinds of rows; the search keeps matching
//! rows with the rows above them. A double-click (or Enter) goes there with `find.goto`, opening
//! symbols in place as a double-click on the stage does (ADR 0045). The tree comes from
//! `explorer::tree` (what `explorer.tree` pages for scripts), made again only when the document
//! or the view changes; the list draws only the rows in view.

use crate::a11y::{self, Named};
use crate::app::App;
use crate::icons::{self, Icon};
use crate::theme::{T, TARGET, Type, color, font, text};
use anicraft_engine::StateId;
use anicraft_engine::doc::{LayerId, LayerKind, SymbolKind, Timeline};
use anicraft_engine::explorer::{self, NodeKind, Options, Row, Show, Tree};
use anicraft_engine::find::Scope;
use egui::{Key, Rect, Sense, Stroke, StrokeKind, WidgetType, pos2, vec2};
use std::collections::{BTreeMap, HashSet};
use serde_json::{Value, json};
use std::sync::Arc;

/// What the Movie Explorer shows and holds between frames (view state).
#[derive(Default)]
pub struct ExplorerView {
    pub show: ShowState,
    /// Every scene (off: only the current one).
    pub this_scene: bool,
    pub hide_definitions: bool,
    pub filter: String,
    /// Open rows; `None` until a row is opened or closed (then: the top rows are open).
    pub expanded: Option<HashSet<String>>,
    /// The selected row, by key.
    pub selected: Option<String>,
    /// Independent text editor windows; their buffers survive dock/tab changes.
    editors: BTreeMap<EditorKey, NoteEditor>,
    /// One engine undo step for a continuous text-editing session.
    note_gesture: bool,
    /// Bumped when anything above changes, so the tree is made again.
    rev: u64,
    cache: Option<((StateId, usize, u64), Arc<Tree>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum EditorKey {
    Scene(usize),
    Layer(u64),
}

#[derive(Clone, Debug)]
struct NoteEditor {
    title: String,
    text: String,
    scene: usize,
    scene_name: String,
    layer: Option<u64>,
    open: bool,
}

/// The Show menu's kinds, kept as the engine's [`Show`] (all on to start).
#[derive(Default)]
pub struct ShowState(pub Show);

impl ExplorerView {
    fn options(&self) -> Options {
        Options {
            scope: if self.this_scene { Scope::Scene } else { Scope::Document },
            show: self.show.0,
            definitions: !self.hide_definitions,
            expanded: self.expanded.clone(),
            all: false,
            filter: self.filter.clone(),
        }
    }

    fn changed(&mut self) {
        self.rev += 1;
    }

    /// Opens or closes `row`.
    fn toggle(&mut self, rows: &[Row], key: &str) {
        let set = self.expanded.get_or_insert_with(|| rows.iter().filter(|r| r.open).map(|r| r.key.clone()).collect());
        if !set.remove(key) {
            set.insert(key.to_string());
        }
        self.changed();
    }
}

/// The tree for the current view, made again only when the document or the view changed.
fn tree(app: &mut App) -> Arc<Tree> {
    let key = (app.s.state, app.s.scene, app.explorer.rev);
    if let Some((k, t)) = &app.explorer.cache
        && *k == key
    {
        return t.clone();
    }
    let t = Arc::new(explorer::tree(&app.s.doc, app.s.scene, &app.explorer.options()));
    app.explorer.cache = Some((key, t.clone()));
    t
}

/// Window › Movie Explorer: the panel comes forward, wherever it is (ADR 0075).
pub fn open(app: &mut App) {
    app.show_panel(crate::dock::Panel::MovieExplorer);
    app.explorer.changed();
}

/// Opens every row above `depth` (screenshots).
pub fn expand_to(app: &mut App, depth: usize) {
    let all = explorer::tree(&app.s.doc, app.s.scene, &Options { all: true, ..app.explorer.options() });
    app.explorer.expanded = Some(all.rows.iter().filter(|r| r.depth < depth && r.children).map(|r| r.key.clone()).collect());
    app.explorer.changed();
}

fn kind_name(k: NodeKind) -> &'static str {
    match k {
        NodeKind::Scene => tr_noop!("Scene"),
        NodeKind::Definitions => tr_noop!("Symbol definitions"),
        NodeKind::Symbol(SymbolKind::Graphic) | NodeKind::Instance(SymbolKind::Graphic) => tr_noop!("Graphic"),
        NodeKind::Symbol(SymbolKind::MovieClip) | NodeKind::Instance(SymbolKind::MovieClip) => tr_noop!("Movie clip"),
        NodeKind::Symbol(SymbolKind::Button) | NodeKind::Instance(SymbolKind::Button) => tr_noop!("Button"),
        NodeKind::Layer(LayerKind::Folder) => tr_noop!("Folder"),
        NodeKind::Layer(LayerKind::Mask) => tr_noop!("Mask layer"),
        NodeKind::Layer(LayerKind::Guide) => tr_noop!("Guide layer"),
        NodeKind::Layer(LayerKind::Camera) => tr_noop!("Camera layer"),
        NodeKind::Layer(LayerKind::Armature) => tr_noop!("Armature layer"),
        NodeKind::Layer(LayerKind::Normal) => tr_noop!("Layer"),
        NodeKind::Frame => tr_noop!("Keyframe"),
        NodeKind::Script => tr_noop!("Frame script"),
        NodeKind::Handler => tr_noop!("Actions"),
        NodeKind::Sound => tr_noop!("Sound"),
        NodeKind::Text => tr_noop!("Text"),
        NodeKind::Bitmap => tr_noop!("Bitmap"),
    }
}

/// What a row says: its document text, or for rows without a name what they are.
fn row_text(r: &Row) -> String {
    match r.kind {
        NodeKind::Definitions => crate::i18n::t(kind_name(r.kind)).to_string(),
        NodeKind::Frame => match (r.frame, r.name.is_empty()) {
            // The label is document text, as written.
            (Some(f), false) => tr!("Frame {frame}: {label}", frame = f + 1, label = r.name),
            (Some(f), true) => tr!("Frame {frame}", frame = f + 1),
            _ => crate::i18n::t(kind_name(r.kind)).to_string(),
        },
        NodeKind::Handler => {
            let event = anicraft_engine::doc::actions::ButtonEvent::ALL.iter().find(|e| e.key() == r.name).map_or(r.name.as_str(), |e| crate::i18n::t(e.label()));
            tr!("On {event}: {actions}", event = event, actions = r.extra)
        }
        NodeKind::Instance(_) if !r.extra.is_empty() => tr!("{instance} ({symbol})", instance = r.extra, symbol = r.name),
        NodeKind::Bitmap if !r.extra.is_empty() => tr!("{instance} ({bitmap})", instance = r.extra, bitmap = r.name),
        _ if r.name.is_empty() => crate::i18n::t(kind_name(r.kind)).to_string(),
        _ => r.name.clone(),
    }
}

/// The row's marker: the Library's symbol square, or the icon the timeline uses.
fn marker(p: &egui::Painter, at: Rect, k: NodeKind, c: egui::Color32) {
    let symbol = |kind: SymbolKind| match kind {
        SymbolKind::Graphic => T.kind_graphic,
        SymbolKind::MovieClip => T.kind_movie_clip,
        SymbolKind::Button => T.kind_button,
    };
    match k {
        NodeKind::Symbol(s) | NodeKind::Instance(s) => {
            let r = at.shrink(1.0);
            p.rect_stroke(r, 2.0, Stroke::new(1.5, symbol(s)), StrokeKind::Inside);
            p.circle_filled(r.center(), 2.5, symbol(s));
        }
        NodeKind::Scene => icons::paint(p, at, Icon::Play, c),
        NodeKind::Definitions => icons::paint(p, at, Icon::Folder, c),
        NodeKind::Layer(LayerKind::Folder) => icons::paint(p, at, Icon::Folder, c),
        NodeKind::Layer(LayerKind::Mask) => icons::paint(p, at, Icon::Mask, c),
        NodeKind::Layer(LayerKind::Guide) => icons::paint(p, at, Icon::Guide, c),
        NodeKind::Layer(LayerKind::Camera) => icons::paint(p, at, Icon::Camera, c),
        NodeKind::Layer(LayerKind::Armature) => icons::paint(p, at, Icon::Bone, c),
        NodeKind::Layer(LayerKind::Normal) => {
            p.rect_stroke(at.shrink(2.0), 1.0, Stroke::new(1.2, c), StrokeKind::Inside);
        }
        NodeKind::Frame => {
            p.circle_filled(at.center(), 3.0, c);
        }
        NodeKind::Script | NodeKind::Handler => {
            p.text(at.center(), egui::Align2::CENTER_CENTER, "{ }", font(Type::DataSmall), T.accent_text);
        }
        NodeKind::Sound => icons::paint(p, at, Icon::Sound, c),
        NodeKind::Text => icons::paint(p, at, Icon::Type, c),
        NodeKind::Bitmap => icons::paint(p, at, Icon::Image, c),
    }
}

/// Display path of a real document layer, including containing folders.
fn layer_path(tl: &Timeline, id: LayerId) -> String {
    let mut names = Vec::new();
    let mut current = Some(id);
    for _ in 0..=anicraft_engine::doc::MAX_LAYER_DEPTH {
        let Some(layer) = current.and_then(|key| tl.layer(key)) else { break };
        names.push(layer.name.clone());
        current = layer.parent;
    }
    names.reverse();
    names.join(".")
}

fn editor_for_row(app: &App, row: &Row) -> Option<(EditorKey, NoteEditor)> {
    let scene_index = row.go.scene?;
    let scene = app.s.doc.scenes.get(scene_index)?;
    match row.kind {
        NodeKind::Scene => Some((EditorKey::Scene(scene_index), NoteEditor {
            title: scene.name.clone(),
            text: scene.description.clone(),
            scene: scene_index,
            scene_name: scene.name.clone(),
            layer: None,
            open: true,
        })),
        NodeKind::Layer(_) if row.go.symbol.is_none() && row.go.path.is_empty() => {
            let id = row.go.layer?;
            let layer = scene.timeline.layer(id)?;
            Some((EditorKey::Layer(id.0), NoteEditor {
                title: format!("{}.{}", scene.name, layer_path(&scene.timeline, id)),
                text: layer.description.clone(),
                scene: scene_index,
                scene_name: scene.name.clone(),
                layer: Some(id.0),
                open: true,
            }))
        }
        _ => None,
    }
}

/// Close editors when the application replaces the current document.
pub fn reset_editors(app: &mut App) {
    if app.explorer.note_gesture && app.s.in_gesture() { app.s.end_gesture(); }
    app.explorer.note_gesture = false;
    app.explorer.editors.clear();
}

/// Separate floating editor for every scene/layer selected in Movie Explorer.
/// egui's multiline TextEdit supplies standard Undo, Cut, Copy and Paste shortcuts.
pub fn editor_windows(app: &mut App, ctx: &egui::Context) {
    let mut edits = Vec::new();
    let mut text_focused = false;
    for (key, editor) in &mut app.explorer.editors {
        if app.s.doc.scenes.get(editor.scene).is_none_or(|scene| scene.name != editor.scene_name || editor.layer.is_some_and(|id| scene.timeline.layer(LayerId(id)).is_none())) {
            editor.open = false;
            continue;
        }
        let title = editor.title.clone();
        let mut changed = false;
        egui::Window::new(format!("Description: {title}"))
            .id(egui::Id::new(("movie_explorer_description", key)))
            .default_width(470.0)
            .default_height(290.0)
            .resizable(true)
            .open(&mut editor.open)
            .show(ctx, |ui| {
                ui.label(text(Type::Caption, title.as_str()));
                let response = ui.add_sized([ui.available_width().max(280.0), ui.available_height().max(170.0) - 22.0],
                    egui::TextEdit::multiline(&mut editor.text).desired_rows(12));
                changed = response.changed();
                text_focused |= response.has_focus();
                response.on_hover_text(tr!("Description is saved immediately. Undo / Cut / Copy / Paste: Ctrl+Z / Ctrl+X / Ctrl+C / Ctrl+V."));
            });
        if changed {
            edits.push((editor.scene, editor.layer, editor.text.clone()));
        }
    }
    app.explorer.editors.retain(|_, editor| editor.open);
    if !edits.is_empty() && !app.explorer.note_gesture && !app.s.in_gesture() {
        app.s.begin_gesture("Edit Description");
        app.explorer.note_gesture = true;
    }
    for (scene, layer, text) in edits {
        let mut params = json!({"scene": scene, "text": text});
        if let Some(id) = layer { params["layer"] = json!(id); }
        app.run("description.set", params);
    }
    if !text_focused && app.explorer.note_gesture {
        app.s.end_gesture();
        app.explorer.note_gesture = false;
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn export_json(app: &mut App) {
    let Some(data) = app.run("description.export", json!({})) else { return };
    let Some(path) = rfd::FileDialog::new().set_title("Export scene and layer descriptions").add_filter("JSON", &["json"]).set_file_name("scene-descriptions.json").save_file() else { return };
    match serde_json::to_string_pretty(&data).map_err(|e| e.to_string()).and_then(|s| std::fs::write(&path, s).map_err(|e| e.to_string())) {
        Ok(()) => app.status_ok(format!("Descriptions saved: {}", path.display())),
        Err(e) => app.status_err(format!("Could not export descriptions: {e}")),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn import_json(app: &mut App) {
    let Some(path) = rfd::FileDialog::new().set_title("Merge scene and layer descriptions").add_filter("JSON", &["json"]).pick_file() else { return };
    let result = std::fs::metadata(&path).map_err(|e| e.to_string()).and_then(|meta| {
        if meta.len() > 64 * 1024 * 1024 { Err("JSON description file exceeds 64 MiB".into()) }
        else { std::fs::read_to_string(&path).map_err(|e| e.to_string()) }
    }).and_then(|content| serde_json::from_str::<Value>(&content).map_err(|e| e.to_string()));
    match result {
        Ok(data) => {
            if let Some(summary) = app.run("description.merge", data) {
                let scenes = summary.get("scenesAdded").and_then(Value::as_u64).unwrap_or(0);
                let layers = summary.get("layersAdded").and_then(Value::as_u64).unwrap_or(0);
                app.status_ok(format!("Descriptions merged: {scenes} scenes, {layers} layers added"));
                app.explorer.changed();
            }
        }
        Err(e) => app.status_err(format!("Could not read descriptions: {e}")),
    }
}

/// Compact JSON action with a fixed hit target; optional ANICRAFT_ASSETS icon
/// overrides the bundled one. The tooltip identifies it without spending header width.
#[cfg(not(target_arch = "wasm32"))]
fn json_icon_button(ui: &mut egui::Ui, name: &str, fallback: Icon, tooltip: &str) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(25.0, 25.0), Sense::click());
    let response = response.on_hover_text(tooltip);
    if response.hovered() { ui.painter().rect_filled(rect, T.rc(), T.hover); }
    let glyph = rect.shrink(4.0);
    #[cfg(feature = "python")]
    let loaded = crate::python::paint_named_icon(ui, glyph, name);
    #[cfg(not(feature = "python"))]
    let loaded = false;
    if !loaded { icons::paint(ui.painter(), glyph, fallback, if response.hovered() { T.icon_hot } else { T.icon }); }
    response.clicked()
}

pub fn panel(app: &mut App, ui: &mut egui::Ui) {
    // The search, display options and two non-destructive JSON actions.
    let mut do_export = false;
    let mut do_import = false;
    ui.horizontal(|ui| {
        // Reserve room for the Show menu and our two JSON action buttons.
        let menu_w_id = ui.id().with("explorer_show_w");
        let menu_w = ui.data(|d| d.get_temp::<f32>(menu_w_id)).unwrap_or(72.0) + ui.spacing().item_spacing.x;
        let json_buttons_w = if cfg!(target_arch = "wasm32") { 0.0 } else { 2.0 * (25.0 + ui.spacing().item_spacing.x) };

        let search = ui.add(
            egui::TextEdit::singleline(&mut app.explorer.filter)
                .hint_text(tr!("Search"))
                .desired_width((ui.available_width() - menu_w - json_buttons_w).max(crate::theme::MIN_FIELD))
        );
        search.set_name(tr!("Search the Movie Explorer"));
        if search.changed() {
            app.explorer.changed();
        }
        let menu = ui.menu_button(tr!("Show"), |ui| {
            let mut changed = false;
            let s = &mut app.explorer.show.0;
            for (on, label, tip) in [
                (&mut s.text, tr!("Text"), tr!("Text fields")),
                (&mut s.symbols, tr!("Symbols"), tr!("Buttons, movie clips and graphics")),
                (&mut s.scripts, tr!("Scripts"), tr!("Frame scripts and instance actions")),
                (&mut s.media, tr!("Sounds and bitmaps"), tr!("Sounds on keyframes, and bitmaps")),
                (&mut s.frames, tr!("Layers and frames"), tr!("Off: what is on them, straight under the scene")),
            ] {
                changed |= ui.checkbox(on, label).on_hover_text(tip).changed();
            }
            ui.separator();
            changed |= ui.radio_value(&mut app.explorer.this_scene, false, tr!("All scenes")).changed();
            changed |= ui.radio_value(&mut app.explorer.this_scene, true, tr!("This scene")).changed();
            ui.separator();
            let mut defs = !app.explorer.hide_definitions;
            if ui.checkbox(&mut defs, tr!("Symbol definitions")).on_hover_text(tr!("Every symbol in the Library, with what is inside it")).changed() {
                app.explorer.hide_definitions = !defs;
                changed = true;
            }
            if changed {
                app.explorer.changed();
            }
        });
        let w = menu.response.rect.width();
        if ui.data(|d| d.get_temp::<f32>(menu_w_id)) != Some(w) {
            ui.data_mut(|d| d.insert_temp(menu_w_id, w));
            ui.ctx().request_discard("the Show button's width changed"); // i18n-exempt: a debug reason, never shown
        }
        menu.response.on_hover_text(tr!("What the Movie Explorer shows")).set_name(tr!("Show in the Movie Explorer"));
        #[cfg(not(target_arch = "wasm32"))]
        {
            do_export = json_icon_button(ui, "save_json", Icon::SaveJson, "Save descriptions and scene/layer hierarchy as JSON");
            do_import = json_icon_button(ui, "load_json", Icon::LoadJson, "Merge descriptions and missing scenes/layers from JSON");
        }
    });
    #[cfg(not(target_arch = "wasm32"))]
    {
        if do_export { export_json(app); }
        if do_import { import_json(app); }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (do_export, do_import);
    ui.add_space(4.0);
    let t = tree(app);
    if t.rows.is_empty() {
        ui.label(text(Type::Caption, if app.explorer.filter.trim().is_empty() { tr!("Nothing to show: turn on more Show buttons.") } else { tr!("Nothing matches the search.") }));
        return;
    }
    let summary = if t.truncated { trn!("The first {n} row", "The first {n} rows", t.rows.len()) } else { trn!("{n} row", "{n} rows", t.rows.len()) };
    ui.label(text(Type::Caption, summary));
    list(app, ui, &t);
}

/// The rows, only those in view drawn.
fn list(app: &mut App, ui: &mut egui::Ui, t: &Tree) {
    let mut toggle: Option<String> = None;
    let mut select: Option<String> = None;
    let mut go: Option<usize> = None;
    let mut edit_windows = Vec::new();
    let room = (ui.available_height() - 8.0).clamp(140.0, 320.0);
    let filtering = !app.explorer.filter.trim().is_empty();
    egui::ScrollArea::vertical().id_salt("explorer_rows").max_height(room).auto_shrink([false, true]).show_rows(ui, TARGET, t.rows.len(), |ui, range| {
        for i in range {
            let Some(r) = t.rows.get(i) else { continue };
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), TARGET), Sense::click());
            let on = app.explorer.selected.as_deref() == Some(r.key.as_str());
            let p = ui.painter();
            if on || resp.hovered() {
                p.rect_filled(rect, T.rc(), if on { T.accent_dim } else { T.hover });
            }
            if on {
                p.rect_filled(Rect::from_min_size(rect.left_top(), vec2(3.0, rect.height())), 0.0, T.accent);
            }
            let indent = 6.0 + r.depth.min(24) as f32 * 14.0;
            let chevron = Rect::from_center_size(pos2(rect.left() + indent + 6.0, rect.center().y), vec2(12.0, 12.0));
            let icon = Rect::from_center_size(pos2(rect.left() + indent + 22.0, rect.center().y), vec2(14.0, 14.0));
            if r.children && !filtering {
                icons::paint(p, chevron, if r.open { Icon::ChevronDown } else { Icon::ChevronRight }, T.icon);
            }
            marker(p, icon, r.kind, color(Type::Body));
            let label = row_text(r);
            let edit_target = editor_for_row(app, r);
            // Keep the editor inside the visible row and beside its name, not at a
            // virtual ScrollArea row's far right (which may be clipped away).
            let edit_rect = Rect::from_center_size(pos2(icon.right() + 13.0, rect.center().y), vec2(23.0, 23.0));
            let edit_response = edit_target.as_ref().map(|(_, editor)| {
                let response = ui.interact(edit_rect, ui.id().with(("description.edit", r.key.as_str())), Sense::click());
                if response.hovered() {
                    ui.painter().rect_filled(edit_rect, T.rc(), T.hover);
                }
                icons::paint(ui.painter(), edit_rect.shrink(5.0), Icon::Pencil, if response.hovered() { T.icon_hot } else { T.icon });
                response.on_hover_text(editor.title.as_str())
            });
            let name_left = if edit_target.is_some() { edit_rect.right() + 4.0 } else { icon.right() + 8.0 };
            let name_rect = Rect::from_min_max(pos2(name_left, rect.top()), pos2(rect.right().max(name_left), rect.bottom()));
            let g = p.layout_no_wrap(label.clone(), font(Type::Body), color(Type::Body));
            p.with_clip_rect(name_rect.intersect(ui.clip_rect())).galley(pos2(name_rect.left(), rect.center().y - g.rect.height() / 2.0), g, color(Type::Body));
            let kind = crate::i18n::t(kind_name(r.kind));
            let name = match (r.children && !filtering, r.open) {
                (true, true) => tr!("{kind}: {row}, open", kind = kind, row = label),
                (true, false) => tr!("{kind}: {row}, closed", kind = kind, row = label),
                _ => tr!("{kind}: {row}", kind = kind, row = label),
            };
            a11y::control(ui, &resp, WidgetType::SelectableLabel, &name, Some(on));
            let resp = resp.on_hover_text(tr!("{row}\nDouble-click to go there", row = name));
            let clicked_edit = edit_response.as_ref().is_some_and(egui::Response::clicked);
            if clicked_edit {
                if let Some(editor) = edit_target {
                    edit_windows.push(editor);
                }
            }
            if resp.clicked() && !clicked_edit {
                let on_chevron = ui.input(|inp| inp.pointer.interact_pos()).is_some_and(|q| q.x < icon.left());
                if on_chevron && r.children && !filtering {
                    toggle = Some(r.key.clone());
                }
                select = Some(r.key.clone());
            }
            if resp.double_clicked() && !clicked_edit {
                go = Some(i);
            }
            if resp.has_focus() && ui.input(|inp| inp.key_pressed(Key::Enter)) {
                go = Some(i);
            }
        }
    });
    for (key, editor) in edit_windows {
        if let Some(existing) = app.explorer.editors.get_mut(&key) {
            existing.open = true;
        } else {
            app.explorer.editors.insert(key, editor);
        }
    }
    if let Some(k) = select {
        app.explorer.selected = Some(k);
    }
    if let Some(k) = toggle {
        app.explorer.toggle(&t.rows, &k);
    }
    if let Some(r) = go.and_then(|i| t.rows.get(i)) {
        goto(app, r);
    }
}

/// Goes where row `r` is.
fn goto(app: &mut App, r: &Row) {
    app.explorer.selected = Some(r.key.clone());
    if r.kind == NodeKind::Definitions {
        return;
    }
    app.playing = false;
    if app.run("find.goto", r.go.to_json()).is_some() {
        app.status_ok(tr!("Went to {row}", row = row_text(r)));
    }
}
