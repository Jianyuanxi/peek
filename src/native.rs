// 键盘钩子、前台切换、开机自启、系统缩略图、OCR
use std::sync::mpsc::Sender;
use std::sync::OnceLock;
use windows::core::HSTRING;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

static HOOK_TX: OnceLock<Sender<isize>> = OnceLock::new();
/// 预览窗口句柄：它在前台时空格交给网页处理
pub static OWN_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

fn down(vk: VIRTUAL_KEY) -> bool {
    unsafe { (GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000) != 0 }
}

/// 网页里输入框（查找框）有焦点时为 true，这时预览窗口里的按键不拦截
pub static TYPING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// 钩子发往主线程的预览窗口按键命令（负数，和资源管理器窗口句柄区分开）
pub const KEY_CLOSE: isize = -1;
pub const KEY_PREV: isize = -2;
pub const KEY_NEXT: isize = -3;

unsafe extern "system" fn kb_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && (wp.0 as u32 == WM_KEYDOWN) {
        let k = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        let vk = k.vkCode;
        if !down(VK_CONTROL) && !down(VK_MENU) && !down(VK_SHIFT) && !down(VK_LWIN) && !down(VK_RWIN) {
            let fg = GetForegroundWindow();
            let send = |v: isize| {
                if let Some(tx) = HOOK_TX.get() {
                    let _ = tx.send(v);
                }
            };
            let own = OWN_HWND.load(std::sync::atomic::Ordering::Relaxed);
            if own != 0 && fg.0 as isize == own {
                // 预览窗口里这几个键在原生层处理：PDF 查看器、视频控件拿到焦点后网页收不到按键
                if !TYPING.load(std::sync::atomic::Ordering::Relaxed) {
                    let cmd = if vk == VK_SPACE.0 as u32 || vk == VK_ESCAPE.0 as u32 {
                        KEY_CLOSE
                    } else if vk == VK_UP.0 as u32 {
                        KEY_PREV
                    } else if vk == VK_DOWN.0 as u32 {
                        KEY_NEXT
                    } else {
                        0
                    };
                    if cmd != 0 {
                        send(cmd);
                        return LRESULT(1);
                    }
                }
            } else if vk == VK_SPACE.0 as u32 && is_explorer_list(fg) {
                send(fg.0 as isize);
                return LRESULT(1);
            }
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

/// 前台是资源管理器/桌面，且焦点在文件列表上（不在地址栏、搜索框、重命名框）
fn is_explorer_list(fg: HWND) -> bool {
    let cls = crate::shell::class_name(fg);
    if !matches!(cls.as_str(), "CabinetWClass" | "ExploreWClass" | "Progman" | "WorkerW") {
        return false;
    }
    unsafe {
        let tid = GetWindowThreadProcessId(fg, None);
        let mut gi = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        if GetGUIThreadInfo(tid, &mut gi).is_err() {
            return false;
        }
        if !gi.hwndCaret.is_invalid() && !gi.hwndCaret.0.is_null() {
            return false;
        }
        let fc = crate::shell::class_name(gi.hwndFocus);
        fc == "DirectUIHWND" || fc == "SysListView32"
    }
}

pub fn start_hook(tx: Sender<isize>) {
    let _ = HOOK_TX.set(tx);
    std::thread::spawn(|| unsafe {
        let _h = SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_proc), None, 0);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

/// 从后台进程抢前台：附加到当前前台线程的输入队列再 SetForegroundWindow
pub fn force_foreground(h: HWND) {
    unsafe {
        let fg = GetForegroundWindow();
        let ft = GetWindowThreadProcessId(fg, None);
        let me = GetCurrentThreadId();
        let attached = ft != me && AttachThreadInput(ft, me, true).as_bool();
        let _ = ShowWindow(h, SW_SHOW);
        let _ = BringWindowToTop(h);
        let _ = SetForegroundWindow(h);
        if attached {
            let _ = AttachThreadInput(ft, me, false);
        }
    }
}

pub fn set_foreground(h: isize) {
    unsafe {
        let _ = SetForegroundWindow(HWND(h as _));
    }
}

/// 窗口所在显示器的工作区 (x, y, w, h)
pub fn monitor_work_area(h: isize) -> Option<(i32, i32, i32, i32)> {
    unsafe {
        let m = MonitorFromWindow(HWND(h as _), MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(m, &mut mi).as_bool() {
            return None;
        }
        let r = mi.rcWork;
        Some((r.left, r.top, r.right - r.left, r.bottom - r.top))
    }
}

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_NAME: &str = "QuickView";

/// 系统"应用模式"是否为深色：WebView2 底色要和页面一致，否则页面画出来之前会闪一下别的颜色
pub fn dark_mode() -> bool {
    unsafe {
        let mut v = 1u32;
        let mut sz = 4u32;
        let k = HSTRING::from(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        let ok = RegGetValueW(HKEY_CURRENT_USER, &k, &HSTRING::from("AppsUseLightTheme"), RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as *mut _), Some(&mut sz)).is_ok();
        ok && v == 0
    }
}

// 自启用"启动"文件夹里的快捷方式，不用 HKCU\...\Run：本机（Win11 26300）开机时 Explorer 跳过了新写入的 Run 项
// （Shell-Core 日志里 QuickView、QuickCopy 都没有"已开始执行命令"，同次开机 Span、Everything 都执行了），
// 启动文件夹快捷方式同次开机正常启动。"设置 > 启动应用"关掉后系统在 StartupApproved\StartupFolder 记一个奇数首字节。
const APPROVED_RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const APPROVED_LNK: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder";
const LNK_NAME: &str = "QuickView.lnk";

/// 旧的 Run 自启项（改名前的 Peek 和之前版本的 QuickView）连同它们在 StartupApproved 里的记录一起删掉，
/// 不删的话开机会多起一个，或"设置 > 启动应用"留一条孤儿项
pub fn remove_old_autostart() {
    unsafe {
        for n in ["Peek", RUN_NAME] {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(RUN_KEY), &HSTRING::from(n));
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(APPROVED_RUN), &HSTRING::from(n));
        }
    }
}

fn startup_lnk() -> Option<std::path::PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_Startup, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as _));
        Some(std::path::PathBuf::from(s?).join(LNK_NAME))
    }
}

/// 快捷方式在，且没被"设置 > 启动应用"关掉
pub fn autostart_enabled() -> bool {
    if !startup_lnk().is_some_and(|p| p.exists()) {
        return false;
    }
    unsafe {
        let mut b = [0u8; 12];
        let mut sz = 12u32;
        let ok = RegGetValueW(HKEY_CURRENT_USER, &HSTRING::from(APPROVED_LNK), &HSTRING::from(LNK_NAME), RRF_RT_REG_BINARY, None, Some(b.as_mut_ptr() as _), Some(&mut sz)).is_ok();
        !ok || b[0] & 1 == 0
    }
}

/// 每次启动时调用：快捷方式不在、或指向的 exe 已不存在（换了安装位置）才重写，不动用户在"设置"里的开关
pub fn ensure_autostart() {
    let Some(p) = startup_lnk() else { return };
    if !p.exists() || lnk_target(&p).is_some_and(|t| !std::path::Path::new(&t).exists()) {
        let _ = write_lnk();
    }
}

fn lnk_target(lnk: &std::path::Path) -> Option<String> {
    use windows::core::Interface;
    use windows::Win32::System::Com::*;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let sl: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        sl.cast::<IPersistFile>().ok()?.Load(&HSTRING::from(lnk.as_os_str()), STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        sl.GetPath(&mut buf, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32).ok()?;
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..n]))
    }
}

fn write_lnk() -> windows::core::Result<()> {
    use windows::core::Interface;
    use windows::Win32::System::Com::*;
    let lnk = startup_lnk().ok_or(windows::core::Error::empty())?;
    let exe = std::env::current_exe().unwrap_or_default();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let sl: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        sl.SetPath(&HSTRING::from(exe.as_os_str()))?;
        if let Some(d) = exe.parent() {
            sl.SetWorkingDirectory(&HSTRING::from(d.as_os_str()))?;
        }
        sl.SetDescription(&HSTRING::from("QuickView 快速预览"))?;
        sl.cast::<IPersistFile>()?.Save(&HSTRING::from(lnk.as_os_str()), true)
    }
}

/// 托盘菜单开关：打开时建快捷方式并清掉"设置"里的禁用记录，关闭时删掉快捷方式
pub fn set_autostart(on: bool) {
    unsafe {
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(APPROVED_LNK), &HSTRING::from(LNK_NAME));
    }
    if on {
        let _ = write_lnk();
    } else if let Some(p) = startup_lnk() {
        let _ = std::fs::remove_file(p);
    }
}

pub fn shell_open(path: &str) {
    unsafe {
        ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(path), None, None, SW_SHOWNORMAL);
    }
}

/// 图片/视频像素尺寸（读系统属性，只读文件头，几毫秒），按 EXIF/视频旋转方向交换宽高
pub fn media_size(path: &str) -> Option<(f64, f64)> {
    use windows::Win32::Storage::EnhancedStorage::*;
    use windows::Win32::UI::Shell::IShellItem2;
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        let item: IShellItem2 = SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
        let get = |k| item.GetUInt32(&k).ok().filter(|v| *v > 0);
        let (w, h, rot) = match (get(PKEY_Image_HorizontalSize), get(PKEY_Image_VerticalSize)) {
            (Some(w), Some(h)) => (w, h, get(PKEY_Photo_Orientation).map(|o| (5..=8).contains(&o)).unwrap_or(false)),
            _ => (get(PKEY_Video_FrameWidth)?, get(PKEY_Video_FrameHeight)?, get(PKEY_Video_Orientation).map(|o| o == 90 || o == 270).unwrap_or(false)),
        };
        Some(if rot { (h as f64, w as f64) } else { (w as f64, h as f64) })
    }
}

/// 系统缩略图（走资源管理器同一套缩略图提供程序），输出 32 位 BMP 字节
pub fn thumbnail_bmp(path: &str, size: i32) -> Option<Vec<u8>> {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
        let item: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
        let hb = item.GetImage(SIZE { cx: size, cy: size }, SIIGBF_BIGGERSIZEOK).ok()?;
        let out = hbitmap_to_bmp(hb);
        let _ = DeleteObject(hb.into());
        out
    }
}

unsafe fn hbitmap_to_bmp(hb: HBITMAP) -> Option<Vec<u8>> {
    let mut bm = BITMAP::default();
    if GetObjectW(hb.into(), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as _)) == 0 {
        return None;
    }
    let (w, h) = (bm.bmWidth, bm.bmHeight);
    let mut bi = BITMAPINFO::default();
    bi.bmiHeader = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w,
        biHeight: -h,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let mut px = vec![0u8; (w * h * 4) as usize];
    let dc = GetDC(None);
    let n = GetDIBits(dc, hb, 0, h as u32, Some(px.as_mut_ptr() as _), &mut bi, DIB_RGB_COLORS);
    ReleaseDC(None, dc);
    if n == 0 {
        return None;
    }
    // 缩略图常是预乘 alpha 或 alpha 全 0：全 0 时补成不透明
    if px.chunks(4).all(|c| c[3] == 0) {
        for c in px.chunks_mut(4) {
            c[3] = 255;
        }
    }
    // BITMAPV4 头，32 位 BI_BITFIELDS 带 alpha
    let hdr = 14 + 108;
    let mut f = Vec::with_capacity(hdr + px.len());
    f.extend_from_slice(b"BM");
    f.extend_from_slice(&((hdr + px.len()) as u32).to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&(hdr as u32).to_le_bytes());
    f.extend_from_slice(&108u32.to_le_bytes());
    f.extend_from_slice(&w.to_le_bytes());
    f.extend_from_slice(&(-h).to_le_bytes());
    f.extend_from_slice(&1u16.to_le_bytes());
    f.extend_from_slice(&32u16.to_le_bytes());
    f.extend_from_slice(&3u32.to_le_bytes()); // BI_BITFIELDS
    f.extend_from_slice(&(px.len() as u32).to_le_bytes());
    f.extend_from_slice(&[0u8; 8]); // 分辨率
    f.extend_from_slice(&[0u8; 8]); // 调色板
    f.extend_from_slice(&0x00FF0000u32.to_le_bytes());
    f.extend_from_slice(&0x0000FF00u32.to_le_bytes());
    f.extend_from_slice(&0x000000FFu32.to_le_bytes());
    f.extend_from_slice(&0xFF000000u32.to_le_bytes());
    f.extend_from_slice(b"BGRs");
    f.extend_from_slice(&[0u8; 36 + 12]); // 端点 + gamma
    f.extend_from_slice(&px);
    Some(f)
}

// ---------- PDF：用系统自带的 Windows.Data.Pdf 按页渲染成图片 ----------
// WebView2 内置查看器要先读完整个文件才出第一页（80 MB 的教材要 5 秒），这里只解析交叉引用表、只渲染看得到的页。
// 最近打开的一个文档留着句柄，滚动时连续取页不用重新加载。
static PDF: std::sync::Mutex<Option<(String, windows::Data::Pdf::PdfDocument)>> = std::sync::Mutex::new(None);

fn pdf_doc(path: &str) -> windows::core::Result<windows::Data::Pdf::PdfDocument> {
    use windows::Data::Pdf::PdfDocument;
    use windows::Storage::StorageFile;
    // 同路径文件被改写后要重新打开：键里带上大小和修改时间
    let key = std::fs::metadata(path).map(|m| format!("{path}|{}|{:?}", m.len(), m.modified().ok())).unwrap_or_else(|_| path.to_string());
    if let Some((p, d)) = PDF.lock().unwrap().as_ref() {
        if *p == key {
            return Ok(d.clone());
        }
    }
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path))?.join()?;
    let doc = PdfDocument::LoadFromFileAsync(&file)?.join()?;
    *PDF.lock().unwrap() = Some((key, doc.clone()));
    Ok(doc)
}

/// 各页尺寸（pt），JSON：[[w,h],...]；加密或损坏的文件返回 Err，页面退回内置查看器
pub fn pdf_pages(path: &str) -> Result<String, String> {
    let e = |x: windows::core::Error| x.message().to_string();
    let doc = pdf_doc(path).map_err(e)?;
    let n = doc.PageCount().map_err(e)?;
    let mut out = String::from("[");
    for i in 0..n {
        let s = doc.GetPage(i).and_then(|p| p.Size()).map_err(e)?;
        out += &format!("{}[{:.1},{:.1}]", if i > 0 { "," } else { "" }, s.Width, s.Height);
    }
    Ok(out + "]")
}

/// 第 n 页（从 0 起）按宽 w 像素渲染成 JPEG
pub fn pdf_render(path: &str, n: u32, w: u32) -> windows::core::Result<Vec<u8>> {
    use windows::Data::Pdf::PdfPageRenderOptions;
    use windows::Graphics::Imaging::BitmapEncoder;
    use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};
    let page = pdf_doc(path)?.GetPage(n)?;
    let opt = PdfPageRenderOptions::new()?;
    opt.SetDestinationWidth(w.clamp(64, 4096))?;
    opt.SetBitmapEncoderId(BitmapEncoder::JpegEncoderId()?)?;
    let s = InMemoryRandomAccessStream::new()?;
    page.RenderWithOptionsToStreamAsync(&s, &opt)?.join()?;
    let len = s.Size()? as u32;
    let rd = DataReader::CreateDataReader(&s.GetInputStreamAt(0)?)?;
    rd.LoadAsync(len)?.join()?;
    let mut buf = vec![0u8; len as usize];
    rd.ReadBytes(&mut buf)?;
    Ok(buf)
}

/// 用 Windows 自带 OCR 识别图片文字（优先简体中文）
pub fn ocr(path: &str) -> Result<String, String> {
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::{BitmapDecoder, BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::{FileAccessMode, StorageFile};
    let e = |x: windows::core::Error| x.message().to_string();
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path)).map_err(e)?.join().map_err(e)?;
    let stream = file.OpenAsync(FileAccessMode::Read).map_err(e)?.join().map_err(e)?;
    let dec = BitmapDecoder::CreateAsync(&stream).map_err(e)?.join().map_err(e)?;
    let max = OcrEngine::MaxImageDimension().unwrap_or(10000);
    let bmp = if dec.PixelWidth().unwrap_or(0) > max || dec.PixelHeight().unwrap_or(0) > max {
        let t = windows::Graphics::Imaging::BitmapTransform::new().map_err(e)?;
        let (w, h) = (dec.PixelWidth().unwrap_or(1) as f64, dec.PixelHeight().unwrap_or(1) as f64);
        let k = max as f64 / w.max(h);
        t.SetScaledWidth((w * k) as u32).map_err(e)?;
        t.SetScaledHeight((h * k) as u32).map_err(e)?;
        dec.GetSoftwareBitmapTransformedAsync(
            BitmapPixelFormat::Bgra8,
            windows::Graphics::Imaging::BitmapAlphaMode::Premultiplied,
            &t,
            windows::Graphics::Imaging::ExifOrientationMode::RespectExifOrientation,
            windows::Graphics::Imaging::ColorManagementMode::DoNotColorManage,
        )
        .map_err(e)?
        .join()
        .map_err(e)?
    } else {
        let b = dec.GetSoftwareBitmapAsync().map_err(e)?.join().map_err(e)?;
        SoftwareBitmap::Convert(&b, BitmapPixelFormat::Bgra8).map_err(e)?
    };
    let mut eng = None;
    for tag in ["zh-Hans-CN", "zh-CN", "zh-Hans"] {
        if let Ok(l) = Language::CreateLanguage(&HSTRING::from(tag)) {
            if OcrEngine::IsLanguageSupported(&l).unwrap_or(false) {
                eng = OcrEngine::TryCreateFromLanguage(&l).ok();
                if eng.is_some() {
                    break;
                }
            }
        }
    }
    let eng = match eng {
        Some(x) => x,
        None => OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| "系统没有可用的 OCR 语言包（设置 → 时间和语言 → 语言 → 添加中文的“光学字符识别”）".to_string())?,
    };
    let res = eng.RecognizeAsync(&bmp).map_err(e)?.join().map_err(e)?;
    let mut out = String::new();
    for line in res.Lines().map_err(e)? {
        let t = line.Text().map(|s| s.to_string()).unwrap_or_default();
        out.push_str(&squeeze_cjk(&t));
        out.push('\n');
    }
    Ok(out.trim_end().to_string())
}

/// OCR 对中文按字加空格，把两个 CJK 字符之间的空格去掉
fn squeeze_cjk(s: &str) -> String {
    let cjk = |c: char| ('\u{2E80}'..='\u{9FFF}').contains(&c) || ('\u{FF00}'..='\u{FFEF}').contains(&c) || ('\u{3000}'..='\u{303F}').contains(&c);
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::new();
    for (i, &c) in cs.iter().enumerate() {
        if c == ' ' && i > 0 && i + 1 < cs.len() && (cjk(cs[i - 1]) || cjk(cs[i + 1])) {
            continue;
        }
        out.push(c);
    }
    out
}

// ---------- 进程间：第二个实例把路径交给常驻实例 ----------
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
static IPC_TX: OnceLock<Sender<String>> = OnceLock::new();
const IPC_CLASS: &str = "QuickViewIPCWindow";

unsafe extern "system" fn ipc_proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if m == WM_COPYDATA {
        let cds = &*(lp.0 as *const COPYDATASTRUCT);
        let n = cds.cbData as usize / 2;
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(cds.lpData as *const u16, n));
        if let Some(tx) = IPC_TX.get() {
            let _ = tx.send(s);
        }
        return LRESULT(1);
    }
    DefWindowProcW(h, m, wp, lp)
}

pub fn start_ipc(tx: Sender<String>) {
    let _ = IPC_TX.set(tx);
    std::thread::spawn(|| unsafe {
        let cls = HSTRING::from(IPC_CLASS);
        let wc = WNDCLASSW { lpfnWndProc: Some(ipc_proc), lpszClassName: windows::core::PCWSTR(cls.as_ptr()), ..Default::default() };
        RegisterClassW(&wc);
        let _ = CreateWindowExW(Default::default(), &cls, &cls, Default::default(), 0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    });
}

/// 发给已运行实例；成功返回 true
pub fn send_to_running(text: &str) -> bool {
    unsafe {
        let Ok(h) = FindWindowExW(Some(HWND_MESSAGE), None, &HSTRING::from(IPC_CLASS), None) else { return false };
        let w: Vec<u16> = text.encode_utf16().collect();
        let cds = COPYDATASTRUCT { dwData: 1, cbData: (w.len() * 2) as u32, lpData: w.as_ptr() as _ };
        SendMessageW(h, WM_COPYDATA, None, Some(LPARAM(&cds as *const _ as isize)));
        true
    }
}
