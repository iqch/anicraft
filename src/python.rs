//! Minimal embedded CPython 3.14 host for scripting, menus and persistent metadata.
//! There is deliberately no object, raster, subscription or remote-code execution API.
use crate::app::App;
use pyo3::{exceptions::{PyRuntimeError, PyValueError}, prelude::*, types::{PyDict, PyModule}};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::HashMap, ffi::CString, path::{Path, PathBuf}, io::Write, sync::{OnceLock, atomic::{AtomicUsize, Ordering}}};
use serde::Deserialize;

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
struct ItemOptions {
    icon: String,
    bold: bool,
    italic: bool,
    enabled: Option<bool>,
    color: Option<[u8; 3]>,
    monochrome: bool,
    icon_color: Option<[u8; 4]>,
}
#[derive(Clone, Default)]
struct MenuOptions {
    icon: String,
    monochrome: bool,
    icon_color: Option<[u8; 4]>,
}
struct Item {
    path: Vec<String>,
    label: String,
    callback: Py<PyAny>,
    options: ItemOptions,
}
#[derive(Clone)]
struct ConsoleLine {
    text: String,
    error: bool,
    prompt: bool,
}
#[derive(Default)]
struct Host {
    started: bool,
    session: Option<anicraft_engine::Session>,
    paths: Vec<Vec<String>>,
    roots: Vec<(String, String)>,
    menu_options: HashMap<Vec<String>, MenuOptions>,
    items: Vec<Item>,
    log: Vec<String>,
    errors: Vec<String>,
    open_console: bool,
    pump: bool,
    output: Vec<ConsoleLine>,
    output_pending: String,
    output_pending_error: bool,
    input: String,
    continuation: bool,
    history: Vec<String>,
    history_index: Option<usize>,
    history_draft: String,
    shown_once: bool,
}
thread_local! { static HOST: RefCell<Host> = RefCell::new(Host::default()); }
thread_local! { static ICON_CACHE: RefCell<HashMap<PathBuf, Option<IconAsset>>> = RefCell::new(HashMap::new()); }

// Temporary, opt-in diagnostics for the transition from Start Screen to a document.
// The log is opened and closed for each line, so a UI freeze still leaves readable evidence.
static DIAG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
static DIAG_FRAMES: AtomicUsize = AtomicUsize::new(0);
static DIAG_LINES: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn trace_document(event: &str) {
    DIAG_FRAMES.store(24, Ordering::Relaxed);
    trace(event);
}

pub(crate) fn trace(event: &str) {
    if DIAG_FRAMES.load(Ordering::Relaxed) == 0 && !event.starts_with("python.start") { return; }
    let Some(path) = DIAG_PATH.get_or_init(|| std::env::var_os("ANICRAFT_DIAG_LOG").filter(|s| !s.is_empty()).map(PathBuf::from)).as_ref() else { return; };
    if DIAG_LINES.fetch_add(1, Ordering::Relaxed) >= 1200 { return; }
    let millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{millis} | {:?} | {event}", std::thread::current().id());
    }
}

pub(crate) fn trace_frame_done() {
    if DIAG_FRAMES.load(Ordering::Relaxed) != 0 {
        trace("frame.end");
        DIAG_FRAMES.fetch_sub(1, Ordering::Relaxed);
    }
}

fn host<T>(f: impl FnOnce(&mut Host) -> PyResult<T>) -> PyResult<T> {
    HOST.with(|cell| {
        let mut h = cell.try_borrow_mut().map_err(|_| PyRuntimeError::new_err("Reentrant Python host call"))?;
        f(&mut h)
    })
}

fn push_line(h: &mut Host, text: String, error: bool) {
    if h.output.len() >= 2000 { h.output.remove(0); }
    h.output.push(ConsoleLine { text: text.chars().take(16384).collect(), error, prompt: false });
}

fn stream(h: &mut Host, value: &str, error: bool) {
    for part in value.chars().take(65536) {
        if error != h.output_pending_error && !h.output_pending.is_empty() {
            let previous = std::mem::take(&mut h.output_pending);
            let previous_error = h.output_pending_error;
            push_line(h, previous, previous_error);
        }
        h.output_pending_error = error;
        if part == '\n' {
            let line = std::mem::take(&mut h.output_pending);
            push_line(h, line, error);
        } else if part != '\r' {
            h.output_pending.push(part);
            if h.output_pending.len() > 16384 {
                let line = std::mem::take(&mut h.output_pending);
                push_line(h, line, error);
            }
        }
    }
}

fn valid_path(path: &[String]) -> bool {
    !path.is_empty() && path.len() <= 8 && path.iter().all(|p| !p.trim().is_empty() && p.len() <= 128)
}

#[pyfunction]
fn _execute(command: String, params: String) -> PyResult<String> {
    if params.len() > 8 * 1024 * 1024 { return Err(PyValueError::new_err("Request too large")); }
    let payload: Value = serde_json::from_str(&params).map_err(|e| PyValueError::new_err(e.to_string()))?;
    host(|h| {
        let session = h.session.as_mut().ok_or_else(|| PyRuntimeError::new_err("execute must run during an AniCraft callback"))?;
        session.execute(&command, &payload).map(|v| v.to_string()).map_err(|e| PyRuntimeError::new_err(e.to_string()))
    })
}

#[pyfunction]
fn _metadata(value: Option<String>) -> PyResult<String> {
    host(|h| {
        let session = h.session.as_mut().ok_or_else(|| PyRuntimeError::new_err("metadata requires an AniCraft callback"))?;
        if let Some(data) = value {
            if data.len() > 8 * 1024 * 1024 { return Err(PyValueError::new_err("Metadata too large")); }
            let parsed = serde_json::from_str(&data).map_err(|e| PyValueError::new_err(e.to_string()))?;
            session.set_user_metadata(parsed);
        }
        let current = &session.doc.user_metadata;
        Ok(if current.is_null() { "{}".to_owned() } else { current.to_string() })
    })
}

#[pyfunction]
fn _register_menu(path: Vec<String>, before: String) -> PyResult<()> {
    if !valid_path(&path) { return Err(PyValueError::new_err("Invalid menu path")); }
    host(|h| {
        if h.paths.len() >= 128 && !h.paths.contains(&path) { return Err(PyValueError::new_err("Menu limit reached")); }
        if !h.paths.contains(&path) { h.paths.push(path.clone()); }
        if path.len() == 1 && !h.roots.iter().any(|(name, _)| name == &path[0]) {
            h.roots.push((path[0].clone(), before));
        }
        Ok(())
    })
}

#[pyfunction]
fn _menu_icon(path: Vec<String>, icon: String, monochrome: bool, icon_color: Option<Vec<u8>>) -> PyResult<()> {
    if !valid_path(&path) { return Err(PyValueError::new_err("Invalid menu path")); }
    let color = match icon_color.as_deref() {
        None => None,
        Some([r, g, b, a]) => Some([*r, *g, *b, *a]),
        _ => return Err(PyValueError::new_err("icon_color must be RGB/RGBA")),
    };
    host(|h| {
        if !h.paths.contains(&path) { return Err(PyValueError::new_err("Menu not registered")); }
        h.menu_options.insert(path, MenuOptions { icon, monochrome, icon_color: color });
        Ok(())
    })
}

#[pyfunction]
fn _asset_path(name: String) -> PyResult<String> {
    Ok(find_icon(&name).map_or_else(String::new, |p| p.display().to_string()))
}

#[pyfunction]
fn _menu_paths() -> PyResult<Vec<Vec<String>>> {
    host(|h| Ok(h.paths.clone()))
}

#[pyfunction]
fn _add_item(path: Vec<String>, label: String, callback: Py<PyAny>, options: String) -> PyResult<()> {
    if !valid_path(&path) || label.trim().is_empty() || label.len() > 128 {
        return Err(PyValueError::new_err("Invalid menu item"));
    }
    let options: ItemOptions = serde_json::from_str(&options).map_err(|e| PyValueError::new_err(e.to_string()))?;
    host(|h| {
        if h.items.len() >= 256 { return Err(PyValueError::new_err("Menu item limit reached")); }
        if !h.paths.contains(&path) { h.paths.push(path.clone()); }
        if !h.roots.iter().any(|(name, _)| name == &path[0]) {
            h.roots.push((path[0].clone(), "Help".into()));
        }
        h.items.push(Item { path, label, callback, options });
        Ok(())
    })
}

#[pyfunction]
fn _log(message: String) -> PyResult<()> {
    host(|h| {
        if h.log.len() >= 1000 { h.log.remove(0); }
        h.log.push(message.chars().take(16384).collect());
        stream(h, &(message + "\n"), false);
        Ok(())
    })
}

#[pyfunction]
fn _stream(text: String, error: bool) -> PyResult<()> {
    host(|h| { stream(h, &text, error); Ok(()) })
}

#[pyfunction]
fn _show_log() -> PyResult<()> {
    host(|h| { h.open_console = true; Ok(()) })
}

#[pyfunction]
fn _enable_pump() -> PyResult<()> {
    host(|h| { h.pump = true; Ok(()) })
}

// Files are searched in caller-specified order. Never interpret an icon name as a path:
// this also prevents scripts from reading arbitrary files through the menu renderer.
fn find_icon(name: &str) -> Option<PathBuf> {
    // ANICRAFT_ASSETS is a semicolon-separated search path, in priority order.
    // Entries may identify either a resource root or its icons directory.
    if name.is_empty() || name.len() > 128 || name.contains('/') || name.contains('\\') || name.contains("..") || name.contains(':') { return None; }
    let candidates: Vec<String> = if name.to_ascii_lowercase().ends_with(".png") || name.to_ascii_lowercase().ends_with(".svg") {
        vec![name.to_owned()]
    } else { vec![format!("{name}.png"), format!("{name}.svg")] };
    let mut roots: Vec<PathBuf> = std::env::var("ANICRAFT_ASSETS").unwrap_or_default()
        .split(';').map(str::trim).map(|s| s.trim_matches('"')).filter(|s| !s.is_empty()).map(PathBuf::from).collect();
    // Also resolve the built-in icons for scripts started without a custom asset path.
    if let Ok(cwd) = std::env::current_dir() { roots.push(cwd.join("assets")); }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.join("assets"));
            if let Some(project) = dir.parent().and_then(|p| p.parent()) { roots.push(project.join("assets")); }
        }
    }
    for root in roots {
        let directory = if root.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.eq_ignore_ascii_case("icons")) { root } else { root.join("icons") };
        for filename in &candidates {
            let candidate = directory.join(filename);
            if candidate.is_file() { return Some(candidate); }
        }
    }
    None
}

#[derive(Clone)]
enum IconAsset {
    Png(egui::TextureHandle),
    Svg(Vec<anicraft_engine::geom::Path>),
}

fn load_icon(ctx: &egui::Context, path: &Path) -> Option<IconAsset> {
    // Each UI icon is decoded once. Oversized/invalid resources are ignored.
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() > 2 * 1024 * 1024 { return None; }
    if path.extension().and_then(|v| v.to_str()).is_some_and(|s| s.eq_ignore_ascii_case("png")) {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().ok()?;
        let info = reader.info();
        let (w, h) = (info.width as usize, info.height as usize);
        if w == 0 || h == 0 || w > 1024 || h > 1024 { return None; }
        let mut buf = vec![0; reader.output_buffer_size()];
        let output = reader.next_frame(&mut buf).ok()?;
        let raw = &buf[..output.buffer_size()];
        let mut rgba = Vec::with_capacity(w * h * 4);
        match output.color_type {
            png::ColorType::Rgba => rgba.extend_from_slice(raw),
            png::ColorType::Rgb => for pixel in raw.chunks_exact(3) { rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]); },
            png::ColorType::Grayscale => for &gray in raw { rgba.extend_from_slice(&[gray, gray, gray, 255]); },
            png::ColorType::GrayscaleAlpha => for px in raw.chunks_exact(2) { rgba.extend_from_slice(&[px[0], px[0], px[0], px[1]]); },
            png::ColorType::Indexed => return None,
        }
        if rgba.len() != w * h * 4 { return None; }
        let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
        let tex = ctx.load_texture(format!("anicraft:python-icon:{}", path.display()), image, egui::TextureOptions::LINEAR);
        Some(IconAsset::Png(tex))
    } else {
        let svg = String::from_utf8(bytes).ok()?;
        let paths = crate::icons::parse_svg(&svg);
        (!paths.is_empty() && paths.len() <= 512).then_some(IconAsset::Svg(paths))
    }
}

fn paint_menu_icon(ui: &egui::Ui, button: egui::Rect, icon: &str, monochrome: bool, color: Option<[u8; 4]>, right: bool) {
    let Some(path) = find_icon(icon) else { return; };
    let asset = ICON_CACHE.with(|cell| {
        let mut cache = cell.borrow_mut();
        cache.entry(path.clone()).or_insert_with(|| load_icon(ui.ctx(), &path)).clone()
    });
    let Some(asset) = asset else { return; };
    let side = 16.0f32.min((button.height() - 3.0).max(1.0));
    let left = button.left() + if right { 5.0 } else { 7.0 };
    let rect = egui::Rect::from_min_size(egui::pos2(left, button.center().y - side * 0.5), egui::vec2(side, side));
    let tint = color.map_or_else(|| ui.visuals().text_color(), |[r, g, b, a]| egui::Color32::from_rgba_unmultiplied(r, g, b, a));
    match asset {
        IconAsset::Png(tex) => {
            ui.painter().image(tex.id(), rect, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), if monochrome { tint } else { egui::Color32::WHITE });
        },
        IconAsset::Svg(paths) => {
            use anicraft_engine::geom::Affine;
            let bounds = paths.iter().map(|p| p.bounds()).filter(|b| !b.is_empty()).collect::<Vec<_>>();
            let min_x = bounds.iter().map(|b| b.min.x).fold(f32::INFINITY, f32::min);
            let min_y = bounds.iter().map(|b| b.min.y).fold(f32::INFINITY, f32::min);
            let max_x = bounds.iter().map(|b| b.max.x).fold(f32::NEG_INFINITY, f32::max);
            let max_y = bounds.iter().map(|b| b.max.y).fold(f32::NEG_INFINITY, f32::max);
            let span = (max_x - min_x).max(max_y - min_y);
            if !span.is_finite() || span <= 0.0 { return; }
            let scale = (side - 2.0) / span;
            let ox = rect.center().x - (min_x + max_x) * scale / 2.0;
            let oy = rect.center().y - (min_y + max_y) * scale / 2.0;
            let matrix = Affine::translate(ox, oy).mul(&Affine::scale(scale, scale));
            for p in paths {
                for line in p.flatten(&matrix, 0.25) {
                    let pts: Vec<egui::Pos2> = line.points.iter().map(|v| egui::pos2(v.x, v.y)).collect();
                    if pts.len() > 1 {
                        let stroke = egui::Stroke::new(1.3, tint);
                        if line.closed { ui.painter().add(egui::Shape::closed_line(pts, stroke)); }
                        else { ui.painter().add(egui::Shape::line(pts, stroke)); }
                    }
                }
            }
        }
    }
}

/// Paint a named image from ANICRAFT_ASSETS into a compact toolbar rect.
/// Return false when the name cannot be resolved (the caller paints a bundled icon).
pub(crate) fn paint_named_icon(ui: &egui::Ui, rect: egui::Rect, name: &str) -> bool {
    if find_icon(name).is_none() { return false; }
    paint_menu_icon(ui, rect.expand2(egui::vec2(7.0, 0.0)), name, false, None, false);
    true
}

pub(crate) fn root_label(title: &str, label: &str) -> String {
    let icon = HOST.with(|h| h.borrow().menu_options.get(&vec![title.to_owned()]).map(|o| o.icon.clone()).unwrap_or_default());
    if find_icon(&icon).is_some() {
        // A real image is drawn in the left 16-point gutter by decorate_root.
        format!("        {label}")
    } else if icon.chars().any(|c| !c.is_ascii()) && icon.chars().count() <= 4 {
        format!("{icon}  {label}")
    } else {
        label.to_owned()
    }
}

pub(crate) fn decorate_root(ui: &egui::Ui, response: &egui::Response, title: &str) {
    let opts = HOST.with(|h| h.borrow().menu_options.get(&vec![title.to_owned()]).cloned());
    if let Some(opts) = opts { paint_menu_icon(ui, response.rect, &opts.icon, opts.monochrome, opts.icon_color, true); }
}

// Kept as plain Python to preserve the existing VectorCraft-style setup script.
// Python functions intentionally expose no handles to AniCraft document objects.
const BOOTSTRAP: &str = r#"
import asyncio as _asyncio
import code as _code
import json as _json
import os as _os
import sys as _sys
import traceback as _traceback
import types as _types
import time as _time
from _anicraft import _execute, _metadata, _register_menu, _menu_paths, _menu_icon, _add_item, _asset_path, _log, _show_log, _enable_pump, _stream

m = _types.ModuleType('anicraft')
_loop_instance = None
_tasks = set()
_qt_app = None
_callbacks = []
_external_windows = {}
_external_filter = None
_qt_processing = False
_pump_active = False
_qt_last_native_pump = 0.0
_console_globals = {'__name__':'__anicraft_console__', '__builtins__':__builtins__, 'anicraft':m}
_interactive_console = _code.InteractiveConsole(locals=_console_globals)

class _Writer:
	def __init__(self, origin, error=False):
		self.origin, self.error = origin, error
		self.encoding = 'utf-8'
		return
	def write(self, value):
		text = str(value)
		if text:
			_stream(text, self.error)
		return len(text)
	def flush(self):
		return
	def isatty(self):
		return False

_sys.stdout = _Writer(_sys.stdout)
_sys.stderr = _Writer(_sys.stderr, True)

def execute(command, params=None):
	return _json.loads(_execute(command, _json.dumps(params if params is not None else {}, allow_nan=False)))

async def execute_async(command, params=None):
	return execute(command, params)

def get_metadata():
	return _json.loads(_metadata(None))

def set_metadata(value):
	return _json.loads(_metadata(_json.dumps(value, allow_nan=False)))

def update_metadata(values):
	data = get_metadata()
	if not isinstance(data, dict) or not isinstance(values, dict):
		raise TypeError('Metadata update requires dictionaries')
	data.update(values)
	return set_metadata(data)

def run_steps(name, steps, reset=False):
	steps = list(steps)
	labels = [label for label, callback in steps]
	if len(set(labels)) != len(labels):
		raise ValueError('Step names must be unique')
	data = get_metadata()
	checkpoints = data.setdefault('_scriptCheckpoints', {})
	state = checkpoints.get(name, {}) if not reset else {}
	if state and state.get('steps') != labels:
		raise ValueError('Step list changed; use reset=True')
	state = {'steps': labels, 'completed': state.get('completed', 0), 'status': 'running'}
	checkpoints[name] = state
	set_metadata(data)
	for index in range(state['completed'], len(steps)):
		try:
			steps[index][1]()
		except Exception as error:
			data = get_metadata()
			state.update(status='failed', error=str(error))
			data.setdefault('_scriptCheckpoints', {})[name] = state
			set_metadata(data)
			raise
		state.update(completed=index+1, status='running')
		data = get_metadata()
		data.setdefault('_scriptCheckpoints', {})[name] = state
		set_metadata(data)
	state['status'] = 'completed'
	data = get_metadata()
	data.setdefault('_scriptCheckpoints', {})[name] = state
	set_metadata(data)
	return state

def _path(path):
	parts = path.split('/') if isinstance(path, str) else list(path)
	if not parts or any(not isinstance(p, str) or not p.strip() for p in parts):
		raise ValueError('Menu path must contain nonempty strings')
	return parts

class Menu:
	def __init__(self, path):
		self.path = _path(path)
		return
	def add_item(self, label, callback, *, icon='', bold=False, italic=False, color=None, enabled=True, monochrome=False, icon_color=None):
		if not callable(callback):
			raise TypeError('Menu handler must be callable')
		options = {'icon':icon,'bold':bool(bold),'italic':bool(italic),'color':color,'enabled':bool(enabled),'monochrome':bool(monochrome),'icon_color':_icon_rgba(icon_color)}
		_add_item(self.path, str(label), callback, _json.dumps(options))
		return self
	def add_menu(self, label, *, icon='', monochrome=False, icon_color=None):
		return add_menu(self.path + [label], icon=icon, monochrome=monochrome, icon_color=icon_color)
	def set_icon(self, icon='', *, monochrome=False, icon_color=None):
		_menu_icon(self.path, str(icon), bool(monochrome), _icon_rgba(icon_color))
		return self
	def add_qmenu(self, label, menu, *, icon='', monochrome=False, icon_color=None):
		from PySide6.QtGui import QCursor
		qt_application()
		return self.add_item(label, lambda: menu.popup(QCursor.pos()), icon=icon, monochrome=monochrome, icon_color=icon_color)

def _icon_rgba(value):
	if value is None:
		return None
	value = tuple(value)
	if len(value) not in (3, 4) or any(not isinstance(v, int) or isinstance(v, bool) or not 0 <= v <= 255 for v in value):
		raise ValueError('icon_color must be an RGB/RGBA tuple of integers from 0 to 255')
	return value + (255,) if len(value) == 3 else value

def add_menu(path, before='Help', *, icon='', monochrome=False, icon_color=None):
	parts = _path(path)
	_register_menu(parts, str(before))
	if icon or icon_color is not None:
		_menu_icon(parts, str(icon), bool(monochrome), _icon_rgba(icon_color))
	return Menu(parts)

def asset_icon_path(name):
	return _asset_path(str(name))

def add_qmenu(menu, path=None, before='Help', *, icon='', monochrome=False, icon_color=None):
	from PySide6.QtGui import QCursor
	qt_application()
	result = add_menu(path or menu.title(), before=before, icon=icon, monochrome=monochrome, icon_color=icon_color)
	result.add_item(menu.title(), lambda: menu.popup(QCursor.pos()), icon=icon, monochrome=monochrome, icon_color=icon_color)
	return result

def get_menus():
	return [Menu(path) for path in _menu_paths()]

def find_menu(path):
	parts = _path(path)
	return Menu(parts) if parts in _menu_paths() else None

def log(message):
	_log(str(message))
	return

def show_log():
	_show_log()
	return

def _loop():
	global _loop_instance
	if _loop_instance is None:
		_loop_instance = _asyncio.new_event_loop()
	return _loop_instance

def run(coroutine):
	_enable_pump()
	task = _loop().create_task(coroutine)
	_tasks.add(task)
	task.add_done_callback(_task_done)
	return task

def _task_done(task):
	_tasks.discard(task)
	if not task.cancelled() and task.exception() is not None:
		log(''.join(_traceback.format_exception(task.exception())))
	return

def qt_application():
	global _qt_app
	_enable_pump()
	import PySide6
	from PySide6.QtCore import QCoreApplication
	from PySide6.QtWidgets import QApplication
	plugins = _os.path.join(_os.path.dirname(PySide6.__file__), 'plugins')
	if _os.path.isdir(plugins):
		QCoreApplication.addLibraryPath(plugins)
	_qt_app = QApplication.instance() or QApplication(['AniCraft'])
	_qt_app.setQuitOnLastWindowClosed(False)
	return _qt_app

def qt_callback(callback):
	"""Queue a Qt signal handler for a safe AniCraft host tick."""
	if not callable(callback):
		raise TypeError('qt_callback requires a callable')
	_enable_pump()
	def queued(*args, **kwargs):
		if len(_callbacks) >= 512:
			raise RuntimeError('Qt callback queue is full')
		_callbacks.append((callback, args, kwargs))
		return
	return queued

def _main_window_id():
	"""Find this process' visible AniCraft winit top-level window on Windows."""
	if _sys.platform != 'win32':
		raise NotImplementedError('Native window ownership is currently Windows-only')
	import ctypes
	from ctypes import wintypes
	import os
	u32 = ctypes.WinDLL('user32', use_last_error=True)
	u32.GetWindowTextW.argtypes = (wintypes.HWND, wintypes.LPWSTR, ctypes.c_int)
	u32.GetWindowTextLengthW.argtypes = (wintypes.HWND,)
	u32.GetWindowTextLengthW.restype = ctypes.c_int
	u32.GetWindowThreadProcessId.argtypes = (wintypes.HWND, ctypes.POINTER(wintypes.DWORD))
	u32.GetWindowThreadProcessId.restype = wintypes.DWORD
	u32.IsWindowVisible.argtypes = (wintypes.HWND,)
	u32.IsWindowVisible.restype = wintypes.BOOL
	u32.GetWindow.argtypes = (wintypes.HWND, ctypes.c_uint)
	u32.GetWindow.restype = wintypes.HWND
	results = []
	visitor_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
	@visitor_type
	def visitor(hwnd, lparam):
		pid = wintypes.DWORD()
		if u32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid)) and pid.value == os.getpid() and u32.IsWindowVisible(hwnd) and not u32.GetWindow(hwnd, 4):
			n = u32.GetWindowTextLengthW(hwnd)
			if 0 < n < 1024:
				buf = ctypes.create_unicode_buffer(n + 1)
				u32.GetWindowTextW(hwnd, buf, n + 1)
				if 'anicraft' in buf.value.lower():
					class_name = ctypes.create_unicode_buffer(256)
					u32.GetClassNameW(hwnd, class_name, 256)
					if not class_name.value.lower().startswith('qt'):
						results.append((int(hwnd), buf.value))
		return True
	u32.EnumWindows.argtypes = (visitor_type, wintypes.LPARAM)
	u32.EnumWindows.restype = wintypes.BOOL
	u32.GetClassNameW.argtypes = (wintypes.HWND, wintypes.LPWSTR, ctypes.c_int)
	u32.EnumWindows(visitor, 0)
	if not results:
		raise RuntimeError('AniCraft main window not found')
	return next((hwnd for hwnd, title in results if title == 'AniCraft'), results[0][0])

def _native_window_owner(winid, owner):
	import ctypes
	from ctypes import wintypes
	u32 = ctypes.WinDLL('user32', use_last_error=True)
	setter = u32.SetWindowLongPtrW if ctypes.sizeof(ctypes.c_void_p) == 8 else u32.SetWindowLongW
	setter.argtypes = (wintypes.HWND, ctypes.c_int, ctypes.c_ssize_t)
	setter.restype = ctypes.c_ssize_t
	ctypes.set_last_error(0)
	previous = setter(winid, -8, owner)
	error = ctypes.get_last_error()
	if previous == 0 and error:
		raise ctypes.WinError(error)
	return previous

def _track_external_window(widget, owner):
	"""Detach owned Qt windows on Close/Hide, preserving native close semantics."""
	global _external_filter
	from PySide6.QtCore import QObject, QEvent
	from PySide6.QtWidgets import QApplication
	if _external_filter is None:
		class _OwnerFilter(QObject):
			def eventFilter(self, watched, event):
				if event.type() in (QEvent.Type.Close, QEvent.Type.Hide):
					if id(watched) in _external_windows:
						try:
							_native_window_owner(int(watched.winId()), 0)
						except OSError as error:
							log(f'Cannot detach Qt tool window: {error}')
				return False
				return
		_external_filter = _OwnerFilter(QApplication.instance())
	key = id(widget)
	if key not in _external_windows:
		widget.installEventFilter(_external_filter)
		widget.destroyed.connect(lambda *_: _external_windows.pop(key, None))
	_external_windows[key] = (widget, owner)
	return

def _sync_external_windows():
	if not _external_windows:
		return
	from shiboken6 import isValid
	for key, (window, owner) in tuple(_external_windows.items()):
		if not isValid(window):
			_external_windows.pop(key, None)
		elif window.isVisible():
			_native_window_owner(int(window.winId()), owner)
	return

def parent_external_window_to_main(winid):
	"""Set AniCraft as the Win32 owner of an external PySide6 top-level window."""
	import ctypes
	from ctypes import wintypes
	import os
	winid = int(winid)
	owner = _main_window_id()
	if not winid or winid == owner:
		raise ValueError('Expected a separate top-level window')
	u32 = ctypes.WinDLL('user32', use_last_error=True)
	u32.GetWindowThreadProcessId.argtypes = (wintypes.HWND, ctypes.POINTER(wintypes.DWORD))
	u32.GetWindowThreadProcessId.restype = wintypes.DWORD
	p = wintypes.DWORD()
	if not u32.GetWindowThreadProcessId(winid, ctypes.byref(p)) or p.value != os.getpid():
		raise ValueError('Window does not belong to AniCraft process')
	getter = u32.GetWindowLongPtrW if ctypes.sizeof(ctypes.c_void_p) == 8 else u32.GetWindowLongW
	getter.argtypes = (wintypes.HWND, ctypes.c_int)
	getter.restype = ctypes.c_ssize_t
	if getter(winid, -16) & 0x40000000:
		raise ValueError('Expected a top-level window, not a child widget')
	_native_window_owner(winid, owner)
	if 'PySide6.QtWidgets' in _sys.modules:
		from PySide6.QtWidgets import QApplication
		widget = next((w for w in QApplication.topLevelWidgets() if int(w.winId()) == winid), None)
		if widget is not None:
			_track_external_window(widget, owner)
			_enable_pump()
	return

def _pump():
	global _qt_processing, _qt_last_native_pump, _pump_active
	if _pump_active:
		return
	_pump_active = True
	try:
		queued, _callbacks[:] = _callbacks[:], []
		for callback, args, kwargs in queued:
			try:
				value = callback(*args, **kwargs)
				if _asyncio.iscoroutine(value):
					run(value)
			except Exception:
				log(_traceback.format_exc())
		if _loop_instance is not None and not _loop_instance.is_running():
			_loop_instance.call_soon(_loop_instance.stop)
			_loop_instance.run_forever()
		if _qt_app is not None:
			from PySide6.QtCore import QCoreApplication, QEvent, QEventLoop
			if _sys.platform == 'win32':
				# winit dispatches Win32 messages. A bounded Qt pass is needed for some
				# native Qt close/activation events; never reenter it recursively.
				# Native Qt events are enabled by default for registered external
				# windows only. Set ANICRAFT_QT_NATIVE_EVENTS=0 to disable.
				enabled = _os.environ.get('ANICRAFT_QT_NATIVE_EVENTS', '1').lower() not in ('0', 'false', 'off', 'no')
				now = _time.monotonic()
				if enabled and _external_windows and not _qt_processing and now - _qt_last_native_pump >= 0.05:
					_qt_processing = True
					_qt_last_native_pump = now
					try:
						_qt_app.processEvents(QEventLoop.ProcessEventsFlag.AllEvents, 1)
					finally:
						_qt_processing = False
				QCoreApplication.sendPostedEvents(None, 0)
			else:
				_qt_app.processEvents(QEventLoop.ProcessEventsFlag.AllEvents, 2)
			QCoreApplication.sendPostedEvents(None, QEvent.Type.DeferredDelete)
			if _sys.platform == 'win32' and _external_windows:
				_sync_external_windows()
	finally:
		_pump_active = False
	return

def _console_push(line):
	"""Submit one physical line and return whether another line is required."""
	return bool(_interactive_console.push(line))

def _console_reset():
	"""Cancel an unfinished compound statement without destroying session globals."""
	_interactive_console.resetbuffer()
	return

for key in ('execute', 'execute_async', 'get_metadata', 'set_metadata', 'update_metadata', 'run_steps', 'Menu', 'add_menu', 'get_menus', 'find_menu', 'log', 'show_log', 'run', 'qt_application', 'qt_callback', 'parent_external_window_to_main', 'asset_icon_path', 'add_qmenu'):
	setattr(m, key, globals()[key])
m._pump = _pump
m._console_push = _console_push
m._console_reset = _console_reset
m.version = '0.2'
_sys.modules['anicraft'] = m
_console_globals['anicraft'] = m
"#;

fn with_session<T>(app: &mut App, f: impl FnOnce() -> T) -> T {
    struct Restore<'a>(&'a mut anicraft_engine::Session);
    impl Drop for Restore<'_> {
        fn drop(&mut self) {
            HOST.with(|h| {
                if let Some(s) = h.borrow_mut().session.take() { *self.0 = s; }
            });
        }
    }
    HOST.with(|h| h.borrow_mut().session = Some(std::mem::take(&mut app.s)));
    let _restore = Restore(&mut app.s);
    f()
}

fn scripts_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("ANICRAFT_SCRIPTS_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(path);
    }
    let mut candidates = Vec::new();
    if let Ok(cwd) = std::env::current_dir() { candidates.push(cwd.join("scripts")); }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("scripts"));
            if let Some(root) = parent.parent().and_then(|p| p.parent()) {
                candidates.push(root.join("scripts"));
            }
        }
    }
    candidates.iter().find(|p| p.join("userSetup.py").is_file()).cloned()
        .or_else(|| candidates.iter().find(|p| p.is_dir()).cloned())
        .unwrap_or_else(|| PathBuf::from("scripts"))
}

fn python_error(err: PyErr, py: Python<'_>) {
    let message = err.to_string();
    HOST.with(|h| h.borrow_mut().errors.push(message));
    err.print(py);
}

fn start(app: &mut App) {
    if HOST.with(|h| h.borrow().started) { return; }
    trace("python.start.begin");
    HOST.with(|h| h.borrow_mut().started = true);
    with_session(app, || {
        Python::attach(|py| {
            let result: PyResult<()> = (|| {
                let native = PyModule::new(py, "_anicraft")?;
                native.add_function(wrap_pyfunction!(_execute, &native)?)?;
                native.add_function(wrap_pyfunction!(_metadata, &native)?)?;
                native.add_function(wrap_pyfunction!(_register_menu, &native)?)?;
                native.add_function(wrap_pyfunction!(_menu_paths, &native)?)?;
                native.add_function(wrap_pyfunction!(_menu_icon, &native)?)?;
                native.add_function(wrap_pyfunction!(_asset_path, &native)?)?;
                native.add_function(wrap_pyfunction!(_add_item, &native)?)?;
                native.add_function(wrap_pyfunction!(_log, &native)?)?;
                native.add_function(wrap_pyfunction!(_stream, &native)?)?;
                native.add_function(wrap_pyfunction!(_show_log, &native)?)?;
                native.add_function(wrap_pyfunction!(_enable_pump, &native)?)?;
                let sys = py.import("sys")?;
                sys.getattr("modules")?.set_item("_anicraft", native)?;
                let code = CString::new(BOOTSTRAP).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
                py.run(code.as_c_str(), None, None)?;
                let directory = scripts_dir();
                sys.getattr("path")?.call_method1("insert", (0, directory.display().to_string()))?;
                let file = directory.join("userSetup.py");
                HOST.with(|h| stream(&mut h.borrow_mut(), &format!("AniCraft Python 3.14 · scripts: {}\n", directory.display()), false));
                if file.is_file() {
                    let source = std::fs::read_to_string(&file).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
                    let code = CString::new(source).map_err(|e| PyRuntimeError::new_err(e.to_string()))?;
                    let globals = PyDict::new(py);
                    globals.set_item("__name__", "__main__")?;
                    globals.set_item("__file__", file.display().to_string())?;
                    py.run(code.as_c_str(), Some(&globals), None)?;
                    HOST.with(|h| stream(&mut h.borrow_mut(), "userSetup.py loaded\n", false));
                } else {
                    HOST.with(|h| stream(&mut h.borrow_mut(), "userSetup.py not found\n", true));
                }
                Ok(())
            })();
            if let Err(err) = result { python_error(err, py); }
        });
    });
    trace("python.start.end");
}

pub(crate) fn tick(app: &mut App) -> bool {
    trace("python.tick.begin");
    start(app);
    trace("python.tick.after_start");
    let show = HOST.with(|h| std::mem::take(&mut h.borrow_mut().open_console));
    if show {
        app.dock.show(crate::dock::Panel::PythonConsole);
        app.dock_changed();
    }
    if !HOST.with(|h| h.borrow().pump) { trace("python.tick.no_pump"); return show; }
    crate::freeze_diag::mark(crate::freeze_diag::Phase::PythonPump);
    with_session(app, || Python::attach(|py| {
        let result: PyResult<()> = (|| {
            let sys = py.import("sys")?;
            let api = sys.getattr("modules")?.get_item("anicraft")?;
            api.getattr("_pump")?.call0()?;
            Ok(())
        })();
        if let Err(err) = result { python_error(err, py); }
    }));
    trace("python.tick.after_pump");
    true
}

pub(crate) fn info() -> Value {
    HOST.with(|h| {
        let h = h.borrow();
        json!({"python":"3.14","started":h.started,"script":scripts_dir().join("userSetup.py").display().to_string(),"errors":h.errors,"log":h.log})
    })
}

fn child_menus(path: &[String], registered: &[Vec<String>]) -> Vec<String> {
    let mut children = Vec::new();
    for entry in registered {
        if entry.len() > path.len() && entry.starts_with(path) && !children.contains(&entry[path.len()]) {
            children.push(entry[path.len()].clone());
        }
    }
    children
}

/// (title, insert-before title), in registration order.
pub(crate) fn roots() -> Vec<(String, String)> {
    HOST.with(|h| h.borrow().roots.clone())
}

fn call_item(app: &mut App, index: usize) {
    with_session(app, || Python::attach(|py| {
        let callback = HOST.with(|h| h.borrow().items.get(index).map(|item| item.callback.clone_ref(py)));
        if let Some(callback) = callback {
            if let Err(err) = callback.call0(py) { python_error(err, py); }
        }
    }));
}

fn draw_items(app: &mut App, ui: &mut egui::Ui, path: &[String]) {
    let (children, entries, decorations) = HOST.with(|h| {
        let h = h.borrow();
        let children = child_menus(path, &h.paths);
        let entries = h.items.iter().enumerate().filter(|(_, i)| i.path == path)
            .map(|(ix, i)| (ix, i.label.clone(), i.options.clone())).collect::<Vec<_>>();
        (children, entries, h.menu_options.clone())
    });
    for child in children {
        let mut nested = path.to_vec();
        nested.push(child.clone());
        let opts = decorations.get(&nested).cloned().unwrap_or_default();
        let decorated = if opts.icon.chars().any(|c| !c.is_ascii()) && opts.icon.chars().count() <= 4 && find_icon(&opts.icon).is_none() {
            format!("{}  {child}", opts.icon)
        } else { format!("      {child}") };
        let response = ui.menu_button(decorated, |ui| draw_items(app, ui, &nested));
        if !opts.icon.is_empty() { paint_menu_icon(ui, response.response.rect, &opts.icon, opts.monochrome, opts.icon_color, false); }
    }
    for (index, label, options) in entries {
        let shown = if options.icon.chars().any(|c| !c.is_ascii()) && options.icon.chars().count() <= 4 && find_icon(&options.icon).is_none() {
            format!("{}  {label}", options.icon)
        } else { format!("      {label}") };
        let mut text = egui::RichText::new(shown);
        if options.bold { text = text.strong(); }
        if options.italic { text = text.italics(); }
        if let Some([r, g, b]) = options.color { text = text.color(egui::Color32::from_rgb(r, g, b)); }
        let response = ui.add_enabled(options.enabled.unwrap_or(true), egui::Button::new(text));
        if !options.icon.is_empty() { paint_menu_icon(ui, response.rect, &options.icon, options.monochrome, options.icon_color, false); }
        if response.clicked() {
            call_item(app, index);
            ui.close();
        }
    }
}

pub(crate) fn menu_contents(app: &mut App, ui: &mut egui::Ui, title: &str) {
    let root = vec![title.to_string()];
    draw_items(app, ui, &root);
}

fn ansi_color(index: u32, default: egui::Color32) -> egui::Color32 {
    let basic = [
        egui::Color32::from_rgb(25, 25, 25), egui::Color32::from_rgb(230, 70, 80),
        egui::Color32::from_rgb(90, 205, 110), egui::Color32::from_rgb(230, 190, 80),
        egui::Color32::from_rgb(95, 150, 250), egui::Color32::from_rgb(205, 100, 215),
        egui::Color32::from_rgb(60, 200, 205), egui::Color32::from_rgb(230, 230, 230),
    ];
    match index {
        30..=37 => basic[(index - 30) as usize],
        90..=97 => basic[(index - 90) as usize].gamma_multiply(1.3),
        _ => default,
    }
}

/// A colored-glyph fallback for common status emoji. Full multi-color emoji
/// depends on the platform font renderer; epaint itself paints text as glyphs.
fn append_console_text(job: &mut egui::text::LayoutJob, value: &str, color: egui::Color32) {
    let mut at = 0;
    for (offset, ch) in value.char_indices() {
        let tint = match ch {
            '✅' | '🐍' | '✔' | '🌿' => Some(egui::Color32::from_rgb(85, 218, 120)),
            '❌' | '🛑' | '❗' => Some(egui::Color32::from_rgb(244, 90, 90)),
            '⚠' | '💡' | '⭐' => Some(egui::Color32::from_rgb(245, 193, 70)),
            '🔥' | '🎉' | '🚀' => Some(egui::Color32::from_rgb(250, 133, 86)),
            _ => None,
        };
        if let Some(tint) = tint {
            if at < offset {
                job.append(&value[at..offset], 0.0, egui::TextFormat { font_id: egui::FontId::monospace(13.0), color, ..Default::default() });
            }
            job.append(&value[offset..offset + ch.len_utf8()], 0.0, egui::TextFormat { font_id: egui::FontId::proportional(14.0), color: tint, ..Default::default() });
            at = offset + ch.len_utf8();
        }
    }
    if at < value.len() {
        job.append(&value[at..], 0.0, egui::TextFormat { font_id: egui::FontId::monospace(13.0), color, ..Default::default() });
    }
}

/// ANSI SGR (standard colors, 256-color and true-color); Unicode stays UTF-8.
fn colored_job(value: &str, error: bool) -> egui::text::LayoutJob {
    let default = if error { egui::Color32::from_rgb(255, 145, 145) } else { egui::Color32::from_rgb(223, 229, 238) };
    let mut color = default;
    let mut bold = false;
    let mut job = egui::text::LayoutJob::default();
    let mut offset = 0;
    while let Some(pos) = value[offset..].find("\u{1b}[") {
        let begin = offset + pos;
        append_console_text(&mut job, &value[offset..begin], color);
        let rest = &value[begin+2..];
        let Some(end) = rest.find('m') else { offset = begin; break; };
        if end > 64 { offset = begin; break; }
        let params: Vec<u32> = rest[..end].split(';').map(|v| v.parse().unwrap_or(0)).collect();
        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0 => { color = default; bold = false; }
                1 => bold = true,
                22 => bold = false,
                39 => color = default,
                code @ (30..=37 | 90..=97) => color = ansi_color(code, default),
                38 if i+2 < params.len() && params[i+1] == 5 => {
                    let n = params[i+2];
                    let base = if n < 16 { ansi_color((n % 8) + if n >= 8 { 90 } else { 30 }, default) }
                        else if n < 232 { let v = n-16; let c = |x: u32| if x == 0 { 0 } else { (55+x*40).min(255) as u8 }; egui::Color32::from_rgb(c(v/36), c((v/6)%6), c(v%6)) }
                        else { let gray = (8+(n-232)*10).min(255) as u8; egui::Color32::from_gray(gray) };
                    color = base;
                    i += 2;
                }
                38 if i+4 < params.len() && params[i+1] == 2 => {
                    color = egui::Color32::from_rgb(params[i+2].min(255) as u8, params[i+3].min(255) as u8, params[i+4].min(255) as u8);
                    i += 4;
                }
                _ => {}
            }
            i += 1;
        }
        offset = begin + 2 + end + 1;
    }
    let left = &value[offset..];
    let _ = bold;
    append_console_text(&mut job, left, color);
    job
}

fn console_command(app: &mut App, source: &str) {
    // One command may contain pasted lines. Submit each to Python's InteractiveConsole
    // so syntax, blocks and expression display behave like the native Python prompt.
    for line in source.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        HOST.with(|h| {
            let mut h = h.borrow_mut();
            let previous = h.continuation;
            let prefix = if previous { "... " } else { ">>> " };
            if h.output.len() >= 2000 { h.output.remove(0); }
            h.output.push(ConsoleLine { text: format!("{prefix}{line}"), error: false, prompt: true });
            if !line.trim().is_empty() && h.history.last().is_none_or(|last| last != line) {
                h.history.push(line.to_owned());
                if h.history.len() > 500 { h.history.remove(0); }
            }
            h.history_index = None;
            h.history_draft.clear();
        });
        let more = with_session(app, || Python::attach(|py| {
            let result: PyResult<bool> = (|| {
                py.import("anicraft")?.getattr("_console_push")?.call1((if line.trim().is_empty() { "" } else { line },))?.extract()
            })();
            match result {
                Ok(more) => more,
                Err(err) => { python_error(err, py); false }
            }
        }));
        HOST.with(|h| h.borrow_mut().continuation = more);
    }
}

fn console_cancel(app: &mut App) {
    with_session(app, || Python::attach(|py| {
        if let Err(err) = py.import("anicraft").and_then(|m| m.getattr("_console_reset")?.call0().map(|_| ())) {
            python_error(err, py);
        }
    }));
    HOST.with(|h| {
        let mut h = h.borrow_mut();
        push_line(&mut h, "KeyboardInterrupt".into(), true);
        h.continuation = false;
        h.input.clear();
        h.history_index = None;
    });
}

fn history_previous(h: &mut Host, current: &str) -> String {
    if h.history.is_empty() { return current.to_owned(); }
    let index = match h.history_index {
        Some(index) => index.saturating_sub(1),
        None => { h.history_draft = current.to_owned(); h.history.len() - 1 }
    };
    h.history_index = Some(index);
    h.history[index].clone()
}

fn history_next(h: &mut Host, current: &str) -> String {
    let Some(index) = h.history_index else { return current.to_owned(); };
    if index + 1 >= h.history.len() {
        h.history_index = None;
        std::mem::take(&mut h.history_draft)
    } else {
        h.history_index = Some(index + 1);
        h.history[index + 1].clone()
    }
}

fn console_line_job(row: &ConsoleLine) -> egui::text::LayoutJob {
    if !row.prompt { return colored_job(&row.text, row.error); }
    let mut job = egui::text::LayoutJob::default();
    let (prefix, code) = row.text.split_at(4); // >>> / ... plus the separating space
    job.append(prefix, 0.0, egui::TextFormat {
        font_id: egui::FontId::monospace(13.0),
        color: egui::Color32::from_rgb(118, 170, 255),
        ..Default::default()
    });
    append_console_text(&mut job, code, egui::Color32::from_rgb(223, 229, 238));
    job
}

fn next_indentation(line: &str, more: bool) -> String {
    if !more { return String::new(); }
    let tail = line.lines().last().unwrap_or(line);
    let count = tail.chars().take_while(|ch| *ch == ' ' || *ch == '\t').count().min(64);
    let mut indent = tail.chars().take(count).collect::<String>();
    if tail.trim_end().ends_with(':') { indent.push('\t'); }
    indent
}

/// Native dock panel: a continuous InteractiveConsole transcript with an editable prompt.
/// Enter submits, Shift+Enter inserts an explicit line break, Up/Down browse history.
pub(crate) fn console(app: &mut App, ui: &mut egui::Ui) {
    trace("console.enter");
    // Test whether the freeze is in the panel renderer while keeping Python enabled.
    if std::env::var("ANICRAFT_REPL_SAFE_MODE").is_ok_and(|value| value == "1") {
        trace("console.safe_mode: UI skipped");
        ui.label("Python REPL UI temporarily disabled for diagnostics; Python host remains active");
        return;
    }
    let (mut input, mut rows, continuation, initial_focus) = HOST.with(|cell| {
        let mut h = cell.borrow_mut();
        let initial_focus = !h.shown_once;
        h.shown_once = true;
        let mut rows = h.output.clone();
        if !h.output_pending.is_empty() {
            rows.push(ConsoleLine { text: h.output_pending.clone(), error: h.output_pending_error, prompt: false });
        }
        (h.input.clone(), rows, h.continuation, initial_focus)
    });
    trace("console.snapshot.done");

    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Python 3.14  •  Interactive Console").small().weak());
        if ui.small_button("Clear output").on_hover_text("Clear transcript; keep variables and command history (Ctrl+L)").clicked() {
            HOST.with(|cell| {
                let mut h = cell.borrow_mut();
                h.output.clear();
                h.output_pending.clear();
            });
            rows.clear();
        }
        if ui.small_button("Copy transcript").clicked() {
            ui.ctx().copy_text(rows.iter().map(|row| row.text.as_str()).collect::<Vec<_>>().join("\n"));
        }
        ui.label(egui::RichText::new("Enter: run  •  Shift+Enter: newline  •  ↑/↓: history  •  Esc: cancel").small().weak());
    });
    ui.separator();
    trace("console.toolbar.done");

    egui::ScrollArea::vertical().id_salt("anicraft_python_repl_scroll")
        .auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        trace("console.scroll.enter");
        ui.set_min_width(ui.available_width());
        for row in &rows {
            if row.text.is_empty() {
                ui.add_space(14.0);
            } else {
                ui.add(egui::Label::new(console_line_job(row)).selectable(true));
            }
        }
        trace("console.rows.done");
        let prompt = if continuation { "... " } else { ">>> " };
        trace("console.text_edit.before");
        let response = ui.horizontal(|ui| {
            ui.label(egui::RichText::new(prompt).monospace().color(egui::Color32::from_rgb(118, 170, 255)));
            // ScrollArea must not receive an infinite-sized child: measure a finite width.
            let clip_width = ui.clip_rect().width();
            let edit_width = if clip_width.is_finite() { (clip_width - 60.0).clamp(120.0, 2048.0) } else { 600.0 };
            ui.add(egui::TextEdit::multiline(&mut input)
                .id_salt("anicraft_python_repl_input")
                .font(egui::TextStyle::Monospace)
                .desired_rows(1)
                .desired_width(edit_width)
                .frame(egui::Frame::new())
                .return_key(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter))
                .lock_focus(true)
                .hint_text(if continuation { "..." } else { "Python expression or statement" }))
        }).inner;
        trace("console.text_edit.after");

        // Do not steal focus when a document opens. The user can focus the console by clicking it.
        if initial_focus { trace("console.first_show.no_autofocus"); }
        // Read Response focus OUTSIDE ui.input(): nested egui Context reads may deadlock.
        trace("console.focus.before");
        let has_focus = response.has_focus();
        let lost_focus = response.lost_focus();
        trace("console.focus.after");
        trace("console.input.before");
        let (enter, up, down, escape, clear) = ui.input(|i| {
            let focused = has_focus || lost_focus;
            (
                focused && i.key_pressed(egui::Key::Enter) && !i.modifiers.shift,
                has_focus && i.key_pressed(egui::Key::ArrowUp) && !input.contains('\n'),
                has_focus && i.key_pressed(egui::Key::ArrowDown) && !input.contains('\n'),
                focused && i.key_pressed(egui::Key::Escape),
                has_focus && i.modifiers.command && i.key_pressed(egui::Key::L),
            )
        });
        trace("console.input.after");
        if clear {
            HOST.with(|cell| {
                let mut h = cell.borrow_mut();
                h.output.clear();
                h.output_pending.clear();
            });
        }
        if escape && (continuation || !input.is_empty()) {
            console_cancel(app);
            input.clear();
            ui.memory_mut(|memory| memory.request_focus(response.id));
        } else if up || down {
            input = HOST.with(|cell| {
                let mut h = cell.borrow_mut();
                if up { history_previous(&mut h, &input) } else { history_next(&mut h, &input) }
            });
        } else if enter {
            // TextEdit uses Shift+Enter for a literal newline; Enter submits.
            let source = input.clone();
            input.clear();
            console_command(app, &source);
            let more = HOST.with(|h| h.borrow().continuation);
            input = next_indentation(&source, more);
            ui.memory_mut(|memory| memory.request_focus(response.id));
        } else if response.changed() {
            HOST.with(|cell| {
                let mut h = cell.borrow_mut();
                h.history_index = None;
                h.history_draft.clear();
            });
        }
        trace("console.store.before");
        HOST.with(|cell| cell.borrow_mut().input = input);
        trace("console.store.after");
        trace("console.scroll.end");
    });
    trace("console.exit");
}
