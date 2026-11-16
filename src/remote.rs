//! Minimal, deliberately loopback-only JSON-line command endpoint.
//! Each line is {"id":1,"method":"engine.execute","params":{"command":"...","params":{}}}.
//! Responses are one JSON line. The UI thread alone touches the engine session.
use crate::app::App;
use serde_json::{Value, json};
use std::{io::{self, BufRead, BufReader, Write}, net::{TcpListener, TcpStream}, sync::{Arc, atomic::{AtomicBool, Ordering}, mpsc}, thread, time::Duration};

const MAX_REQUEST_BYTES: usize = 64 * 1024 * 1024;
const MAX_SVG_BYTES: usize = 48 * 1024 * 1024;

struct Request { value: Value, reply: mpsc::SyncSender<Value> }

pub(crate) struct Remote {
    port: Option<u16>,
    chosen: u16,
    stop: Option<Arc<AtomicBool>>,
    requests: Option<mpsc::Receiver<Request>>,
    error: String,
}
impl Default for Remote {
    fn default() -> Self { Self {port:None, chosen:7980, stop:None, requests:None, error:String::new()} }
}
impl Drop for Remote {
    fn drop(&mut self) { self.stop(); }
}
impl Remote {
    fn stop(&mut self) {
        if let Some(flag) = self.stop.take() { flag.store(true, Ordering::Relaxed); }
        self.port = None;
        self.requests = None;
    }
    /// Startup equivalent of VectorCraft's --control <port>. The listener remains
    /// loopback-only, and the API Control menu can still disable it later.
    pub(crate) fn activate(&mut self, port: u16, ctx: egui::Context) -> Result<(), String> {
        if port == 0 { return Err("port must be between 1 and 65535".into()); }
        self.chosen = port;
        self.start(ctx)
    }
    fn start(&mut self, ctx: egui::Context) -> Result<(), String> {
        if self.port.is_some() { return Err("Already listening".into()); }
        let listener = TcpListener::bind(("127.0.0.1", self.chosen)).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::sync_channel::<Request>(32);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        thread::Builder::new().name("anicraft-control".into()).spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream,_)) => {
                        // Windows can inherit a non-blocking accepted socket from the listener.
                        // The per-client BufReader below expects blocking reads with timeouts.
                        if let Err(error) = stream.set_nonblocking(false) {
                            trace(&format!("accepted socket could not enter blocking mode: {error}"));
                            continue;
                        }
                        let tx = tx.clone(); let ctx = ctx.clone();
                        let _ = thread::Builder::new().name("anicraft-client".into()).spawn(move || client(stream, tx, ctx));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(30)),
                    Err(_) => break,
                }
            }
        }).map_err(|e| e.to_string())?;
        self.requests = Some(rx); self.stop = Some(stop); self.port = Some(port);
        Ok(())
    }
    /// Returns true if a bridge import was executed this frame. The caller
    /// can defer Python/Qt event pumping until the next frame so an image update
    /// and a nested Qt message pump are not executed in one UI callback.
    pub(crate) fn poll(&mut self, app: &mut App) -> bool {
        let Some(rx) = &self.requests else { return false };
        let mut imported = false;
        for _ in 0..16 {
            let Ok(req) = rx.try_recv() else { break };
            let id = req.value.get("id").cloned().unwrap_or(Value::Null);
            let method = req.value.get("method").and_then(Value::as_str).unwrap_or("");
            let params = req.value.get("params").cloned().unwrap_or_else(|| json!({}));
            let is_bridge = method == "engine.execute" && params.get("command").and_then(Value::as_str) == Some("bridge.import_svg");
            let start = std::time::Instant::now();
            if is_bridge {
                crate::freeze_diag::mark(crate::freeze_diag::Phase::BridgeBegin);
                trace("bridge.import_svg begin on UI thread");
            }
            let result: Result<Value, String> = match method {
                "automation.status" => Ok(json!({"port":self.port, "maxRequestBytes":MAX_REQUEST_BYTES, "maxSvgBytes":MAX_SVG_BYTES, "supportsEmbeddedImages":true})),
                "engine.execute" => {
                    match params.get("command").and_then(Value::as_str) {
                        Some(command) => app.s.execute(command, params.get("params").unwrap_or(&Value::Null)).map_err(|e| e.to_string()),
                        None => Err("Missing command".into()),
                    }
                },
                #[cfg(feature="python")]
                "python.info" => Ok(crate::python::info()),
                _ => Err("Unknown method".into()),
            };
            let answer = match result {
                Ok(value) => json!({"id":id,"result":value}),
                Err(message) => json!({"id":id,"error":{"message":message}}),
            };
            let _ = req.reply.try_send(answer);
            if is_bridge {
                imported = true;
                crate::freeze_diag::mark(crate::freeze_diag::Phase::BridgeDone);
                trace(&format!("bridge.import_svg done on UI thread in {} ms", start.elapsed().as_millis()));
                // Process only one potentially expensive SVG/bitmap import per frame.
                // Remaining requests are picked up on the next frame.
                break;
            }
        }
        imported
    }
}
// Optional diagnostics: set ANICRAFT_BRIDGE_LOG to a writable file path.
fn trace(message: &str) {
    let Some(path) = std::env::var_os("ANICRAFT_BRIDGE_LOG") else { return };
    let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(std::path::PathBuf::from(path)) else { return };
    let _ = writeln!(file, "{:?} {:?} {message}", std::time::SystemTime::now(), thread::current().id());
}

// Read one bounded JSON line without first allocating an unbounded input string.
fn read_request(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().map_err(|error| io::Error::new(error.kind(), format!("read failed after {} bytes: {error}", line.len())))?;
        if available.is_empty() {
            if line.is_empty() { return Ok(None); }
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "JSON request ended before newline"));
        }
        let take = available.iter().position(|c| *c == b'\n').map_or(available.len(), |index| index + 1);
        let too_large = line.len().saturating_add(take) > MAX_REQUEST_BYTES;
        if !too_large { line.extend_from_slice(&available[..take]); }
        let completed = available.get(take.saturating_sub(1)) == Some(&b'\n');
        reader.consume(take);
        if too_large {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "JSON request exceeds 64 MiB"));
        }
        if completed { return Ok(Some(line)); }
    }
}

fn client(mut stream: TcpStream, tx: mpsc::SyncSender<Request>, ctx: egui::Context) {
    // Large inline PNG/JPEG data can take longer to transfer and decode than vector-only SVG.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(120)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(60)));
    let peer = stream.peer_addr().map(|a| a.to_string()).unwrap_or_else(|_| "unknown".into());
    trace(&format!("accepted {peer}"));
    let cloned = match stream.try_clone() { Ok(c) => c, Err(error) => { trace(&format!("clone failed {peer}: {error}")); return } };
    let mut reader = BufReader::new(cloned);
    loop {
        let line = match read_request(&mut reader) {
            Ok(Some(line)) => line,
            Ok(None) => { trace(&format!("closed by peer {peer}")); return },
            Err(error) => {
                trace(&format!("read error {peer}: {error}"));
                let answer = json!({"id":null,"error":{"message":error.to_string()}});
                let _ = writeln!(stream, "{answer}");
                return;
            }
        };
        trace(&format!("received {peer}: {} bytes", line.len()));
        let request: Value = match serde_json::from_slice(&line) {
            Ok(v) => v,
            Err(error) => {
                trace(&format!("invalid JSON {peer}: {error}"));
                let answer = json!({"id":null,"error":{"message":format!("Invalid JSON request: {error}")}});
                let _ = writeln!(stream, "{answer}");
                return;
            }
        };
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        // A capabilities probe is independent of document/UI state. Answer it on
        // this network thread, so opening a document or a slow render cannot stall
        // the sender before the artwork itself is transferred.
        if method == "automation.status" {
            let port = stream.local_addr().map(|addr| addr.port()).unwrap_or(0);
            let answer = json!({"id":request.get("id").cloned().unwrap_or(Value::Null),
                "result":{"port":port,"maxRequestBytes":MAX_REQUEST_BYTES,
                "maxSvgBytes":MAX_SVG_BYTES,"supportsEmbeddedImages":true}});
            if let Err(error) = writeln!(stream, "{answer}") {
                trace(&format!("status response failed {peer}: {error}"));
                return;
            }
            trace(&format!("status response {peer}"));
            continue;
        }
        let command = request.get("params").and_then(|v| v.get("command")).and_then(Value::as_str).unwrap_or("?").to_owned();
        trace(&format!("enqueue {peer} method={method} command={command}"));
        let request_id = request.get("id").cloned().unwrap_or(Value::Null);
        let (reply, rx) = mpsc::sync_channel(1);
        if tx.try_send(Request {value:request, reply}).is_err() {
            trace(&format!("queue full {peer}"));
            let answer = json!({"id":request_id,"error":{"message":"AniCraft command queue is busy"}});
            let _ = writeln!(stream, "{answer}");
            return;
        }
        ctx.request_repaint();
        let answer = match rx.recv_timeout(Duration::from_secs(120)) {
            Ok(answer) => answer,
            Err(error) => {
                trace(&format!("response timeout {peer} command={command}: {error}"));
                json!({"id":request_id,"error":{"message":"AniCraft timed out while executing the command"}})
            }
        };
        trace(&format!("response {peer} command={command} error={}", answer.get("error").is_some()));
        if let Err(error) = writeln!(stream, "{answer}") {
            trace(&format!("write error {peer}: {error}"));
            return;
        }
    }
}
pub(crate) fn button(app: &mut App, ui: &mut egui::Ui) {
    let label = app.remote.port.map_or_else(|| "API off".to_owned(), |p| format!("API :{p}"));
    ui.menu_button(label, |ui| {
        ui.label("Automation · localhost JSON TCP");
        if let Some(p) = app.remote.port {
            ui.label(format!("127.0.0.1:{p}"));
            if ui.button("Disable access").clicked() { app.remote.stop(); ui.close(); }
        } else {
            ui.add(egui::DragValue::new(&mut app.remote.chosen).range(1..=65535).prefix("Port "));
            if ui.button("Enable access").clicked() {
                if let Err(e) = app.remote.start(ui.ctx().clone()) { app.remote.error = e; }
                else { app.remote.error.clear(); }
            }
        }
        if !app.remote.error.is_empty() { ui.label(&app.remote.error); }
    });
}
