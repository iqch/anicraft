# Embedded Python 3.14, REPL, menus and PySide6

> **Last updated:** 2026-11-16. **Status:** Available in desktop builds compiled with the `python` feature.  
> **Runtime distinction:** CPython is **3.14**; the `anicraft` Python facade currently advertises API version `0.2`.

## Build and deployment

The Windows build entry point in our fork is:

```bat
scripts\build\build.cmd
```

The build uses `cargo build --release -p anicraft --features python --jobs 8`, with `ANICRAFT_PYTHON_HOME` pointing to the Python 3.14 installation (the development default was `C:\Python\3.14.6`). It arranges `python314.dll`, optionally `python3.dll`, a `python314._pth` import configuration and the `scripts` Python files beside the Release executable.

This build uses the existing Python standard library and installed packages from the configured installation; packaging a Python runtime file next to the executable does **not** mean the whole standard library and PySide6 are bundled into a portable standalone deployment.

For a custom Python installation set `ANICRAFT_PYTHON_HOME` before running the build. If using PySide6, install it in the **same Python 3.14** environment. Non-Python builds and the browser/WebAssembly app do not provide this embedded host.

## Startup and scripts

At application startup the host resolves a scripts directory. `ANICRAFT_SCRIPTS_DIR` takes priority; otherwise it looks for `scripts` in customary working-directory/executable-relative locations. It adds that directory to `sys.path`, initializes the `anicraft` facade and loads `userSetup.py` once if present.

Startup scripts execute as trusted local Python, in process, with the user's permissions. They are not sandboxed. Do not execute arbitrary untrusted document content as Python. The startup script must not call `QApplication.exec()` or create a separate permanent GUI event loop.

## Main API

The supported facade, visible as `import anicraft`, includes:

| API | Description |
|---|---|
| `execute(command, params=None)` | Invoke an engine command by registered command ID; JSON-compatible result. |
| `execute_async(command, params=None)` | Async facade; currently delegates to the same underlying execution. |
| `get_metadata()` / `set_metadata(value)` / `update_metadata(values)` | Document user metadata; respect internal reserved keys. |
| `run_steps(name, steps, reset=False)` | Checkpointed sequence of named callbacks, stored under `_scriptCheckpoints`. |
| `add_menu(path, before='Help', ...)` / `find_menu(path)` / `get_menus()` | Register/find menus. |
| `Menu.add_item(...)`, `Menu.add_menu(...)`, `Menu.add_qmenu(...)` | Python menu callbacks, submenus and PySide6 QMenus. |
| `log(text)` / `show_log()` | Send text to and open the Python console/log. |
| `run(coroutine)` | Schedule work on the host's cooperative asyncio pump. |
| `qt_application()` / `qt_callback(callback)` | Initialize Qt and queue callbacks to a safe host UI tick. |
| `parent_external_window_to_main(winid)` | Attach a separate top-level PySide6 window as a native **Win32-owned** window. |
| `asset_icon_path(name)` | Resolve a named icon from configured asset roots. |

The host deliberately **does not** provide direct mutable document-object handles, generic raster exchange, object-change subscriptions, or a remote execute-arbitrary-Python endpoint. Use engine commands and structured metadata instead.

### Example `scripts/userSetup.py`

```python
import anicraft

def show_document_info():
	anicraft.log(str(anicraft.get_metadata()))
	return

menu = anicraft.add_menu('Pipeline', before='Help', icon='vision')
menu.add_item('Show metadata', show_document_info, icon='vision')
```

Python indentations in this example are literal **tabs**. The `vision` icon is optional; if it cannot be found, the item still appears.

### Example of an engine command from the console

```python
import anicraft
anicraft.execute('scene.set_duration', {'frames': 120})
anicraft.execute('layer.set_properties', {'pre_visibility': 'off', 'post_visibility': 'continue'})
```

Use command IDs and params from the **current** AniCraft command registry. Engine calls require an active application session/callback; this is not an externally callable network interface.

## Python Console (REPL)

The console is a dockable panel, initially located with Timeline in the development layout. `anicraft` is pre-imported. It is an interactive Python REPL (compound input/continuation and history), not just a single-expression evaluator. `stdout` and `stderr` are redirected into a bounded UI stream; errors and prompts are distinguished, and Unicode/emoji output is supported. Re-opening the panel keeps the same interpreter state.

The host has protection against reentrant calls and bounds on line/log lengths. Calls to the Rust engine should not be recursively triggered from an unsafe native Qt event context; use `anicraft.qt_callback()` to defer signal handlers to the host update loop.

## Qt / PySide6 integration

- `qt_application()` returns or creates the one `QApplication`; **never call `QApplication.exec()`** in `userSetup.py`.
- `qt_callback(fn)` creates a wrapper appropriate for Qt signals. The wrapper queues work for a safe AniCraft host tick. This avoids reentrancy and keeps engine operations in the host context.
- `add_qmenu()` permits actual Qt menus initiated from AniCraft menu actions.
- `parent_external_window_to_main(widget.winId())` associates an external top-level Qt window with AniCraft on Windows. This is native window ownership, **not** embedding a QWidget inside egui.
- Native Windows event pumping can be disabled through `ANICRAFT_QT_NATIVE_EVENTS=0` when diagnosing difficult host/Qt event interactions. Do not enable nested event loops as a workaround.

A safe UI pattern for a callback:

```python
import anicraft
from PySide6.QtWidgets import QPushButton

anicraft.qt_application()
button = QPushButton('Log from Qt')

def when_clicked():
	anicraft.log('Qt button clicked')
	return

button.clicked.connect(anicraft.qt_callback(when_clicked))
button.show()
anicraft.parent_external_window_to_main(button.winId())
```

For a long-lived window retain a Python reference, and give empty layout-only windows a nonzero initial size. On non-Windows platforms the native ownership helper currently raises `NotImplementedError`.

## Icons and environment

`ANICRAFT_ASSETS` is a semicolon-separated list of local resource roots, searched in order. Each root may be an `icons` folder or contain an `icons/` child. `icon='vision'` checks for `vision.png` then `vision.svg`; names must not be arbitrary filesystem paths. The host also checks its standard local `assets` locations.

Menu entries may be styled using `icon`, `bold`, `italic`, `color`, `enabled`, `monochrome`, and `icon_color`. Named PNG/SVG icons, and short emoji/glyph labels, are supported where appropriate.

## Diagnostics and limits

- `ANICRAFT_DIAG_LOG` enables bounded, opt-in host start/new-document diagnostics. Do not leave temporary diagnostic instrumentation running without a need.
- If creating a new document hangs, test with Python/Qt startup work minimized before investigating scene rendering; older PySide6 startup calls caused freezes and were reworked.
- No claims of macOS/Linux native external Qt window ownership are made by this Windows implementation.
- User scripts are trusted executable code; review `userSetup.py` and imports before running them.

## Implementation anchors

`src/python.rs` contains the embedded facade, REPL, menus/icons, Qt integration, startup and event pump. `scripts/build/build.cmd` is the Windows Release entry point. Embedded Python is intentionally a small command/metadata interface rather than a full Rust object binding.
