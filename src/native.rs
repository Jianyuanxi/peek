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

unsafe extern "system" fn kb_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && (wp.0 as u32 == WM_KEYDOWN) {
        let k = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        if k.vkCode == VK_SPACE.0 as u32
            && !down(VK_CONTROL)
            && !down(VK_MENU)
            && !down(VK_SHIFT)
            && !down(VK_LWIN)
            && !down(VK_RWIN)
        {
            let fg = GetForegroundWindow();
            if is_explorer_list(fg) {
                if let Some(tx) = HOOK_TX.get() {
                    let _ = tx.send(fg.0 as isize);
                }
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
const RUN_NAME: &str = "Peek";

pub fn autostart_enabled() -> bool {
    unsafe {
        let mut sz = 0u32;
        RegGetValueW(HKEY_CURRENT_USER, &HSTRING::from(RUN_KEY), &HSTRING::from(RUN_NAME), RRF_RT_REG_SZ, None, None, Some(&mut sz)).is_ok()
    }
}

pub fn set_autostart(on: bool) {
    unsafe {
        if on {
            let exe = std::env::current_exe().unwrap_or_default();
            let v = format!("\"{}\"", exe.display());
            let w: Vec<u16> = v.encode_utf16().chain(Some(0)).collect();
            let _ = RegSetKeyValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(RUN_KEY),
                &HSTRING::from(RUN_NAME),
                REG_SZ.0,
                Some(w.as_ptr() as _),
                (w.len() * 2) as u32,
            );
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(RUN_KEY), &HSTRING::from(RUN_NAME));
        }
    }
}

pub fn shell_open(path: &str) {
    unsafe {
        ShellExecuteW(None, &HSTRING::from("open"), &HSTRING::from(path), None, None, SW_SHOWNORMAL);
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
const IPC_CLASS: &str = "PeekIPCWindow";

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
