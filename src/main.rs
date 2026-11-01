#![windows_subsystem = "windows"]
mod info;
mod native;
mod paddle;
mod shell;

use serde_json::{json, Value};
use std::borrow::Cow;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tao::dpi::{LogicalSize, PhysicalPosition};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::windows::{IconExtWindows, WindowBuilderExtWindows, WindowExtWindows};
use tao::window::{Icon, Window, WindowBuilder};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem};
use tray_icon::TrayIconBuilder;
use wry::{http::Response, WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows};

/// 预览窗口隐藏多久后释放 WebView2 进程（释放后再打开会多 0.3~1 秒冷启动）
const RELEASE_AFTER: Duration = Duration::from_secs(300);
/// 顶栏高度（逻辑像素，和 app.css #bar 一致）
const BAR: f64 = 38.0;

/// 环境变量 PEEK_RELEASE_SECS 可改释放时间（调试用）
fn release_after() -> Duration {
    std::env::var("PEEK_RELEASE_SECS").ok().and_then(|v| v.parse().ok()).map(Duration::from_secs).unwrap_or(RELEASE_AFTER)
}

enum UserEvent {
    Preview { fg: isize, items: Vec<String> },
    List { items: Vec<String>, index: usize },
    Js(String),
    TrayRetry,
    /// 预览窗口在前台时钩子截下的按键（native::KEY_*）
    Key(isize),
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
    asset!("vendor/github-markdown.css", "text/css"),
];

fn bytes(ty: &str, b: Option<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    match b {
        Some(b) => Response::builder().header("Content-Type", ty).header("Cache-Control", "no-store").body(Cow::Owned(b)).unwrap(),
        None => Response::builder().status(404).body(Cow::Borrowed(&[][..])).unwrap(),
    }
}

/// 缩略图和 PDF 渲染要几十到几百毫秒，放到后台线程，别卡住 UI 线程
fn serve_async(req: wry::http::Request<Vec<u8>>, res: wry::RequestAsyncResponder) {
    let path = req.uri().path().trim_start_matches('/').to_string();
    if !matches!(path.as_str(), "thumb" | "pdfinfo" | "pdfpage" | "imgconv") {
        return res.respond(serve(&path));
    }
    if path == "imgconv" {
        // 文档里浏览器解不了的内嵌图（EMF/WMF/TIFF），页面把字节 POST 过来
        let body = req.into_body();
        return drop(std::thread::spawn(move || res.respond(bytes("image/bmp", native::convert_image(&body)))));
    }
    let q = req.uri().query().unwrap_or("").to_string();
    let arg = |k: &str| q.split('&').find_map(|kv| kv.strip_prefix(k).and_then(|v| v.strip_prefix('='))).map(url_decode);
    let p = arg("p").unwrap_or_default();
    let num = |k: &str, d: u32| arg(k).and_then(|v| v.parse().ok()).unwrap_or(d);
    let (s, n, w) = (num("s", 1024), num("n", 0), num("w", 1200));
    std::thread::spawn(move || {
        res.respond(match path.as_str() {
            "thumb" => bytes("image/bmp", native::thumbnail_bmp(&p, s as i32)),
            "pdfinfo" => bytes("application/json", native::pdf_pages(&p).ok().map(String::into_bytes)),
            _ => bytes("image/jpeg", native::pdf_render(&p, n, w).ok()),
        })
    });
}

fn serve(path: &str) -> Response<Cow<'static, [u8]>> {
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
    // 页面就绪前要执行的脚本按顺序排队（show 之后紧跟 position，只留一条会把 show 覆盖掉，冷启动时窗口空白）
    pending: Vec<String>,
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
            .with_asynchronous_custom_protocol("peek".into(), |_id, req, res| serve_async(req, res))
            // 替换 wry 的默认参数（保留它关掉的那几项），再关掉组件自动更新：不然 WebView2 会往数据目录下载约 57 MB 用不到的组件
            .with_additional_browser_args("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --disable-component-update")
            .with_url("http://peek.localhost/index.html")
            .with_ipc_handler(move |req| {
                let _ = p.send_event(UserEvent::Ipc(req.body().clone()));
            })
            .with_devtools(cfg!(debug_assertions))
            .with_background_color(if native::dark_mode() { (30, 30, 32, 255) } else { (246, 246, 247, 255) })
            .build(&self.window)
            .expect("WebView2 创建失败");
        map_drives(&wv);
        hook_keys(&wv, self.proxy.clone());
        self.webview = Some(wv);
        self.ready = false;
        self.pending.clear();
    }

    fn eval(&mut self, js: String) {
        match (&self.webview, self.ready) {
            (Some(w), true) => {
                let _ = w.evaluate_script(&js);
            }
            _ => self.pending.push(js),
        }
    }

    /// 先按文件类型把窗口调到目标大小，再交给页面渲染
    fn show_current(&mut self) {
        let Some(path) = self.items.get(self.index).cloned() else { return };
        let v = info::basic(&path, self.index, self.items.len());
        self.window.set_title(v["name"].as_str().unwrap_or("QuickView"));
        let f = |k: &str| v[k].as_f64().unwrap_or(0.0);
        self.resize(f("pw"), f("ph"), f("pr"));
        self.eval(format!("peek.show({})", v));
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

    /// 内容区 cw×ch（逻辑像素）加顶栏，按比例缩到不超过工作区的 ratio，不放大；最小 300×200：原版最小宽 400，窄长图（竖拍照片、小图）两边会留白。
    /// 第一次显示在工作区居中；已显示时（切换文件、页面校正尺寸）保持窗口中心不动，只改大小
    fn resize(&self, cw: f64, ch: f64, ratio: f64) {
        if cw < 1.0 || ch < 1.0 || self.window.is_maximized() {
            return;
        }
        let visible = self.window.is_visible();
        let anchor = if visible || self.prev_fg == 0 { hwnd_of(&self.window).0 as isize } else { self.prev_fg };
        let Some((x, y, w, h)) = native::monitor_work_area(anchor) else { return };
        let s = self.window.scale_factor();
        let (maxw, maxh) = (w as f64 * ratio / s, h as f64 * ratio / s - BAR);
        let k = (maxw / cw).min(maxh / ch).min(1.0);
        let size = LogicalSize::new((cw * k).max(300.0).round(), (ch * k + BAR).max(200.0).round());
        let (old_pos, old) = (self.window.outer_position().unwrap_or_default(), self.window.outer_size());
        if self.window.inner_size().to_logical::<f64>(s) != size {
            self.window.set_inner_size(size);
        }
        let o = self.window.outer_size();
        let (ow, oh) = (o.width as i32, o.height as i32);
        let (cx, cy) = if visible { (old_pos.x + old.width as i32 / 2, old_pos.y + old.height as i32 / 2) } else { (x + w / 2, y + h / 2) };
        let nx = (cx - ow / 2).clamp(x, (x + w - ow).max(x));
        let ny = (cy - oh / 2).clamp(y, (y + h - oh).max(y));
        self.window.set_outer_position(PhysicalPosition::new(nx, ny));
    }

    fn hide(&mut self) {
        if !self.window.is_visible() {
            return;
        }
        if let Some(w) = &self.webview {
            let _ = w.evaluate_script("peek.clear()");
        }
        self.window.set_visible(false);
        native::TYPING.store(false, std::sync::atomic::Ordering::Relaxed);
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
                for js in std::mem::take(&mut self.pending) {
                    self.eval(js);
                }
                let js = format!("peek.setPinned({})", self.pinned);
                self.eval(js);
            }
            "close" => self.hide(),
            "typing" => native::TYPING.store(v["on"].as_bool().unwrap_or(false), std::sync::atomic::Ordering::Relaxed),
            "drag" => {
                let _ = self.window.drag_window();
            }
            // 页面拿到真实尺寸后校正（系统属性读不到尺寸的图片/视频，如 SVG）
            "fit" => {
                let f = |k: &str| v[k].as_f64().unwrap_or(0.0);
                self.resize(f("w"), f("h"), 0.8);
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
                    let mut h = info::hashes(&cur);
                    h["path"] = cur.into();
                    let _ = p.send_event(UserEvent::Js(format!("peek.hash({})", h)));
                });
            }
            "ocr" => {
                let p = self.proxy.clone();
                std::thread::spawn(move || {
                    let r = match native::ocr(&cur) {
                        Ok(t) => json!({"path": cur, "text": t}),
                        Err(e) => json!({"path": cur, "error": e}),
                    };
                    let _ = p.send_event(UserEvent::Js(format!("peek.ocr({})", r)));
                });
            }
            "convert" => {
                let p = self.proxy.clone();
                std::thread::spawn(move || {
                    let r = match info::convert_pdf(&cur) {
                        Ok(pdf) => json!({"path": cur, "pdf": pdf, "url": info::file_url(&pdf)}),
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

/// 焦点在 WebView 里任何位置（视频控件、PDF 查看器）时，空格/Esc/↑/↓ 都先经过这里，网页拿不到
fn hook_keys(wv: &WebView, proxy: EventLoopProxy<UserEvent>) {
    use webview2_com::AcceleratorKeyPressedEventHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    use wry::WebViewExtWindows;
    let handler = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let (mut kind, mut vk) = (COREWEBVIEW2_KEY_EVENT_KIND::default(), 0u32);
        unsafe {
            args.KeyEventKind(&mut kind)?;
            args.VirtualKey(&mut vk)?;
        }
        if kind == COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN {
            let cmd = native::preview_key(vk);
            if cmd != 0 {
                unsafe { args.SetHandled(true)? };
                let _ = proxy.send_event(UserEvent::Key(cmd));
            }
        }
        Ok(())
    }));
    let mut token = 0i64;
    unsafe {
        let _ = wv.controller().add_AcceleratorKeyPressed(&handler, &mut token);
    }
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

/// 自检：列出所有资源管理器窗口的选中项，写到 %TEMP%\quickview-selftest.txt
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
    let _ = std::fs::write(std::env::temp_dir().join("quickview-selftest.txt"), out);
}

fn single_instance() -> bool {
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    unsafe {
        let _ = windows::Win32::System::Threading::CreateMutexW(None, true, &HSTRING::from("Local\\QuickViewSingleton"));
        GetLastError() != ERROR_ALREADY_EXISTS
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|a| a == "--selftest").unwrap_or(false) {
        selftest();
        return;
    }
    // 调试用：QuickView.exe --ocr <图片> <输出文件>，把识别结果写进输出文件（窗口程序没有控制台）
    if args.len() == 3 && args[0] == "--ocr" {
        let t = std::time::Instant::now();
        // QUICKVIEW_OCR=paddle 只走 PaddleOCR，出错直接写出错误（正常流程出错会静默退回系统 OCR）
        let r = if std::env::var("QUICKVIEW_OCR").as_deref() == Ok("paddle") { paddle::ocr(&args[1]) } else { native::ocr(&args[1]) };
        let r = r.unwrap_or_else(|e| format!("ERR {e}"));
        let _ = std::fs::write(&args[2], format!("{}ms\n{r}", t.elapsed().as_millis()));
        return;
    }
    // 调试用：QuickView.exe --imgconv <EMF/WMF/TIFF> <输出.bmp>
    if args.len() == 3 && args[0] == "--imgconv" {
        if let Some(b) = std::fs::read(&args[1]).ok().and_then(|d| native::convert_image(&d)) {
            let _ = std::fs::write(&args[2], b);
        }
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
        let f = info::data_dir().join("crash.log");
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        if let Ok(mut fh) = std::fs::OpenOptions::new().create(true).append(true).open(f) {
            use std::io::Write;
            let _ = writeln!(fh, "[unix {}] {}", t, i);
        }
    }));
    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // 首次运行默认开机自启；用户在托盘关掉后写一个标记，不再自动打开
    // 旧名 Peek 的数据目录和自启项迁过来
    let data = info::data_dir();
    let old = data.with_file_name("Peek");
    if old.is_dir() && !data.exists() {
        let _ = std::fs::rename(&old, &data);
    }
    native::remove_old_autostart();
    let _ = std::fs::create_dir_all(&data);
    let off_flag = data.join("autostart.off");
    if !off_flag.exists() {
        native::ensure_autostart();
    }
    // WebView2 以前自动下载的组件（DRM、广告过滤规则等，约 57 MB，预览用不到）现在用 --disable-component-update 关掉了，
    // 留下的在创建 WebView2 之前删掉：WebView2 一启动就会从 component_crx_cache 把它们装回去并占住文件
    let wv = data.join("WebView2").join("EBWebView");
    for d in ["component_crx_cache", "WidevineCdm", "Subresource Filter"] {
        let _ = std::fs::remove_dir_all(wv.join(d));
    }
    // 转换出的 PDF 按 30 天 / 300 MB 回收，放后台
    std::thread::spawn(info::clean_cache);

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
        let kp = proxy.clone();
        std::thread::spawn(move || {
            for fg in hrx {
                if fg < 0 {
                    let _ = kp.send_event(UserEvent::Key(fg));
                } else {
                    let _ = w.send(WorkerCmd::Query(fg));
                }
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
        .with_title("QuickView")
        .with_visible(false)
        .with_inner_size(LogicalSize::new(880.0, 600.0))
        .with_decorations(false)
        .with_undecorated_shadow(true)
        .with_window_icon(Icon::from_resource(1, None).ok())
        .with_taskbar_icon(Icon::from_resource(1, None).ok())
        .with_min_inner_size(LogicalSize::new(300.0, 200.0))
        .build(&event_loop)
        .unwrap();
    native::OWN_HWND.store(window.hwnd() as isize, std::sync::atomic::Ordering::Relaxed);
    // 无边框窗口在第一次显示前还没重算非客户区，tao 会把标题栏高度算进 set_inner_size，第一次打开高出 29px；先强制重算一次
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::*;
        let _ = SetWindowPos(hwnd_of(&window), None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }

    // 托盘图标按系统小图标尺寸（随缩放）从 exe 资源里取对应那一帧
    let tray_px = (16.0 * window.scale_factor()).round() as u32;
    let tray_icon = tray_icon::Icon::from_resource(1, Some((tray_px, tray_px))).expect("exe 缺少图标资源");
    let menu = Menu::new();
    let auto_item = CheckMenuItem::new("开机自启", true, native::autostart_enabled(), None);
    let quit_item = MenuItem::new("退出", true, None);
    let _ = menu.append(&auto_item);
    let _ = menu.append(&quit_item);
    // 开机自启时任务栏可能还没起来，托盘创建会失败；失败不退出，前两分钟每 2 秒重试一次，之后跟着 Tick 重试
    let make_tray = move || TrayIconBuilder::new().with_menu(Box::new(menu.clone())).with_tooltip("QuickView 快速预览（资源管理器里选中文件按空格）").with_icon(tray_icon.clone()).build().ok();
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
        pending: Vec::new(),
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
            Event::UserEvent(UserEvent::Key(k)) => match k {
                native::KEY_CLOSE => app.hide(),
                native::KEY_PREV => app.step(-1),
                native::KEY_NEXT => app.step(1),
                _ => {}
            },
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
