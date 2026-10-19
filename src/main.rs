#![windows_subsystem = "windows"]
mod info;
mod native;
mod shell;

use serde_json::{json, Value};
use std::borrow::Cow;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tao::dpi::{LogicalSize, PhysicalPosition};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::windows::WindowExtWindows;
use tao::window::{Window, WindowBuilder};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem};
use tray_icon::TrayIconBuilder;
use wry::{http::Response, WebContext, WebView, WebViewBuilder, WebViewExtWindows};

/// 预览窗口隐藏多久后释放 WebView2 进程（释放后再打开会多 0.3~1 秒冷启动）
const RELEASE_AFTER: Duration = Duration::from_secs(300);

/// 环境变量 PEEK_RELEASE_SECS 可改释放时间（调试用）
fn release_after() -> Duration {
    std::env::var("PEEK_RELEASE_SECS").ok().and_then(|v| v.parse().ok()).map(Duration::from_secs).unwrap_or(RELEASE_AFTER)
}

enum UserEvent {
    Preview { fg: isize, items: Vec<String> },
    List { items: Vec<String>, index: usize },
    Js(String),
    TrayRetry,
    Ipc(String),
    Menu(MenuEvent),
    Tick,
}

enum WorkerCmd {
    Query(isize),
    Select(String),
}

macro_rules! asset {
    ($p:literal, $m:literal) => {
        ($p, include_bytes!(concat!("../ui/", $p)) as &[u8], $m)
    };
}
static ASSETS: &[(&str, &[u8], &str)] = &[
    asset!("index.html", "text/html; charset=utf-8"),
    asset!("app.js", "text/javascript; charset=utf-8"),
    asset!("app.css", "text/css; charset=utf-8"),
    asset!("vendor/docx-preview.min.js", "text/javascript"),
    asset!("vendor/jszip.min.js", "text/javascript"),
    asset!("vendor/xlsx.full.min.js", "text/javascript"),
    asset!("vendor/pptx-preview.umd.js", "text/javascript"),
    asset!("vendor/marked.umd.js", "text/javascript"),
    asset!("vendor/highlight.min.js", "text/javascript"),
    asset!("vendor/hl-light.css", "text/css"),
    asset!("vendor/hl-dark.css", "text/css"),
];

fn serve(req: wry::http::Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let uri = req.uri();
    let path = uri.path().trim_start_matches('/');
    if path == "thumb" {
        let q = uri.query().unwrap_or("");
        let p = q.split('&').find_map(|kv| kv.strip_prefix("p=")).map(url_decode).unwrap_or_default();
        let s = q.split('&').find_map(|kv| kv.strip_prefix("s=")).and_then(|v| v.parse().ok()).unwrap_or(1024);
        return match native::thumbnail_bmp(&p, s) {
            Some(b) => Response::builder().header("Content-Type", "image/bmp").header("Cache-Control", "no-store").body(Cow::Owned(b)).unwrap(),
            None => Response::builder().status(404).body(Cow::Borrowed(&[][..])).unwrap(),
        };
    }
    let path = if path.is_empty() { "index.html" } else { path };
    match ASSETS.iter().find(|a| a.0 == path) {
        Some(a) => Response::builder().header("Content-Type", a.2).body(Cow::Borrowed(a.1)).unwrap(),
        None => Response::builder().status(404).body(Cow::Borrowed(&[][..])).unwrap(),
    }
}

fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

struct App {
    window: Window,
    ctx: WebContext,
    webview: Option<WebView>,
    ready: bool,
    pending: Option<String>,
    items: Vec<String>,
    index: usize,
    prev_fg: isize,
    hidden_at: Option<Instant>,
    pinned: bool,
    proxy: EventLoopProxy<UserEvent>,
    worker: mpsc::Sender<WorkerCmd>,
}

impl App {
    fn ensure_webview(&mut self) {
        if self.webview.is_some() {
            return;
        }
        let p = self.proxy.clone();
        let wv = WebViewBuilder::new_with_web_context(&mut self.ctx)
            .with_custom_protocol("peek".into(), |_id, req| serve(req))
            .with_url("http://peek.localhost/index.html")
            .with_ipc_handler(move |req| {
                let _ = p.send_event(UserEvent::Ipc(req.body().clone()));
            })
            .with_devtools(cfg!(debug_assertions))
            .with_background_color((32, 32, 32, 255))
            .build(&self.window)
            .expect("WebView2 创建失败");
        map_drives(&wv);
        self.webview = Some(wv);
        self.ready = false;
    }

    fn eval(&mut self, js: String) {
        match (&self.webview, self.ready) {
            (Some(w), true) => {
                let _ = w.evaluate_script(&js);
            }
            _ => self.pending = Some(js),
        }
    }

    fn show_current(&mut self) {
        let Some(path) = self.items.get(self.index).cloned() else { return };
        let v = info::basic(&path, self.index, self.items.len());
        let is_dir = v["isDir"].as_bool().unwrap_or(false);
        self.window.set_title(v["name"].as_str().unwrap_or("Peek"));
        self.eval(format!("peek.show({})", v));
        if is_dir {
            let p = self.proxy.clone();
            std::thread::spawn(move || {
                let s = info::folder_stats(&path);
                let _ = p.send_event(UserEvent::Js(format!("peek.folder({})", s)));
            });
        }
    }

    fn open_preview(&mut self, fg: isize, items: Vec<String>) {
        let visible = self.window.is_visible();
        // 预览已开、再对同一个文件按空格 => 关闭
        if visible && items.len() == 1 && self.items.get(self.index) == items.first() {
            self.hide();
            return;
        }
        self.prev_fg = fg;
        self.items = items;
        self.index = 0;
        self.ensure_webview();
        if !visible {
            self.place(fg);
        }
        self.show_current();
        self.window.set_visible(true);
        if let Some(w) = &self.webview {
            let _ = w.set_memory_usage_level(wry::MemoryUsageLevel::Normal);
        }
        native::force_foreground(hwnd_of(&self.window));
        if let Some(w) = &self.webview {
            let _ = w.focus();
        }
        self.hidden_at = None;
    }

    fn place(&self, fg: isize) {
        if let Some((x, y, w, h)) = native::monitor_work_area(fg) {
            let size = self.window.outer_size();
            let (ww, wh) = (size.width as i32, size.height as i32);
            self.window.set_outer_position(PhysicalPosition::new(x + (w - ww) / 2, y + (h - wh) / 2));
        }
    }

    /// 按内容比例设窗口大小并居中：内容区 cw×ch（CSS 像素），ew/eh 是界面框架占的宽高。
    /// 最大占工作区 88%，内容比这小就按原尺寸，不放大；最小 420×300
    fn fit(&self, cw: f64, ch: f64, ew: f64, eh: f64) {
        if cw < 1.0 || ch < 1.0 || self.window.is_maximized() {
            return;
        }
        let anchor = if self.prev_fg != 0 { self.prev_fg } else { hwnd_of(&self.window).0 as isize };
        let Some((x, y, w, h)) = native::monitor_work_area(anchor) else { return };
        let s = self.window.scale_factor();
        let (maxw, maxh) = ((w as f64 * 0.88 / s - ew).max(1.0), (h as f64 * 0.88 / s - eh).max(1.0));
        let k = (maxw / cw).min(maxh / ch).min(1.0);
        let size = LogicalSize::new((cw * k + ew).max(420.0).round(), (ch * k + eh).max(300.0).round());
        if self.window.inner_size().to_logical::<f64>(s) == size {
            return;
        }
        self.window.set_inner_size(size);
        let o = self.window.outer_size();
        self.window.set_outer_position(PhysicalPosition::new(x + (w - o.width as i32) / 2, y + (h - o.height as i32) / 2));
    }

    fn hide(&mut self) {
        if !self.window.is_visible() {
            return;
        }
        if let Some(w) = &self.webview {
            let _ = w.evaluate_script("peek.clear()");
        }
        self.window.set_visible(false);
        if let Some(w) = &self.webview {
            let _ = w.set_memory_usage_level(wry::MemoryUsageLevel::Low);
        }
        self.hidden_at = Some(Instant::now());
        native::set_foreground(self.prev_fg);
    }

    fn step(&mut self, d: i64) {
        if self.items.len() < 2 {
            return;
        }
        let n = self.items.len() as i64;
        self.index = (((self.index as i64 + d) % n + n) % n) as usize;
        let _ = self.worker.send(WorkerCmd::Select(self.items[self.index].clone()));
        self.show_current();
    }

    fn ipc(&mut self, msg: &str) {
        let v: Value = serde_json::from_str(msg).unwrap_or_default();
        let cmd = v["cmd"].as_str().unwrap_or("");
        let cur = self.items.get(self.index).cloned().unwrap_or_default();
        match cmd {
            "ready" => {
                self.ready = true;
                if let Some(js) = self.pending.take() {
                    self.eval(js);
                }
                let js = format!("peek.setPinned({})", self.pinned);
                self.eval(js);
            }
            "close" => self.hide(),
            "fit" => {
                let f = |k: &str| v[k].as_f64().unwrap_or(0.0);
                self.fit(f("w"), f("h"), f("ew"), f("eh"));
            }
            "prev" => self.step(-1),
            "next" => self.step(1),
            "open" => {
                native::shell_open(&cur);
                self.hide();
            }
            "pin" => {
                self.pinned = !self.pinned;
                self.window.set_always_on_top(self.pinned);
                let js = format!("peek.setPinned({})", self.pinned);
                self.eval(js);
            }
            "hash" => {
                let p = self.proxy.clone();
                std::thread::spawn(move || {
                    let h = info::hashes(&cur);
                    let _ = p.send_event(UserEvent::Js(format!("peek.hash({})", h)));
                });
            }
            "ocr" => {
                let p = self.proxy.clone();
                std::thread::spawn(move || {
                    let r = match native::ocr(&cur) {
                        Ok(t) => json!({"text": t}),
                        Err(e) => json!({"error": e}),
                    };
                    let _ = p.send_event(UserEvent::Js(format!("peek.ocr({})", r)));
                });
            }
            "convert" => {
                let p = self.proxy.clone();
                std::thread::spawn(move || {
                    let r = match info::convert_pdf(&cur) {
                        Ok(pdf) => json!({"path": cur, "url": info::file_url(&pdf)}),
                        Err(e) => json!({"path": cur, "error": e}),
                    };
                    let _ = p.send_event(UserEvent::Js(format!("peek.converted({})", r)));
                });
            }
            _ => {}
        }
    }
}

fn hwnd_of(w: &Window) -> windows::Win32::Foundation::HWND {
    windows::Win32::Foundation::HWND(w.hwnd() as _)
}

/// 每个盘符映射成虚拟主机 x.peekfs，网页直接按 URL 读本地文件（支持视频拖动、Range 请求）
fn map_drives(wv: &WebView) {
    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    use windows::core::{Interface, HSTRING};
    let Ok(core3) = wv.webview().cast::<ICoreWebView2_3>() else { return };
    let mask = unsafe { windows::Win32::Storage::FileSystem::GetLogicalDrives() };
    for i in 0..26u32 {
        if mask & (1 << i) != 0 {
            let l = (b'a' + i as u8) as char;
            unsafe {
                let _ = core3.SetVirtualHostNameToFolderMapping(
                    &HSTRING::from(format!("{}.peekfs", l)),
                    &HSTRING::from(format!("{}:\\", l.to_ascii_uppercase())),
                    COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_ALLOW,
                );
            }
        }
    }
}

fn tray_icon_rgba() -> tray_icon::Icon {
    // 32x32 圆角方块 + 眼睛
    let n = 32usize;
    let mut px = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32 - 15.5, y as f32 - 15.5);
            let i = (y * n + x) * 4;
            let inside = fx.abs() < 15.0 && fy.abs() < 15.0 && !(fx.abs() > 11.0 && fy.abs() > 11.0 && (fx.abs() - 11.0).hypot(fy.abs() - 11.0) > 4.0);
            if !inside {
                continue;
            }
            let eye = (fx / 12.0).powi(2) + (fy / 7.0).powi(2) < 1.0;
            let pupil = fx.hypot(fy) < 4.5;
            let c: [u8; 3] = if pupil { [30, 64, 175] } else if eye { [255, 255, 255] } else { [37, 99, 235] };
            px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    tray_icon::Icon::from_rgba(px, n as u32, n as u32).unwrap()
}

/// 自检：列出所有资源管理器窗口的选中项，写到 %TEMP%\peek-selftest.txt
fn selftest() {
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetWindowThreadProcessId};
    let mut out = String::new();
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        let mut h = windows::Win32::Foundation::HWND::default();
        loop {
            h = match FindWindowExW(None, Some(h), &windows::core::HSTRING::from("CabinetWClass"), None) { Ok(x) => x, Err(_) => break };
            let _ = GetWindowThreadProcessId(h, None);
            let t = std::time::Instant::now();
            match shell::selection(h) {
                Some((sel, v)) => {
                    let all = v.all();
                    out += &format!("hwnd={:?} sel={:?} all={} ({:?})
", h.0, sel, all.len(), t.elapsed());
                }
                None => out += &format!("hwnd={:?} 无选中
", h.0),
            }
        }
    }
    let _ = std::fs::write(std::env::temp_dir().join("peek-selftest.txt"), out);
}

fn single_instance() -> bool {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    unsafe {
        let _ = windows::Win32::System::Threading::CreateMutexW(None, true, &HSTRING::from("Local\\PeekQuickLookSingleton"));
        GetLastError() != ERROR_ALREADY_EXISTS
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|a| a == "--selftest").unwrap_or(false) {
        selftest();
        return;
    }
    if !single_instance() {
        // 已在运行：把参数里的路径交给它预览
        if let Some(p) = args.first() {
            let full = std::fs::canonicalize(p).map(|x| x.to_string_lossy().trim_start_matches(r"\\?\").to_string()).unwrap_or(p.clone());
            native::send_to_running(&full);
        }
        return;
    }
    // panic=abort 下进程会静默退出，先把原因记到 %LOCALAPPDATA%\Peek\crash.log
    std::panic::set_hook(Box::new(|i| {
        let f = std::path::PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("Peek").join("crash.log");
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        if let Ok(mut fh) = std::fs::OpenOptions::new().create(true).append(true).open(f) {
            use std::io::Write;
            let _ = writeln!(fh, "[unix {}] {}", t, i);
        }
    }));
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // 首次运行默认开机自启；用户在托盘关掉后写一个标记，不再自动打开
    let data = std::path::PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("Peek");
    let _ = std::fs::create_dir_all(&data);
    let off_flag = data.join("autostart.off");
    if !off_flag.exists() && !native::autostart_enabled() {
        native::set_autostart(true);
    }

    // Shell 工作线程（STA）：查询选中项、同步选中
    let (wtx, wrx) = mpsc::channel::<WorkerCmd>();
    {
        let p = proxy.clone();
        std::thread::spawn(move || unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
            let mut view: Option<shell::View> = None;
            for cmd in wrx {
                match cmd {
                    WorkerCmd::Query(fg) => {
                        let Some((sel, v)) = shell::selection(windows::Win32::Foundation::HWND(fg as _)) else { continue };
                        let single = sel.len() == 1;
                        let first = sel[0].clone();
                        let _ = p.send_event(UserEvent::Preview { fg, items: sel });
                        if single {
                            // 先出预览，再取整个视图做左右切换
                            let all = v.all();
                            if let Some(i) = all.iter().position(|x| *x == first) {
                                let _ = p.send_event(UserEvent::List { items: all, index: i });
                            }
                        }
                        view = Some(v);
                    }
                    WorkerCmd::Select(path) => {
                        if let Some(v) = &view {
                            shell::select_in_view(v, &path);
                        }
                    }
                }
            }
        });
    }
    {
        let (itx, irx) = mpsc::channel::<String>();
        native::start_ipc(itx);
        let p = proxy.clone();
        std::thread::spawn(move || {
            for path in irx {
                let _ = p.send_event(UserEvent::Preview { fg: 0, items: vec![path] });
            }
        });
    }
    let (htx, hrx) = mpsc::channel::<isize>();
    native::start_hook(htx);
    {
        let w = wtx.clone();
        std::thread::spawn(move || {
            for fg in hrx {
                let _ = w.send(WorkerCmd::Query(fg));
            }
        });
    }
    {
        let p = proxy.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(30));
            if p.send_event(UserEvent::Tick).is_err() {
                break;
            }
        });
    }

    let window = WindowBuilder::new()
        .with_title("Peek")
        .with_visible(false)
        .with_inner_size(LogicalSize::new(1100.0, 760.0))
        .with_min_inner_size(LogicalSize::new(420.0, 300.0))
        .build(&event_loop)
        .unwrap();
    native::OWN_HWND.store(window.hwnd() as isize, std::sync::atomic::Ordering::Relaxed);

    let menu = Menu::new();
    let auto_item = CheckMenuItem::new("开机自启", true, native::autostart_enabled(), None);
    let quit_item = MenuItem::new("退出", true, None);
    let _ = menu.append(&auto_item);
    let _ = menu.append(&quit_item);
    // 开机自启时任务栏可能还没起来，托盘创建会失败；失败不退出，前两分钟每 2 秒重试一次，之后跟着 Tick 重试
    let make_tray = move || TrayIconBuilder::new().with_menu(Box::new(menu.clone())).with_tooltip("Peek 快速预览（资源管理器里选中文件按空格）").with_icon(tray_icon_rgba()).build().ok();
    let mut tray = make_tray();
    if tray.is_none() {
        let p = proxy.clone();
        std::thread::spawn(move || {
            for _ in 0..60 {
                std::thread::sleep(Duration::from_secs(2));
                if p.send_event(UserEvent::TrayRetry).is_err() {
                    break;
                }
            }
        });
    }
    {
        let p = proxy.clone();
        MenuEvent::set_event_handler(Some(move |e| {
            let _ = p.send_event(UserEvent::Menu(e));
        }));
    }

    let mut app = App {
        window,
        ctx: WebContext::new(Some(data.join("WebView2"))),
        webview: None,
        ready: false,
        pending: None,
        items: vec![],
        index: 0,
        prev_fg: 0,
        hidden_at: None,
        pinned: false,
        proxy,
        worker: wtx,
    };

    event_loop.run(move |event, _, cf| {
        *cf = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Preview { fg, items }) => app.open_preview(fg, items),
            Event::UserEvent(UserEvent::List { items, index }) => {
                // 只在用户还停留在同一个文件时替换列表
                if app.items.len() == 1 && app.items.get(0) == items.get(index) {
                    app.items = items;
                    app.index = index;
                    let js = format!("peek.position({},{})", app.index, app.items.len());
                    app.eval(js);
                }
            }
            Event::UserEvent(UserEvent::Js(js)) => app.eval(js),
            Event::UserEvent(UserEvent::Ipc(m)) => app.ipc(&m),
            Event::UserEvent(UserEvent::Menu(e)) => {
                if e.id == quit_item.id() {
                    *cf = ControlFlow::Exit;
                } else if e.id == auto_item.id() {
                    let on = !native::autostart_enabled();
                    native::set_autostart(on);
                    if on {
                        let _ = std::fs::remove_file(&off_flag);
                    } else {
                        let _ = std::fs::write(&off_flag, b"");
                    }
                    auto_item.set_checked(on);
                }
            }
            Event::UserEvent(UserEvent::TrayRetry) => {
                if tray.is_none() {
                    tray = make_tray();
                }
            }
            Event::UserEvent(UserEvent::Tick) => {
                if tray.is_none() {
                    tray = make_tray();
                }
                if let Some(t) = app.hidden_at {
                    if t.elapsed() > release_after() && app.webview.is_some() {
                        app.webview = None;
                        app.ready = false;
                    }
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => app.hide(),
            _ => {}
        }
    });
}
