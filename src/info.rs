// 文件信息：大小、时间、文件夹统计（文件夹大小走 Everything 的 es.exe）
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

fn es(args: &[&str]) -> Option<String> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = Command::new("es.exe").args(args).creation_flags(CREATE_NO_WINDOW).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

fn es_json(args: &[&str]) -> Vec<Value> {
    es(args).and_then(|s| serde_json::from_str::<Vec<Value>>(s.trim()).ok()).unwrap_or_default()
}

/// 文件夹统计：总大小、文件/子文件夹数、最大的子项。全部来自 Everything 索引
pub fn folder_stats(path: &str) -> Value {
    let dir = path.trim_end_matches('\\').to_string();
    let p = Path::new(&dir);
    let mut size: Option<u64> = None;
    if let Some(parent) = p.parent().filter(|_| p.file_name().is_some()) {
        let mut parent_s = parent.to_string_lossy().to_string();
        if !parent_s.ends_with('\\') {
            parent_s.push('\\');
        }
        let want = format!("{}\\", dir).to_lowercase();
        for v in es_json(&["-json", "-size", "-parent", &parent_s, "folder:"]) {
            if v["filename"].as_str().map(|f| f.to_lowercase() == want).unwrap_or(false) {
                size = v["size"].as_u64();
            }
        }
    } else {
        // 盘符根目录：用磁盘已用空间
        size = disk_used(&format!("{}\\", dir));
    }
    let count = |kind: &str| es(&["-get-result-count", "-path", &format!("{}\\", dir), kind]).and_then(|s| s.trim().parse::<u64>().ok());
    let files = count("file:");
    let folders = count("folder:");
    let pdir = format!("{}\\", dir);
    let mut kids: Vec<Value> = es_json(&["-json", "-size", "-n", "30", "-sort", "size-descending", "-parent", &pdir, "folder:"]);
    kids.extend(es_json(&["-json", "-size", "-n", "30", "-sort", "size-descending", "-parent", &pdir, "file:"]));
    kids.sort_by_key(|v| std::cmp::Reverse(v["size"].as_u64().unwrap_or(0)));
    kids.truncate(30);
    let kids: Vec<Value> = kids
        .into_iter()
        .map(|v| {
            let f = v["filename"].as_str().unwrap_or("").to_string();
            let is_dir = f.ends_with('\\');
            let name = f.trim_end_matches('\\').rsplit('\\').next().unwrap_or("").to_string();
            let s = v["size"].as_u64().unwrap_or(0);
            json!({"name": name, "dir": is_dir, "size": s, "sizeText": fmt_size(s)})
        })
        .collect();
    json!({
        "path": path,
        "size": size,
        "sizeText": size.map(fmt_size),
        "files": files,
        "folders": folders,
        "children": kids,
        "everything": files.is_some(),
    })
}

fn disk_used(root: &str) -> Option<u64> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut free = 0u64;
    let mut total = 0u64;
    unsafe { GetDiskFreeSpaceExW(&HSTRING::from(root), None, Some(&mut total), Some(&mut free)).ok()? };
    Some(total - free)
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
