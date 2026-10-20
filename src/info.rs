// 文件信息：大小、时间、哈希、旧版 Office 转 PDF
use serde_json::{json, Value};
use std::os::windows::fs::MetadataExt;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;
use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};

/// 1024 进制，自动选单位，保留两位小数；小于 1 KB 显示字节
pub fn fmt_size(b: u64) -> String {
    const U: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    if b < 1024 {
        return format!("{} B", b);
    }
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{:.2} {}", v, U[i])
}

fn fmt_time(ft: u64) -> String {
    if ft == 0 {
        return String::new();
    }
    unsafe {
        let f = FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft >> 32) as u32 };
        let mut utc = SYSTEMTIME::default();
        let mut loc = SYSTEMTIME::default();
        if FileTimeToSystemTime(&f, &mut utc).is_err() || SystemTimeToTzSpecificLocalTime(None, &utc, &mut loc).is_err() {
            return String::new();
        }
        format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", loc.wYear, loc.wMonth, loc.wDay, loc.wHour, loc.wMinute, loc.wSecond)
    }
}

/// 把本地路径映射成 WebView2 虚拟主机 URL：C:\a b\c.txt -> http://c.peekfs/a%20b/c.txt
pub fn file_url(path: &str) -> String {
    let p = path.replace('\\', "/");
    let (drive, rest) = match p.split_once(":/") {
        Some((d, r)) => (d.to_ascii_lowercase(), r.to_string()),
        None => return String::new(),
    };
    let mut enc = String::new();
    for b in rest.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => enc.push(b as char),
            _ => enc.push_str(&format!("%{:02X}", b)),
        }
    }
    format!("http://{}.peekfs/{}", drive, enc)
}

/// 扩展名 → 预览类型（ui/app.js 按这个 kind 分派渲染器）
const KINDS: &[(&str, &str)] = &[
    ("image", "jpg jpeg jfif pjpeg png apng gif webp bmp ico cur svg avif"),
    ("thumbimg", "heic heif tif tiff psd raw cr2 cr3 nef arw dng orf rw2 raf jxr wdp dds tga exr hdr"),
    ("video", "mp4 m4v webm mov mkv ogv 3gp ts m2ts"),
    ("audio", "mp3 wav flac ogg oga opus m4a aac weba"),
    ("pdf", "pdf"),
    ("md", "md markdown mdown mkd"),
    ("html", "html htm xhtml mht"),
    ("csv", "csv tsv"),
    ("docx", "docx docm dotx dotm"),
    ("sheet", "xlsx xlsm xlsb xls xltx ods et ett numbers"),
    ("pptx", "pptx pptm ppsx potx"),
    ("legacy", "doc dot wps wpt rtf odt ott ppt pps pot dps dpt odp odg vsd"),
    ("zip", "zip jar apk xpi vsix nupkg epub cbz whl aar ipa"),
    ("font", "ttf otf woff woff2"),
];

fn kind_of(path: &str, name: &str, ext: &str, is_dir: bool) -> &'static str {
    if is_dir {
        return "folder";
    }
    if let Some((k, _)) = KINDS.iter().find(|(_, list)| list.split(' ').any(|e| e == ext)) {
        return k;
    }
    let n = name.to_ascii_lowercase();
    if ["readme", "license", "makefile", "dockerfile", "changelog"].iter().any(|x| n == *x) {
        return "text";
    }
    // 其余按内容判断：前 8 KB 没有 0 字节（或有 UTF-16 BOM）就当文本
    if looks_text(path) {
        "text"
    } else {
        "other"
    }
}

fn looks_text(path: &str) -> bool {
    use std::io::Read;
    let mut buf = [0u8; 8192];
    let Ok(n) = std::fs::File::open(path).and_then(|mut f| f.read(&mut buf)) else { return false };
    let b = &buf[..n];
    b.starts_with(&[0xFF, 0xFE]) || b.starts_with(&[0xFE, 0xFF]) || !b.contains(&0)
}

/// PDF 第一页尺寸（pt）：在文件前 1 MB 里找第一个 /MediaBox [x0 y0 x1 y1]
fn pdf_page_size(path: &str) -> Option<(f64, f64)> {
    use std::io::Read;
    let mut buf = Vec::new();
    std::fs::File::open(path).ok()?.take(1 << 20).read_to_end(&mut buf).ok()?;
    let i = buf.windows(9).position(|w| w == b"/MediaBox")?;
    let tail = String::from_utf8_lossy(&buf[i + 9..(i + 120).min(buf.len())]).to_string();
    let inner = tail.split_once('[')?.1.split_once(']')?.0.to_string();
    let v: Vec<f64> = inner.split_whitespace().filter_map(|x| x.parse().ok()).collect();
    let (w, h) = ((v.get(2)? - v.first()?).abs(), (v.get(3)? - v.get(1)?).abs());
    (w > 1.0 && h > 1.0).then_some((w, h))
}

/// 窗口内容区想要的大小（逻辑像素）和最多占工作区的比例，照原版 QuickLook 各插件的 PreferredSize。
/// 在显示内容之前先把窗口调好，不会先大后小闪一下。sized=false 表示没读到真实尺寸，页面加载后再校正
pub fn preferred_size(path: &str, kind: &str) -> (f64, f64, f64, bool) {
    match kind {
        "image" | "thumbimg" => match crate::native::media_size(path) {
            Some((w, h)) => (w, h, 0.8, true),
            None => (800.0, 600.0, 0.8, false),
        },
        "video" => match crate::native::media_size(path) {
            Some((w, h)) => (w.max(100.0), h.max(100.0), 0.8, true),
            None => (1366.0, 768.0, 0.8, false),
        },
        "audio" => (500.0, 300.0, 0.8, true),
        // PDF 按第一页比例，尽量大（占工作区 90%）
        "pdf" => {
            let (w, h) = pdf_page_size(path).unwrap_or((595.0, 842.0));
            (w * 3.0, h * 3.0, 0.9, true)
        }
        "docx" | "sheet" | "pptx" | "legacy" => (1200.0, 800.0, 0.8, true),
        "md" | "html" => (1000.0, 600.0, 0.9, true),
        "csv" => (1000.0, 700.0, 0.9, true),
        "zip" => (800.0, 400.0, 0.9, true),
        "text" | "font" => (800.0, 600.0, 0.9, true),
        _ => (453.0, 172.0, 0.9, true),
    }
}

pub fn basic(path: &str, index: usize, total: usize) -> Value {
    let p = Path::new(path);
    let name = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
    let ext = p.extension().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    let md = std::fs::metadata(path).ok();
    let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
    let size = md.as_ref().filter(|m| !m.is_dir()).map(|m| m.len());
    let attrs = md.as_ref().map(|m| m.file_attributes()).unwrap_or(0);
    let kind = if md.is_some() { kind_of(path, &name, &ext, is_dir) } else { "other" };
    let (pw, ph, pr, sized) = preferred_size(path, kind);
    json!({
        "path": path,
        "name": name,
        "ext": ext,
        "kind": kind,
        "pw": pw, "ph": ph, "pr": pr, "sized": sized,
        "isDir": is_dir,
        "exists": md.is_some(),
        "size": size,
        "sizeText": size.map(fmt_size),
        "created": md.as_ref().map(|m| fmt_time(m.creation_time())).unwrap_or_default(),
        "modified": md.as_ref().map(|m| fmt_time(m.last_write_time())).unwrap_or_default(),
        "accessed": md.as_ref().map(|m| fmt_time(m.last_access_time())).unwrap_or_default(),
        "hidden": attrs & 2 != 0,
        "readonly": attrs & 1 != 0,
        "url": file_url(path),
        "index": index,
        "total": total,
    })
}

/// MD5 + SHA256，流式读取
pub fn hashes(path: &str) -> Value {
    use md5::Digest as _;
    use std::io::Read;
    let Ok(mut f) = std::fs::File::open(path) else { return json!({"error": "无法打开文件"}) };
    let mut m = md5::Md5::new();
    let mut s = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                m.update(&buf[..n]);
                sha2::Digest::update(&mut s, &buf[..n]);
            }
            Err(_) => return json!({"error": "读取失败"}),
        }
    }
    let hex = |b: &[u8]| b.iter().map(|x| format!("{:02x}", x)).collect::<String>();
    json!({"md5": hex(&m.finalize()), "sha256": hex(&sha2::Digest::finalize(s))})
}

/// 旧版 Office（doc/ppt/wps/rtf…）用 LibreOffice 转 PDF，按 路径+大小+修改时间 缓存
pub fn convert_pdf(path: &str) -> Result<String, String> {
    use std::hash::{Hash, Hasher};
    let md = std::fs::metadata(path).map_err(|e| e.to_string())?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (path, md.len(), md.last_write_time()).hash(&mut h);
    let base = std::path::PathBuf::from(std::env::var("LOCALAPPDATA").unwrap_or_default()).join("Peek");
    let cache = base.join("cache");
    let _ = std::fs::create_dir_all(&cache);
    let out = cache.join(format!("{:016x}.pdf", h.finish()));
    if out.exists() {
        return Ok(out.to_string_lossy().to_string());
    }
    let soffice = [r"C:\Program Files\LibreOffice\program\soffice.exe", r"C:\Program Files (x86)\LibreOffice\program\soffice.exe"]
        .iter()
        .find(|p| Path::new(p).exists())
        .ok_or("没有安装 LibreOffice，旧版 Office 格式只能显示缩略图")?;
    let tmp = cache.join(format!("tmp{:016x}", h.finish()));
    let _ = std::fs::create_dir_all(&tmp);
    // 独立的 LibreOffice 配置目录，不和用户正在用的 LibreOffice 冲突
    let profile = format!("-env:UserInstallation=file:///{}", base.join("lo-profile").to_string_lossy().replace('\\', "/"));
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let st = Command::new(soffice)
        .args([&profile, "--headless", "--norestore", "--nologo", "--convert-to", "pdf", "--outdir"])
        .arg(&tmp)
        .arg(path)
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| e.to_string())?;
    let produced = std::fs::read_dir(&tmp).ok().and_then(|mut d| d.find_map(|e| e.ok().map(|e| e.path()).filter(|p| p.extension().map(|x| x == "pdf").unwrap_or(false))));
    let r = match produced {
        Some(p) => std::fs::rename(&p, &out).map(|_| out.to_string_lossy().to_string()).map_err(|e| e.to_string()),
        None => Err(format!("转换失败（LibreOffice 退出码 {:?}）", st.code())),
    };
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn size_units() {
        assert_eq!(fmt_size(512), "512 B");
        assert_eq!(fmt_size(1024), "1.00 KB");
        assert_eq!(fmt_size(1536), "1.50 KB");
        assert_eq!(fmt_size(1 << 30), "1.00 GB");
        assert_eq!(fmt_size(2_500_000_000), "2.33 GB");
    }
    #[test]
    fn url_encode() {
        assert_eq!(file_url("C:\\a b\\中.txt"), "http://c.peekfs/a%20b/%E4%B8%AD.txt");
    }
}
