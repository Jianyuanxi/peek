'use strict';
// pptx-preview 的 UMD 包引用了这几个打包残留的全局名，先占位（原来在 index.html 内联，CSP 不允许内联脚本）
for (const k of ['require$$0', 'require$$2', 'require$$0$1', 'require$$1']) window[k] = window[k] || {};
const $ = s => document.querySelector(s);
const post = (cmd, extra) => window.ipc.postMessage(JSON.stringify(Object.assign({ cmd }, extra || {})));
const esc = s => String(s ?? '').replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
const fmtSize = b => {
  if (b == null) return '';
  const u = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  if (b < 1024) return b + ' B';
  let i = 0, v = b;
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return v.toFixed(2) + ' ' + u[i];
};
const fmtDur = s => {
  if (!isFinite(s)) return '';
  s = Math.round(s);
  const h = Math.floor(s / 3600), m = Math.floor(s % 3600 / 60), x = s % 60;
  return (h ? h + ':' + String(m).padStart(2, '0') : m) + ':' + String(x).padStart(2, '0');
};

// 预览类型（info.kind）和窗口初始大小由 Rust 端 info::basic 按扩展名/文件头决定
const HL_LANG = { rs: 'rust', py: 'python', js: 'javascript', mjs: 'javascript', cjs: 'javascript', ts: 'typescript', tsx: 'typescript', jsx: 'javascript',
  cs: 'csharp', kt: 'kotlin', h: 'cpp', hpp: 'cpp', cc: 'cpp', cxx: 'cpp', c: 'c', sh: 'bash', zsh: 'bash', ps1: 'powershell', psm1: 'powershell',
  bat: 'dos', cmd: 'dos', yml: 'yaml', toml: 'ini', cfg: 'ini', conf: 'ini', vue: 'xml', html: 'xml', xaml: 'xml', csproj: 'xml', svg: 'xml',
  md: 'markdown', ets: 'typescript', rb: 'ruby', pl: 'perl', m: 'objectivec', jsonc: 'json', json5: 'json', ipynb: 'json', gradle: 'groovy', tf: 'ini' };

let cur = null, curFont = null, token = 0, meta = {}, pinned = false;
const view = $('#view');

const loaded = {};
function lib(src) {
  return loaded[src] || (loaded[src] = new Promise((ok, bad) => {
    const s = document.createElement('script');
    s.src = 'vendor/' + src; s.onload = ok; s.onerror = bad;
    document.head.appendChild(s);
  }));
}

function css(href) {
  return loaded[href] || (loaded[href] = new Promise(ok => {
    const l = document.createElement('link');
    l.rel = 'stylesheet'; l.href = 'vendor/' + href; l.onload = l.onerror = ok;
    document.head.appendChild(l);
  }));
}

function toast(t) {
  const el = $('#toast'); el.textContent = t; el.hidden = false;
  clearTimeout(toast.t); toast.t = setTimeout(() => el.hidden = true, 1400);
}

function setMeta(k, v) { if (v === '' || v == null) delete meta[k]; else meta[k] = v; renderProps(); }

function renderProps() {
  if (!cur || $('#side').hidden) return;
  const rows = [['名称', cur.name], ['位置', cur.path], ['大小', cur.sizeText],
    ['类型', cur.isDir ? '文件夹' : (cur.ext ? cur.ext.toUpperCase() + ' 文件' : '文件')], ...Object.entries(meta),
    ['创建时间', cur.created], ['修改时间', cur.modified], ['访问时间', cur.accessed],
    ['属性', [cur.readonly && '只读', cur.hidden && '隐藏'].filter(Boolean).join('、') || '—']];
  $('#props').innerHTML = rows.filter(r => r[1] != null && r[1] !== '').map(r => `<tr><td>${esc(r[0])}</td><td>${esc(r[1])}</td></tr>`).join('');
  $('#hashbox').hidden = cur.isDir;
}

// ---------- 读取 ----------
async function readHead(url, max) {
  const r = await fetch(url);
  if (!r.ok) throw new Error('HTTP ' + r.status);
  const rd = r.body.getReader(); const chunks = []; let n = 0, done = false;
  while (n < max) { const x = await rd.read(); if (x.done) { done = true; break; } chunks.push(x.value); n += x.value.length; }
  if (!done) rd.cancel();
  const buf = new Uint8Array(Math.min(n, max)); let o = 0;
  for (const c of chunks) { const take = Math.min(c.length, buf.length - o); buf.set(c.subarray(0, take), o); o += take; if (o >= buf.length) break; }
  return { buf, truncated: !done || n > max };
}
function decode(buf) {
  if (buf[0] === 0xFF && buf[1] === 0xFE) return [new TextDecoder('utf-16le').decode(buf), 'UTF-16 LE'];
  if (buf[0] === 0xFE && buf[1] === 0xFF) return [new TextDecoder('utf-16be').decode(buf), 'UTF-16 BE'];
  try { return [new TextDecoder('utf-8', { fatal: true }).decode(buf), buf[0] === 0xEF ? 'UTF-8 BOM' : 'UTF-8']; }
  catch (e) {
    // 截断可能切断多字节字符：去掉末尾几个字节再试一次
    for (let k = 1; k <= 3; k++) {
      try { return [new TextDecoder('utf-8', { fatal: true }).decode(buf.subarray(0, buf.length - k)), 'UTF-8']; } catch (e2) {}
    }
    return [new TextDecoder('gbk').decode(buf), 'GBK'];
  }
}
const buffer = url => fetch(url).then(r => { if (!r.ok) throw new Error('HTTP ' + r.status); return r.arrayBuffer(); });
const thumbUrl = (p, s) => `http://peek.localhost/thumb?p=${encodeURIComponent(p)}&s=${s || 1280}&t=${Date.now()}`;

// 文档内嵌的 EMF/WMF（Office 常见）、TIFF（Keynote 导出）浏览器解不了：加载失败时把字节交给后台用系统 GDI/WIC 转成 BMP
document.addEventListener('error', async e => {
  const el = e.target, svg = el.tagName === 'image', src = svg ? el.getAttribute('href') || el.getAttribute('xlink:href') : el.src;
  if (!(el.tagName === 'IMG' || svg) || el.dataset.conv || !/^(blob|data):/.test(src || '')) return;
  el.dataset.conv = 1;
  try {
    const r = await fetch('http://peek.localhost/imgconv', { method: 'POST', body: await (await fetch(src)).blob() });
    if (!r.ok) return;
    const url = URL.createObjectURL(await r.blob());
    svg ? el.setAttribute('href', url) : el.src = url;
  } catch { }
}, true);

// 系统属性读不到真实尺寸时（info.sized=false，如 SVG），页面拿到尺寸后再让窗口校正一次
const fit = (w, h, t) => { if (t === token && !cur.sized && w > 0 && h > 0) post('fit', { w, h }); };
// 加密/损坏、系统渲染不了的 PDF 才退回 WebView2 内置查看器：去掉顶部工具栏，
// FitH 总会多出几像素横向滚动，把 iframe 加高后裁掉底部那条横向滚动条
const pdfFrame = url => `<div class="center"><iframe class="full pdf" src="${url}#toolbar=0&navpanes=0&view=FitH"></iframe></div>`;

// PDF 由 Rust 用 Windows.Data.Pdf 按页渲染：先拿各页尺寸排好占位，只加载滚到附近的页，大文件也能立刻出第一页
async function renderPdf(path, url, t) {
  const q = `p=${encodeURIComponent(path)}`;
  const r = await fetch(`http://peek.localhost/pdfinfo?${q}`);
  if (t !== token) return;
  if (!r.ok) { view.innerHTML = pdfFrame(url); return; }
  const pages = await r.json();
  if (t !== token) return;
  setMeta('页数', pages.length);
  view.innerHTML = '<div class="pdf"></div>';
  const host = view.firstChild;
  const io = new IntersectionObserver(es => es.forEach(e => {
    if (!e.isIntersecting) return;
    const d = e.target; io.unobserve(d);
    const img = new Image(); img.alt = ''; img.draggable = false;
    img.onload = () => d.classList.add('ok');
    img.src = `http://peek.localhost/pdfpage?${q}&n=${d.dataset.n}&w=${Math.round(d.clientWidth * devicePixelRatio)}`;
    d.appendChild(img);
  }), { root: view, rootMargin: '150% 0px' });
  for (let i = 0; i < pages.length; i++) {
    const d = document.createElement('div');
    d.className = 'pg'; d.dataset.n = i; d.style.aspectRatio = pages[i][0] + ' / ' + pages[i][1];
    host.appendChild(d); io.observe(d);
  }
}

// ---------- 渲染器 ----------
// 信息卡片（文件夹、不支持的格式、出错）：左边系统图标，右边名称、说明、大小和修改时间，同原版
function card(info, sub) {
  const extra = [info.sizeText, info.modified].filter(Boolean).join('　');
  view.innerHTML = `<div class="card"><img alt="" src="${thumbUrl(info.path, 256)}"><div class="txt"><div class="big">${esc(info.name)}</div>` +
    `<div class="sub">${esc(sub || '')}</div>${extra ? `<div class="sub">${esc(extra)}</div>` : ''}</div></div>`;
  view.querySelector('img').onerror = function () { this.remove(); };
}

function converting(info) {
  view.innerHTML = `<div class="conv"><img alt="" src="${thumbUrl(info.path, 1280)}"><div class="sub">正在转换完整预览…（第一次约 10 秒，之后秒开）</div></div>`;
  const img = view.querySelector('img');
  img.onerror = () => img.remove();
  img.onload = () => img.style.maxWidth = img.naturalWidth + 'px';   // 系统缩略图分辨率不高，不放大，免得糊
}

function renderImage(info, src, isThumb) {
  view.innerHTML = `<div class="zoomwrap checker"><img class="img" alt="" draggable="false"></div>`;
  const img = view.querySelector('img'), wrap = view.firstChild;
  const t = token;
  img.onload = () => {
    if (t !== token) return;
    if (!isThumb) setMeta('尺寸', img.naturalWidth + ' × ' + img.naturalHeight);
    fit(img.naturalWidth, img.naturalHeight, t);
  };
  img.onerror = () => { if (t !== token) return; if (!isThumb) renderImage(info, thumbUrl(info.path, 2048), true); else card(info, '无法解码此图片'); };
  img.src = src;
  let scale = 0;
  const apply = () => {
    if (!scale) { img.classList.remove('actual'); img.style.width = ''; return; }
    img.classList.add('actual'); img.style.width = img.naturalWidth * scale + 'px';
  };
  wrap.addEventListener('dblclick', () => { scale = scale ? 0 : 1; apply(); });
  // 滚轮直接缩放，以鼠标所在点为中心
  wrap.addEventListener('wheel', e => {
    e.preventDefault();
    if (!img.naturalWidth) return;
    const r = wrap.getBoundingClientRect(), mx = e.clientX - r.left, my = e.clientY - r.top;
    const ow = img.clientWidth, oh = img.clientHeight;
    const fx = (wrap.scrollLeft + mx - img.offsetLeft) / ow, fy = (wrap.scrollTop + my - img.offsetTop) / oh;
    if (!scale) scale = ow / img.naturalWidth;
    scale = Math.min(16, Math.max(0.02, scale * (e.deltaY < 0 ? 1.15 : 1 / 1.15)));
    apply();
    wrap.scrollLeft = img.offsetLeft + fx * img.clientWidth - mx;
    wrap.scrollTop = img.offsetTop + fy * img.clientHeight - my;
    setMeta('缩放', Math.round(scale * 100) + '%');
  }, { passive: false });
  let drag = null;
  wrap.addEventListener('mousedown', e => { drag = { x: e.clientX, y: e.clientY, l: wrap.scrollLeft, t: wrap.scrollTop }; });
  window.onmouseup = () => drag = null;
  wrap.addEventListener('mousemove', e => { if (drag) { wrap.scrollLeft = drag.l - (e.clientX - drag.x); wrap.scrollTop = drag.t - (e.clientY - drag.y); } });
  $('#b-ocr').hidden = false;
}

function renderMedia(info, tag) {
  view.innerHTML = `<div class="center"><${tag} controls autoplay src="${info.url}"></${tag}></div>`;
  const m = view.querySelector(tag);
  const t = token;
  m.onloadedmetadata = () => {
    if (t !== token) return;
    setMeta('时长', fmtDur(m.duration));
    if (tag === 'video' && m.videoWidth) { setMeta('分辨率', m.videoWidth + ' × ' + m.videoHeight); fit(m.videoWidth, m.videoHeight, t); }
  };
  m.onerror = () => t === token && card(info, '内置播放器不支持此编码，按 Enter 用默认播放器打开');
  // 点过进度条/音量/播放键后焦点留在控件里，控件会吞掉所有按键（空格、Esc、↑/↓ 都到不了页面），用完就让出焦点
  for (const ev of ['seeked', 'volumechange', 'play', 'pause', 'ratechange'])
    m.addEventListener(ev, () => { if (document.activeElement === m) m.blur(); });
}

async function renderText(info, t) {
  const max = info.ext === 'json' || info.ext === 'ipynb' ? 8 << 20 : 2 << 20;
  const { buf, truncated } = await readHead(info.url, max);
  if (t !== token) return;
  if (!buf.length) return card(info, '空文件');
  const [text, enc] = decode(buf);
  setMeta('编码', enc);
  let lines = 1; for (let i = 0; i < text.length; i++) if (text.charCodeAt(i) === 10) lines++;
  setMeta('行数', (truncated ? '≥ ' : '') + lines.toLocaleString());
  const note = truncated ? `<div class="note">文件较大，只显示开头 ${fmtSize(buf.length)}</div>` : '';
  let body = text;
  if ((info.ext === 'json' || info.ext === 'ipynb') && !truncated) { try { body = JSON.stringify(JSON.parse(text), null, 2); } catch (e) { } }
  view.innerHTML = note + `<pre class="code"><code></code></pre>`;
  const code = view.querySelector('code');
  code.textContent = body;
  if (body.length < 400000) {
    await lib('highlight.min.js'); if (t !== token) return;
    const lang = HL_LANG[info.ext] || info.ext;
    try {
      code.innerHTML = hljs.getLanguage(lang) ? hljs.highlight(body, { language: lang, ignoreIllegals: true }).value : (info.kind === 'text' ? hljs.highlightAuto(body.slice(0, 100000)).value + esc(body.slice(100000)) : esc(body));
      code.classList.add('hljs');
    } catch (e) { }
  }
}

// Markdown 里可以夹任意 HTML：去掉能执行脚本或跳走的元素、on* 事件属性和 javascript: 链接。
// 页面本身还有 CSP 禁止内联脚本，这里是第二道
function sanitize(html) {
  const tpl = document.createElement('template');
  tpl.innerHTML = html;
  tpl.content.querySelectorAll('script,style,iframe,frame,object,embed,link,meta,base,form,svg,math').forEach(e => e.remove());
  tpl.content.querySelectorAll('*').forEach(e => {
    for (const a of [...e.attributes]) {
      const n = a.name.toLowerCase(), v = a.value.replace(/[\s\u0000-\u001f]/g, '').toLowerCase();
      if (n.startsWith('on') || n === 'style' || n === 'srcdoc' || n === 'formaction' ||
        ((n === 'href' || n === 'src' || n === 'xlink:href' || n === 'action') && /^(javascript|vbscript|data:text)/.test(v))) e.removeAttribute(a.name);
    }
  });
  return tpl.content;
}

async function renderMarkdown(info, t) {
  const { buf } = await readHead(info.url, 4 << 20);
  await Promise.all([lib('marked.umd.js'), lib('highlight.min.js'), css('github-markdown.css')]);
  if (t !== token) return;
  const [text, enc] = decode(buf);
  setMeta('编码', enc);
  const base = info.url.slice(0, info.url.lastIndexOf('/') + 1);
  let html = marked.parse(text);
  view.innerHTML = `<div class="mdwrap markdown-body"><div class="mdbody"><article class="md"></article></div></div>`;
  const wrap = view.firstChild, body = wrap.firstChild, art = body.firstChild;
  art.appendChild(sanitize(html));
  art.querySelectorAll('img').forEach(i => { const s = i.getAttribute('src'); if (s && !/^[a-z]+:/i.test(s)) i.src = base + s; });
  art.querySelectorAll('pre code').forEach(c => { try { hljs.highlightElement(c); } catch (e) { } });
  art.querySelectorAll('a').forEach(a => a.addEventListener('click', e => e.preventDefault()));
  // 侧边目录：至少两个标题才显示，点击跳转，滚动时高亮当前章节
  const hs = [...art.querySelectorAll('h1,h2,h3,h4')];
  if (hs.length < 2) return;
  const toc = document.createElement('nav'); toc.className = 'toc';
  const top = Math.min(...hs.map(h => +h.tagName[1]));
  const links = hs.map(h => {
    const a = document.createElement('a');
    a.textContent = h.textContent; a.title = h.textContent;
    a.style.marginLeft = (+h.tagName[1] - top) + 'em';
    a.onclick = () => body.scrollTo({ top: h.offsetTop - 8 });
    toc.appendChild(a); return a;
  });
  wrap.prepend(toc);
  let on = -1;
  const spy = () => {
    let i = 0; const y = body.scrollTop + 24;
    while (i + 1 < hs.length && hs[i + 1].offsetTop <= y) i++;
    if (body.scrollTop > 0 && body.scrollTop + body.clientHeight >= body.scrollHeight - 2) i = hs.length - 1;
    if (i === on) return;
    if (on >= 0) links[on].classList.remove('on');
    links[i].classList.add('on'); on = i;
  };
  body.addEventListener('scroll', spy, { passive: true }); spy();
}

function parseCsv(text, sep) {
  const rows = []; let row = [], f = '', q = false;
  for (let i = 0; i < text.length && rows.length < 3000; i++) {
    const c = text[i];
    if (q) { if (c === '"') { if (text[i + 1] === '"') { f += '"'; i++; } else q = false; } else f += c; }
    else if (c === '"') q = true;
    else if (c === sep) { row.push(f); f = ''; }
    else if (c === '\n' || c === '\r') { if (c === '\r' && text[i + 1] === '\n') i++; row.push(f); rows.push(row); row = []; f = ''; }
    else f += c;
  }
  if (f || row.length) { row.push(f); rows.push(row); }
  return rows;
}
async function renderCsv(info, t) {
  const { buf, truncated } = await readHead(info.url, 4 << 20);
  if (t !== token) return;
  const [text, enc] = decode(buf);
  setMeta('编码', enc);
  const sep = info.ext === 'tsv' ? '\t' : (text.split('\n')[0].split('\t').length > text.split('\n')[0].split(',').length ? '\t' : ',');
  const rows = parseCsv(text, sep);
  setMeta('行数', (truncated || rows.length >= 3000 ? '≥ ' : '') + rows.length);
  view.innerHTML = (truncated || rows.length >= 3000 ? `<div class="note">只显示前 ${rows.length} 行</div>` : '') +
    '<table class="grid">' + rows.map(r => '<tr>' + r.map(c => `<td title="${esc(c)}">${esc(c)}</td>`).join('') + '</tr>').join('') + '</table>';
}

async function renderDocx(info, t) {
  view.innerHTML = '<div class="spin">正在解析文档…</div>';
  const [buf] = await Promise.all([buffer(info.url), lib('docx-preview.min.js')]);
  if (t !== token) return;
  const host = document.createElement('div'); host.className = 'docx-host';
  const doc = await docx.renderAsync(buf, host, null, { inWrapper: true, breakPages: true, ignoreLastRenderedPageBreak: false, renderHeaders: true, renderFooters: true, renderFootnotes: true, useBase64URL: false, experimental: true });
  if (t !== token) return;
  // 目录项带“超链接”字符样式，WPS/Word 显示成普通黑字，docx-preview 照样式渲染成蓝色下划线
  const styles = doc?.stylesPart?.styles || [], cls = s => 'docx_' + s.id.replace(/[ .]+/g, '-').replace(/[&]+/g, 'and').toLowerCase();
  const link = styles.filter(s => /^hyperlink$/i.test(s.name || '')).map(cls);
  styles.filter(s => /^toc \d$/i.test(s.name || '')).forEach(s => host.querySelectorAll('p.' + CSS.escape(cls(s)) + ' span').forEach(sp => sp.classList.remove(...link)));
  // 单元格里的负缩进：WPS/Word 截在单元格内，docx-preview 照搬成负 margin 会伸出边框
  host.querySelectorAll('td p').forEach(p => {
    if (parseFloat(p.style.marginInlineEnd) < 0) p.style.marginInlineEnd = '0';
    if (parseFloat(p.style.marginInlineStart) < 0) p.style.marginInlineStart = '0';
    // 悬挂缩进比左缩进还大（编号段落常见）时首行也会伸出边框，夹到单元格左边
    const cs = getComputedStyle(p), ml = parseFloat(cs.marginLeft) || 0, ti = parseFloat(cs.textIndent) || 0;
    if (ml + ti < 0) p.style.textIndent = -ml + 'px';
  });
  view.innerHTML = ''; view.appendChild(host);
  const pages = host.querySelectorAll('section.docx').length;
  if (pages) setMeta('页数', '约 ' + pages);
  fitDocx(host);
}
function fitDocx(host) {
  const sec = host.querySelector('section.docx'); if (!sec) return;
  const w = sec.offsetWidth + 40, avail = view.clientWidth;
  host.style.zoom = w > avail ? (avail / w).toFixed(3) : '';
}

async function renderSheet(info, t) {
  view.innerHTML = '<div class="spin">正在解析表格…</div>';
  const [buf] = await Promise.all([buffer(info.url), lib('xlsx.full.min.js')]);
  if (t !== token) return;
  const wb = XLSX.read(buf, { type: 'array', sheetRows: 1001, cellStyles: false, cellDates: true, dense: true });
  if (t !== token) return;
  setMeta('工作表', wb.SheetNames.length);
  view.innerHTML = '<div class="sheetwrap"><div class="sheet"></div><div class="tabs"></div></div>';
  const sheet = view.querySelector('.sheet'), tabs = view.querySelector('.tabs');
  const show = i => {
    const ws = wb.Sheets[wb.SheetNames[i]];
    let html = XLSX.utils.sheet_to_html(ws, { header: '', footer: '' });
    sheet.innerHTML = html.replace('<table', '<table class="grid"');
    const ref = ws['!ref'] ? XLSX.utils.decode_range(ws['!ref']) : null;
    if (ref) setMeta('区域', (ref.e.r + 1) + ' 行 × ' + (ref.e.c + 1) + ' 列' + (ref.e.r >= 1000 ? '（只显示前 1000 行）' : ''));
    tabs.querySelectorAll('button').forEach((b, j) => b.classList.toggle('on', j === i));
  };
  wb.SheetNames.forEach((n, i) => { const b = document.createElement('button'); b.textContent = n; b.onclick = () => show(i); tabs.appendChild(b); });
  if (wb.SheetNames.length) show(0);
}

async function renderPptx(info, t) {
  view.innerHTML = '<div class="spin">正在解析演示文稿…</div>';
  const [buf] = await Promise.all([buffer(info.url), lib('pptx-preview.umd.js')]);
  if (t !== token) return;
  // 按演示文稿自己的页面比例（16:9、4:3…）排版，写死 16:9 会把 4:3 的图形拉变形
  let ratio = 9 / 16;
  try {
    const xml = await (await JSZip.loadAsync(buf)).file('ppt/presentation.xml').async('string');
    const m = /<p:sldSz[^>]*\bcx="(\d+)"[^>]*\bcy="(\d+)"/.exec(xml);
    if (m && +m[1] > 0) { ratio = +m[2] / +m[1]; setMeta('页面', (+m[1] / 360000).toFixed(1) + ' × ' + (+m[2] / 360000).toFixed(1) + ' cm'); }
  } catch (e) { }
  if (t !== token) return;
  const host = document.createElement('div'); host.className = 'pptx-host';
  view.innerHTML = ''; view.appendChild(host);
  const w = Math.min(1280, view.clientWidth - 64);
  const p = pptxPreview.init(host, { width: w, height: Math.round(w * ratio), mode: 'list' });
  await p.preview(buf);
  if (t !== token) return;
  try { const n = p.slideCount || (p.pptx && p.pptx.slides && p.pptx.slides.length); if (n) setMeta('幻灯片', n + ' 张'); } catch (e) { }
}

async function renderZip(info, t) {
  if (info.size > 300 << 20) return card(info, '压缩包较大，不展开目录');
  view.innerHTML = '<div class="spin">正在读取压缩包目录…</div>';
  const zip = await JSZip.loadAsync(await buffer(info.url));
  if (t !== token) return;
  const rows = []; let total = 0, files = 0;
  zip.forEach((p, f) => {
    const s = f._data && f._data.uncompressedSize || 0;
    if (!f.dir) { files++; total += s; }
    rows.push({ p, dir: f.dir, s, d: f.date });
  });
  rows.sort((a, b) => a.p.localeCompare(b.p));
  setMeta('内容', files + ' 个文件，解压后 ' + fmtSize(total));
  view.innerHTML = '<table class="list">' + rows.slice(0, 5000).map(r => {
    const depth = r.p.replace(/\/$/, '').split('/').length - 1;
    const name = r.p.replace(/\/$/, '').split('/').pop();
    return `<tr><td class="n" style="padding-left:${14 + depth * 16}px">${r.dir ? '📁 ' : ''}${esc(name)}</td><td class="r">${r.dir ? '' : fmtSize(r.s)}</td><td class="r">${r.d ? r.d.toLocaleString() : ''}</td></tr>`;
  }).join('') + '</table>';
}

async function renderFont(info, t) {
  const ff = new FontFace('peekfont' + t, `url("${info.url}")`);
  try { await ff.load(); } catch (e) { return t === token && card(info, '无法加载字体'); }
  if (t !== token) return;
  document.fonts.add(ff); curFont = ff;
  const s = 'The quick brown fox jumps over the lazy dog 0123456789 永和九年，岁在癸丑，暮春之初';
  view.innerHTML = `<div class="font-sample" style="font-family:'peekfont${t}'">` +
    [12, 16, 24, 36, 48, 72].map(px => `<div style="font-size:${px}px">${s}</div>`).join('') +
    `<div style="font-size:32px">ABCDEFGHIJKLMNOPQRSTUVWXYZ<br>abcdefghijklmnopqrstuvwxyz<br>!@#$%^&*()[]{}</div></div>`;
}

// ---------- 入口 ----------
const peek = window.peek = {
  async show(info) {
    const t = ++token;
    cur = info; meta = {};
    $('#name').textContent = info.name;
    $('#b-ocr').hidden = true; setPop('ocrpop', false); $('#hash').textContent = ''; $('#b-hash').disabled = false;
    if (curFont) { document.fonts.delete(curFont); curFont = null; }
    showFind(false);
    view.scrollTop = 0; view.scrollLeft = 0;
    renderProps();
    if (!info.exists) return card(info, '文件不存在');
    try {
      switch (info.kind) {
        case 'folder': return card(info, '文件夹');
        case 'image': return renderImage(info, info.url);
        case 'thumbimg': return renderImage(info, thumbUrl(info.path, 2560), true);
        case 'video': return renderMedia(info, 'video');
        case 'audio': return renderMedia(info, 'audio');
        case 'pdf': return await renderPdf(info.path, info.url, t);
        case 'html': view.innerHTML = `<iframe class="full" sandbox="" src="${info.url}"></iframe>`; return;
        case 'md': return await renderMarkdown(info, t);
        case 'csv': return await renderCsv(info, t);
        case 'docx': return await renderDocx(info, t);
        case 'sheet': return await renderSheet(info, t);
        case 'pptx': return await renderPptx(info, t);
        case 'legacy':
          converting(info);
          post('convert'); return;
        case 'zip': return await renderZip(info, t);
        case 'font': return await renderFont(info, t);
        case 'text': return await renderText(info, t);
        default: return card(info, info.ext ? info.ext.toUpperCase() + ' 文件' : '文件');
      }
    } catch (e) {
      if (t === token) card(info, '预览失败：' + (e && e.message || e));
    }
  },
  converted(r) {
    if (!cur || r.path !== cur.path) return;
    if (r.error) return card(cur, r.error);
    renderPdf(r.pdf, r.url, token);
  },
  clear() { token++; view.innerHTML = ''; cur = null; },
  position(i, n) { if (cur) { cur.index = i; cur.total = n; } },
  setPinned(b) { pinned = b; $('#b-pin').classList.toggle('on', b); },
  hash(h) { if (h.path !== cur?.path) return; $('#hash').textContent = h.error ? h.error : `MD5\n${h.md5}\nSHA256\n${h.sha256}`; $('#b-hash').disabled = false; },
  ocr(r) {
    $('#b-ocr').disabled = false;
    if (!cur || r.path && r.path !== cur.path) return;
    $('#ocr').textContent = cur.ocr = r.error ? '识别失败：' + r.error : (r.text || '（没有识别到文字）');
    setPop('ocrpop', true);
  },
};

// ---------- 交互 ----------
function copyPath() { if (cur) navigator.clipboard.writeText(cur.path).then(() => toast('已复制路径')); }
// 浮动卡片：详细信息、识别结果各有一个开关按钮；两者互斥，点卡片和按钮以外的地方就关掉（查找框例外，一直留着）
const POPS = { side: '#b-info', ocrpop: '#b-ocr' };
function setPop(id, on) {
  if (on) for (const k in POPS) if (k !== id) setPop(k, false);
  $('#' + id).hidden = !on; $(POPS[id]).classList.toggle('on', on);
  if (id === 'side' && on) renderProps();
}
document.addEventListener('mousedown', e => {
  for (const k in POPS) if (!$('#' + k).hidden && !e.target.closest('#' + k + ',' + POPS[k])) setPop(k, false);
}, true);

// 查找：自己遍历预览区的文字，用 CSS Custom Highlight 高亮全部结果和当前结果（不改动页面 DOM），显示"第几个/共几个"。
// window.find 在输入框有焦点时找不到页面里的字，PPT 等预览里点上下箭头没反应，所以不用它
let hits = [], hit = -1;
function showFind(on) {
  $('#findbar').hidden = !on; $('#b-find').classList.toggle('on', on);
  if (on) { $('#find-in').focus(); $('#find-in').select(); runFind(); }
  else { hits = []; hit = -1; CSS.highlights.clear(); $('#find-st').textContent = ''; }
}
const toggleFind = () => showFind($('#findbar').hidden);
function runFind() {
  const q = $('#find-in').value.toLowerCase();
  hits = []; hit = -1; CSS.highlights.clear();
  if (q) {
    const tw = document.createTreeWalker(view, NodeFilter.SHOW_TEXT);
    for (let n; (n = tw.nextNode()) && hits.length < 5000;) {
      const t = n.data.toLowerCase();
      for (let i = t.indexOf(q); i >= 0 && hits.length < 5000; i = t.indexOf(q, i + q.length)) {
        const r = new Range(); r.setStart(n, i); r.setEnd(n, i + q.length);
        if (r.getClientRects().length) hits.push(r);   // 跳过看不见的（display:none 的工作表、折叠内容）
      }
    }
    if (hits.length) { CSS.highlights.set('peek-hit', new Highlight(...hits)); return goFind(0); }
  }
  $('#find-st').textContent = q ? '0/0' : '';
}
function goFind(i) {
  if (!hits.length) return;
  hit = (i + hits.length) % hits.length;
  CSS.highlights.set('peek-cur', new Highlight(hits[hit]));
  hits[hit].startContainer.parentElement.scrollIntoView({ block: 'center', inline: 'nearest' });
  $('#find-st').textContent = (hit + 1) + '/' + hits.length;
}
const doFind = back => { if (hits.length) goFind(hit + (back ? -1 : 1)); else runFind(); $('#find-in').focus(); };
let findT;
$('#find-in').addEventListener('input', () => { clearTimeout(findT); findT = setTimeout(runFind, 120); });
$('#find-in').addEventListener('keydown', e => {
  if (e.key === 'Enter') { e.preventDefault(); doFind(e.shiftKey); }
  else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); showFind(false); }
  e.stopPropagation();
});
$('#b-find').onclick = toggleFind;
$('#f-prev').onclick = () => doFind(true);
$('#f-next').onclick = () => doFind(false);
$('#f-close').onclick = () => showFind(false);
$('#b-copy').onclick = copyPath;
$('#b-open').onclick = () => post('open');
$('#b-pin').onclick = () => post('pin');
$('#b-info').onclick = () => setPop('side', $('#side').hidden);
$('#b-hash').onclick = () => { $('#b-hash').disabled = true; $('#hash').textContent = '计算中…'; post('hash'); };
$('#b-ocr').onclick = () => {
  if (!$('#ocrpop').hidden) return setPop('ocrpop', false);
  if (cur && cur.ocr != null) { $('#ocr').textContent = cur.ocr; return setPop('ocrpop', true); }   // 同一张图识别过就直接显示
  $('#b-ocr').disabled = true; toast('正在识别文字…'); post('ocr');
};

$('#b-close').onclick = () => post('close');
// 无系统标题栏：按住顶栏空白处（不是按钮）拖动窗口
$('#bar').addEventListener('mousedown', e => { if (e.button === 0 && !e.target.closest('button')) { e.preventDefault(); post('drag'); } });
// 查找框有焦点时告诉原生钩子别拦空格和方向键
$('#find-in').addEventListener('focus', () => post('typing', { on: true }));
$('#find-in').addEventListener('blur', () => post('typing', { on: false }));
$('#b-ocrcopy').onclick = () => navigator.clipboard.writeText($('#ocr').textContent).then(() => toast('已复制识别结果'));

document.addEventListener('keydown', e => {
  const tag = (e.target.tagName || '').toLowerCase();
  if (tag === 'input' || tag === 'textarea') return;
  const k = e.key;
  if (k === 'Escape' || (k === ' ' && !e.ctrlKey)) { e.preventDefault(); post('close'); }
  else if (k === 'ArrowUp' && !e.ctrlKey) { e.preventDefault(); post('prev'); }
  else if (k === 'ArrowDown' && !e.ctrlKey) { e.preventDefault(); post('next'); }
  else if (k === 'Enter') { e.preventDefault(); post('open'); }
  else if (e.ctrlKey && (k === 'c' || k === 'C')) {
    if (!String(window.getSelection())) { e.preventDefault(); copyPath(); }
  }
  else if (e.ctrlKey && (k === 'f' || k === 'F')) { e.preventDefault(); toggleFind(); }
  else if (e.ctrlKey && (k === 't' || k === 'T')) { e.preventDefault(); post('pin'); }
  else if (!e.ctrlKey && (k === 'i' || k === 'I')) { $('#b-info').click(); }
}, true);
window.addEventListener('resize', () => { const h = view.querySelector('.docx-host'); if (h) fitDocx(h); });
document.addEventListener('contextmenu', e => { if (!String(window.getSelection())) e.preventDefault(); });

post('ready');
