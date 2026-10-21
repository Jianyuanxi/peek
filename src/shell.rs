// 资源管理器 / 桌面：取当前选中项、视图内全部项（按显示顺序）、同步选中
use windows::core::{Interface, PWSTR};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, IDispatch, IServiceProvider, CLSCTX_ALL};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::*;
use windows::core::w;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, GetClassNameW, IsWindowVisible};

pub fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub struct View {
    view: IShellView,
    fv: IFolderView2,
}

fn browser_of(disp: &IDispatch) -> Option<IShellBrowser> {
    let sp: IServiceProvider = disp.cast().ok()?;
    unsafe { sp.QueryService::<IShellBrowser>(&SID_STopLevelBrowser).ok() }
}

fn active_browser(fg: HWND) -> Option<IShellBrowser> {
    unsafe {
        let sw: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).ok()?;
        let cls = class_name(fg);
        if cls == "Progman" || cls == "WorkerW" {
            let mut h = 0i32;
            let loc = VARIANT::from(CSIDL_DESKTOP as i32);
            let disp = sw.FindWindowSW(&loc, &VARIANT::default(), SWC_DESKTOP, &mut h, SWFO_NEEDDISPATCH).ok()?;
            return browser_of(&disp);
        }
        // Win11 多标签：一个顶层窗口下每个标签一个 ShellTabWindowClass 子窗口，当前标签在 Z 序最上面，
        // FindWindowEx 返回的第一个就是它（隐藏的标签 IsWindowVisible 也是真，不能靠它判断）
        let active = FindWindowExW(Some(fg), None, w!("ShellTabWindowClass"), None).ok();
        let n = sw.Count().ok()?;
        let mut fallback = None;
        for i in 0..n {
            let Ok(disp) = sw.Item(&VARIANT::from(i)) else { continue };
            let Ok(wb) = disp.cast::<IWebBrowser2>() else { continue };
            let Ok(h) = wb.HWND() else { continue };
            if h.0 as isize != fg.0 as isize {
                continue;
            }
            let Some(sb) = browser_of(&disp) else { continue };
            if let Ok(tab) = sb.GetWindow() {
                match active {
                    Some(a) if a == tab => return Some(sb),
                    None if IsWindowVisible(tab).as_bool() => return Some(sb),
                    _ => {}
                }
            }
            fallback.get_or_insert(sb);
        }
        fallback
    }
}

fn item_path(it: &IShellItem) -> Option<String> {
    unsafe {
        let p: PWSTR = it.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        s
    }
}

fn items(fv: &IFolderView2, flags: i32) -> Vec<String> {
    unsafe {
        let Ok(arr) = fv.Items::<IShellItemArray>(_SVGIO(flags)) else { return vec![] };
        let n = arr.GetCount().unwrap_or(0);
        (0..n).filter_map(|i| arr.GetItemAt(i).ok().and_then(|it| item_path(&it))).collect()
    }
}

/// 返回 (选中的路径, 视图对象)
pub fn selection(fg: HWND) -> Option<(Vec<String>, View)> {
    unsafe {
        let sb = active_browser(fg)?;
        let view = sb.QueryActiveShellView().ok()?;
        let fv: IFolderView2 = view.cast().ok()?;
        let sel = items(&fv, SVGIO_SELECTION.0 | SVGIO_FLAG_VIEWORDER.0);
        if sel.is_empty() {
            return None;
        }
        Some((sel, View { view, fv }))
    }
}

impl View {
    /// 视图内全部文件系统项，按当前排序
    pub fn all(&self) -> Vec<String> {
        items(&self.fv, SVGIO_ALLVIEW.0 | SVGIO_FLAG_VIEWORDER.0)
    }
}

pub fn select_in_view(v: &View, path: &str) {
    unsafe {
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        if SHParseDisplayName(&windows::core::HSTRING::from(path), None, &mut pidl, 0, None).is_err() || pidl.is_null() {
            return;
        }
        let child = ILFindLastID(pidl);
        let flags = SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0;
        let _ = v.view.SelectItem(child, flags as u32);
        CoTaskMemFree(Some(pidl as _));
    }
}

