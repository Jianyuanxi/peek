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

pub fn basic(path: &str, index: usize, total: usize) -> Value {
    let p = Path::new(path);
    let name = p.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
    let ext = p.extension().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    let md = std::fs::metadata(path).ok();
    let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
    let size = md.as_ref().filter(|m| !m.is_dir()).map(|m| m.len());
    let attrs = md.as_ref().map(|m| m.file_attributes()).unwrap_or(0);
    json!({
        "path": path,
        "name": name,
        "ext": ext,
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
