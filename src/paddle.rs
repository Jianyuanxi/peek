//! 图片 OCR：PaddleOCR PP-OCRv6 tiny（检测 + 识别两个 ONNX 模型，约 6 MB），用系统自带的 Windows ML 推理，
//! 不打包 onnxruntime。模型每次识别时加载（约 50 ms），用完即释放，常驻内存不增加。
//! 后处理是简化版 DB：阈值二值化 → 连通域 → 外接矩形外扩，只处理横排文字（截图、文档足够；竖排/旋转文字会认不好）。
use windows::core::{Interface, HSTRING};
use windows::AI::MachineLearning::{LearningModel, LearningModelBinding, LearningModelDevice, LearningModelDeviceKind, LearningModelSession, TensorFloat};

type R<T> = Result<T, String>;
fn e(x: windows::core::Error) -> String {
    x.message().to_string()
}

/// 模型目录：安装目录下的 ocr\，开发时退回仓库里的 assets\ocr
fn model_dir() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let a = exe.parent()?.join("ocr");
    if a.join("rec.onnx").exists() {
        return Some(a);
    }
    let b = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join("ocr");
    b.join("rec.onnx").exists().then_some(b)
}

pub fn available() -> bool {
    model_dir().is_some()
}

/// 解码图片为 RGB（透明部分垫白底），返回 (宽, 高, 像素)
fn decode(path: &str) -> R<(usize, usize, Vec<u8>)> {
    use windows::Graphics::Imaging::*;
    use windows::Storage::{FileAccessMode, StorageFile};
    let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path)).map_err(e)?.join().map_err(e)?;
    let stream = file.OpenAsync(FileAccessMode::Read).map_err(e)?.join().map_err(e)?;
    let dec = BitmapDecoder::CreateAsync(&stream).map_err(e)?.join().map_err(e)?;
    let (w, h) = (dec.OrientedPixelWidth().map_err(e)? as usize, dec.OrientedPixelHeight().map_err(e)? as usize);
    let px = dec
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Straight,
            &BitmapTransform::new().map_err(e)?,
            ExifOrientationMode::RespectExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )
        .map_err(e)?
        .join()
        .map_err(e)?
        .DetachPixelData()
        .map_err(e)?;
    let mut rgb = vec![0u8; w * h * 3];
    for (i, p) in px.chunks_exact(4).enumerate().take(w * h) {
        let a = p[3] as u32;
        for c in 0..3 {
            rgb[i * 3 + c] = ((p[2 - c] as u32 * a + 255 * (255 - a)) / 255) as u8;
        }
    }
    Ok((w, h, rgb))
}

/// 把 rgb 图里 (x0,y0,cw,ch) 区域双线性缩放到 dw×dh，按 BGR 顺序写成 CHW 张量；norm(通道, 0~1 的值)
fn to_tensor(rgb: &[u8], w: usize, rect: (f32, f32, f32, f32), dw: usize, dh: usize, norm: impl Fn(usize, f32) -> f32) -> Vec<f32> {
    let (x0, y0, cw, ch) = rect;
    let h = rgb.len() / 3 / w;
    let mut t = vec![0f32; 3 * dw * dh];
    let (sx, sy) = (cw / dw as f32, ch / dh as f32);
    for y in 0..dh {
        let fy = (y0 + (y as f32 + 0.5) * sy - 0.5).clamp(0.0, (h - 1) as f32);
        let (y1, wy) = (fy as usize, fy.fract());
        let y2 = (y1 + 1).min(h - 1);
        for x in 0..dw {
            let fx = (x0 + (x as f32 + 0.5) * sx - 0.5).clamp(0.0, (w - 1) as f32);
            let (x1, wx) = (fx as usize, fx.fract());
            let x2 = (x1 + 1).min(w - 1);
            for c in 0..3 {
                let g = |xx: usize, yy: usize| rgb[(yy * w + xx) * 3 + c] as f32;
                let v = (g(x1, y1) * (1.0 - wx) + g(x2, y1) * wx) * (1.0 - wy) + (g(x1, y2) * (1.0 - wx) + g(x2, y2) * wx) * wy;
                // BGR：第 0 个通道放蓝
                t[(2 - c) * dw * dh + y * dw + x] = norm(2 - c, v / 255.0);
            }
        }
    }
    t
}

fn run(s: &LearningModelSession, shape: &[i64], data: &[f32]) -> R<(Vec<i64>, Vec<f32>)> {
    let b = LearningModelBinding::CreateFromSession(s).map_err(e)?;
    b.Bind(&HSTRING::from("x"), &TensorFloat::CreateFromShapeArrayAndDataArray(shape, data).map_err(e)?).map_err(e)?;
    let r = s.Evaluate(&b, &HSTRING::new()).map_err(e)?;
    let outs = r.Outputs().map_err(e)?;
    let first = outs.First().map_err(e)?.Current().map_err(e)?;
    let t: TensorFloat = first.Value().map_err(e)?.cast().map_err(e)?;
    let shape: Vec<i64> = t.Shape().map_err(e)?.into_iter().collect();
    let v = t.GetAsVectorView().map_err(e)?;
    let mut out = vec![0f32; v.Size().map_err(e)? as usize];
    v.GetMany(0, &mut out).map_err(e)?;
    Ok((shape, out))
}

/// 检测文字框，返回原图坐标 (x0, y0, x1, y1)
fn detect(s: &LearningModelSession, rgb: &[u8], w: usize, h: usize) -> R<Vec<(f32, f32, f32, f32)>> {
    // 超过 150 万像素才缩小，边长取 32 的倍数。检测的内存峰值随像素数涨（1440×810 原尺寸约 200 MB），
    // 实测缩到 60 万像素时结果和原尺寸仍有 98.8% 一致，150 万对 1080p 截图只缩到约 0.85 倍
    let k = (1.5e6 / (w * h) as f32).sqrt().min(1.0);
    let r32 = |v: f32| ((v / 32.0).round() as usize).max(1) * 32;
    let (dw, dh) = (r32(w as f32 * k), r32(h as f32 * k));
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.229, 0.224, 0.225];
    let t = to_tensor(rgb, w, (0.0, 0.0, w as f32, h as f32), dw, dh, |c, v| (v - MEAN[c]) / STD[c]);
    let (_, p) = run(s, &[1, 3, dh as i64, dw as i64], &t)?;
    // 连通域（4 邻接）
    let mut lab = vec![0u32; dw * dh];
    let mut boxes = Vec::new();
    let mut stack = Vec::new();
    let (sx, sy) = (w as f32 / dw as f32, h as f32 / dh as f32);
    for i in 0..dw * dh {
        if lab[i] != 0 || p[i] <= 0.3 {
            continue;
        }
        let id = boxes.len() as u32 + 1;
        let (mut x0, mut y0, mut x1, mut y1, mut sum, mut n) = (dw, dh, 0, 0, 0f32, 0usize);
        lab[i] = id;
        stack.push(i);
        while let Some(j) = stack.pop() {
            let (x, y) = (j % dw, j / dw);
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
            sum += p[j];
            n += 1;
            let mut push = |k: usize| {
                if lab[k] == 0 && p[k] > 0.3 {
                    lab[k] = id;
                    stack.push(k);
                }
            };
            if x > 0 { push(j - 1) }
            if x + 1 < dw { push(j + 1) }
            if y > 0 { push(j - dw) }
            if y + 1 < dh { push(j + dw) }
        }
        let (bw, bh) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);
        // 平均置信度太低或太小的丢掉；按 DB 的 unclip（比例 1.5）外扩
        let keep = sum / n as f32 >= 0.6 && bw.min(bh) >= 3.0;
        let d = bw * bh * 1.5 / (2.0 * (bw + bh));
        boxes.push(keep.then(|| {
            (
                ((x0 as f32 - d) * sx).max(0.0),
                ((y0 as f32 - d) * sy).max(0.0),
                ((x1 as f32 + 1.0 + d) * sx).min(w as f32),
                ((y1 as f32 + 1.0 + d) * sy).min(h as f32),
            )
        }));
    }
    Ok(boxes.into_iter().flatten().collect())
}

/// 识别所有框。按宽高比排序后分批推理（同批补零到同宽，和 PaddleOCR 一致），比逐框调用快；
/// 每批输出是 批数×步数×字表大小 的浮点数，按总宽度限制批大小，免得长行一批占几十 MB
fn recognize(s: &LearningModelSession, chars: &[String], rgb: &[u8], w: usize, boxes: &[(f32, f32, f32, f32)]) -> R<Vec<String>> {
    let width = |b: &(f32, f32, f32, f32)| ((48.0 * (b.2 - b.0) / (b.3 - b.1).max(1.0)).round() as usize).clamp(16, 3200);
    let mut order: Vec<usize> = (0..boxes.len()).collect();
    order.sort_by_key(|&i| width(&boxes[i]));
    let mut out = vec![String::new(); boxes.len()];
    let mut i = 0;
    while i < order.len() {
        // 已按宽度升序，批里最后一个最宽
        let mut j = i + 1;
        while j < order.len() && j - i < 16 && width(&boxes[order[j]]) * (j - i + 1) <= 1600 {
            j += 1;
        }
        let dw = width(&boxes[order[j - 1]]);
        let n = j - i;
        let mut t = vec![0f32; n * 3 * 48 * dw];
        for (k, &bi) in order[i..j].iter().enumerate() {
            let b = boxes[bi];
            let bw = width(&b);
            let one = to_tensor(rgb, w, (b.0, b.1, b.2 - b.0, b.3 - b.1), bw, 48, |_, v| (v - 0.5) / 0.5);
            for c in 0..3 {
                for y in 0..48 {
                    let src = &one[(c * 48 + y) * bw..(c * 48 + y + 1) * bw];
                    let d = ((k * 3 + c) * 48 + y) * dw;
                    t[d..d + bw].copy_from_slice(src);
                }
            }
        }
        let (shape, p) = run(s, &[n as i64, 3, 48, dw as i64], &t)?;
        let (steps, nc) = (shape[1] as usize, shape[2] as usize);
        for (k, &bi) in order[i..j].iter().enumerate() {
            let mut last = 0;
            for st in 0..steps {
                let row = &p[(k * steps + st) * nc..(k * steps + st + 1) * nc];
                let m = (0..nc).fold(0, |m, x| if row[x] > row[m] { x } else { m });
                if m != 0 && m != last {
                    if let Some(c) = chars.get(m) {
                        out[bi].push_str(c);
                    }
                }
                last = m;
            }
        }
        i = j;
    }
    Ok(out)
}

pub fn ocr(path: &str) -> R<String> {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
    let dir = model_dir().ok_or("找不到 OCR 模型")?;
    let (w, h, rgb) = decode(path)?;
    let load = |p: std::path::PathBuf| -> R<(LearningModel, LearningModelSession)> {
        let dev = LearningModelDevice::Create(LearningModelDeviceKind::Cpu).map_err(e)?;
        let m = LearningModel::LoadFromFilePath(&HSTRING::from(p.as_os_str())).map_err(e)?;
        let s = LearningModelSession::CreateFromModelOnDevice(&m, &dev).map_err(e)?;
        Ok((m, s))
    };
    // 识别模型在另一个线程里同时加载，省掉一次加载时间
    let rp = dir.join("rec.onnx");
    let rec_th = std::thread::spawn(move || {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
        }
        load(rp)
    });
    let (_, det) = load(dir.join("det.onnx"))?;
    let boxes = detect(&det, &rgb, w, h)?;
    drop(det);
    let (rm, rec) = rec_th.join().map_err(|_| "加载识别模型失败")??;
    if boxes.is_empty() {
        return Ok(String::new());
    }
    // 字表在模型元数据 character 里，一行一个；下标 0 是 CTC 空白，末尾补空格
    let meta = rm.Metadata().map_err(e)?.Lookup(&HSTRING::from("character")).map_err(e)?.to_string();
    let mut chars = vec![String::new()];
    chars.extend(meta.split('\n').map(str::to_string));
    chars.push(" ".into());

    // 按行归组：和某行第一个框竖直方向重叠超过 60%（按较矮的算）就并进那一行，行内按 x 排序。
    // 只和行首框比，避免表格里上下错开半行的各列被一路串成一行
    let mut bs = boxes;
    bs.sort_by(|a, b| (a.1 + a.3).total_cmp(&(b.1 + b.3)));
    let mut lines: Vec<Vec<(f32, f32, f32, f32)>> = Vec::new();
    for b in bs {
        let same = |r: &(f32, f32, f32, f32)| (b.3.min(r.3) - b.1.max(r.1)) >= 0.6 * (b.3 - b.1).min(r.3 - r.1);
        match lines.iter_mut().rev().take(8).find(|l| same(&l[0])) {
            Some(l) => l.push(b),
            None => lines.push(vec![b]),
        }
    }
    lines.sort_by(|a, b| (a[0].1 + a[0].3).total_cmp(&(b[0].1 + b[0].3)));
    for l in lines.iter_mut() {
        l.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    let flat: Vec<_> = lines.iter().flatten().copied().collect();
    let mut texts = recognize(&rec, &chars, &rgb, w, &flat)?.into_iter();
    let mut out = String::new();
    for l in lines {
        let mut prev: Option<(f32, f32, f32, f32)> = None;
        for b in l {
            let t = texts.next().unwrap_or_default();
            if t.is_empty() {
                continue;
            }
            // 同一行相隔较远的框（如表格不同列）之间加空格
            if let Some(p) = prev {
                if b.0 - p.2 > (b.3 - b.1) * 0.5 {
                    out.push_str("  ");
                }
            }
            out.push_str(t.trim());
            prev = Some(b);
        }
        if prev.is_some() {
            out.push('\n');
        }
    }
    Ok(out.trim_end().to_string())
}
