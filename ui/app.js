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

const EXT = {
  image: 'jpg jpeg jfif pjpeg png apng gif webp bmp ico cur svg avif',
  thumbimg: 'heic heif tif tiff psd raw cr2 cr3 nef arw dng orf rw2 raf jxr wdp dds tga exr hdr',
  video: 'mp4 m4v webm mov mkv ogv 3gp ts m2ts',
  audio: 'mp3 wav flac ogg oga opus m4a aac weba',
  pdf: 'pdf',
  md: 'md markdown mdown mkd',
  html: 'html htm xhtml mht',
  csv: 'csv tsv',
  docx: 'docx docm dotx dotm',
  sheet: 'xlsx xlsm xlsb xls xltx ods et ett numbers',
  pptx: 'pptx pptm ppsx potx',
  legacy: 'doc dot wps wpt rtf odt ott ppt pps pot dps dpt odp odg vsd',
  zip: 'zip jar apk xpi vsix nupkg epub cbz whl aar ipa',
  font: 'ttf otf woff woff2',
  text: 'txt log ini cfg conf config toml yaml yml json jsonc json5 xml xaml csproj vcxproj props targets sln plist svgz ' +
    'c h cc cpp cxx hpp hh hxx cs java kt kts scala go rs py pyw pyi rb php pl pm lua r dart swift m mm ' +
    'js mjs cjs jsx ts tsx vue svelte css scss sass less styl astro ' +
    'sh bash zsh fish ps1 psm1 psd1 bat cmd vbs ahk reg ' +
    'sql graphql gql proto thrift prisma ' +
    'gradle properties env gitignore gitattributes editorconfig dockerfile makefile cmake mk ninja ' +
    'tex bib rst adoc org srt ass vtt lrc nfo diff patch ipynb ets json5 hap ' +
    'asm s v sv vhd vhdl zig nim ex exs erl hrl clj cljs hs elm fs fsx ml mli jl groovy tf hcl nix',
};
const KIND = {};
for (const [k, v] of Object.entries(EXT)) for (const e of v.split(/\s+/)) if (e && !KIND[e]) KIND[e] = k;
const HL_LANG = { rs: 'rust', py: 'python', js: 'javascript', mjs: 'javascript', cjs: 'javascript', ts: 'typescript', tsx: 'typescript', jsx: 'javascript',
  cs: 'csharp', kt: 'kotlin', h: 'cpp', hpp: 'cpp', cc: 'cpp', cxx: 'cpp', c: 'c', sh: 'bash', zsh: 'bash', ps1: 'powershell', psm1: 'powershell',
  bat: 'dos', cmd: 'dos', yml: 'yaml', toml: 'ini', cfg: 'ini', conf: 'ini', vue: 'xml', html: 'xml', xaml: 'xml', csproj: 'xml', svg: 'xml',
  md: 'markdown', ets: 'typescript', rb: 'ruby', pl: 'perl', m: 'objectivec', jsonc: 'json', json5: 'json', ipynb: 'json', gradle: 'groovy', tf: 'ini' };

let cur = null, token = 0, meta = {}, pinned = false;
const view = $('#view');

const loaded = {};
function lib(src) {
  return loaded[src] || (loaded[src] = new Promise((ok, bad) => {
    const s = document.createElement('script');
    s.src = 'vendor/' + src; s.onload = ok; s.onerror = bad;
    document.head.appendChild(s);
  }));
}

function toast(t) {
  const el = $('#toast'); el.textContent = t; el.hidden = false;
  clearTimeout(toast.t); toast.t = setTimeout(() => el.hidden = true, 1400);
}

function setMeta(k, v) { if (v === '' || v == null) delete meta[k]; else meta[k] = v; renderStatus(); renderProps(); }

function renderStatus() {
  if (!cur) return;
  const parts = [];
  const size = cur.isDir ? (cur.folderSizeText || '统计中…') : cur.sizeText;
  if (size) parts.push('<b>' + esc(size) + '</b>');
  for (const [k, v] of Object.entries(meta)) parts.push(esc(k) + ' ' + esc(v));
  if (cur.modified) parts.push('修改 ' + esc(cur.modified));
  $('#status').innerHTML = parts.join(' · ');
}

function renderProps() {
  if (!cur || $('#side').hidden) return;
  const rows = [['名称', cur.name], ['位置', cur.path], ['大小', cur.isDir ? cur.folderSizeText : cur.sizeText],
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
    try { return [new TextDecoder('utf-8', { fatal: true }).decode(buf.subarray(0, buf.length - 3)), 'UTF-8']; }
    catch (e2) { return [new TextDecoder('gbk').decode(buf), 'GBK']; }
  }
}
function looksText(buf) {
  const n = Math.min(buf.length, 8192);
  for (let i = 0; i < n; i++) if (buf[i] === 0) return buf[0] === 0xFF && buf[1] === 0xFE || buf[0] === 0xFE && buf[1] === 0xFF;
  return true;
}
const buffer = url => fetch(url).then(r => { if (!r.ok) throw new Error('HTTP ' + r.status); return r.arrayBuffer(); });
const thumbUrl = (p, s) => `http://peek.localhost/thumb?p=${encodeURIComponent(p)}&s=${s || 1280}&t=${Date.now()}`;

// 让窗口按内容比例调整：w/h 是内容区想要的 CSS 像素，ew/eh 是标题栏、状态栏、侧栏占掉的部分。
// 等窗口真的变了（或 150 ms 超时）再返回，后续按新尺寸排版
function fit(w, h, t) {
  if (t != null && t !== token) return Promise.resolve();
  const ew = window.innerWidth - view.clientWidth, eh = window.innerHeight - view.clientHeight;
  return new Promise(ok => {
    const done = () => { clearTimeout(tm); window.removeEventListener('resize', f); ok(); };
    const f = () => requestAnimationFrame(done);
    const tm = setTimeout(done, 150);
    window.addEventListener('resize', f);
    post('fit', { w, h, ew, eh });
  });
}
// 文档类本来就上下滚动，不按 A4 整页比例（那样宽度只剩 580，字太小），取接近 1:1 的竖窗口
const DEFAULT_SIZE = [1100, 700], PAGE_SIZE = [880, 900];

// ---------- 渲染器 ----------
function card(info, sub) {
  view.innerHTML = `<div class="card"><img alt="" src="${thumbUrl(info.path, 512)}"><div class="big">${esc(info.name)}</div><div class="sub">${esc(sub || '')}</div></div>`;
  view.querySelector('img').onerror = function () { this.remove(); };
}

function renderImage(info, src, isThumb) {
  view.innerHTML = `<div class="zoomwrap checker"><img class="img" alt="" draggable="false"></div>`;
  const img = view.querySelector('img'), wrap = view.firstChild;
  const t = token;
  img.onload = () => {
    if (!isThumb) setMeta('尺寸', img.naturalWidth + ' × ' + img.naturalHeight);
    fit(img.naturalWidth, img.naturalHeight, t);
  };
  img.onerror = () => { if (!isThumb) renderImage(info, thumbUrl(info.path, 2048), true); else card(info, '无法解码此图片'); };
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
  window.addEventListener('mouseup', () => drag = null);
  wrap.addEventListener('mousemove', e => { if (drag) { wrap.scrollLeft = drag.l - (e.clientX - drag.x); wrap.scrollTop = drag.t - (e.clientY - drag.y); } });
  $('#b-ocr').hidden = false;
}

function renderMedia(info, tag) {
  view.innerHTML = `<div class="center"><${tag} controls autoplay src="${info.url}"></${tag}></div>`;
  const m = view.querySelector(tag);
  const t = token;
  m.onloadedmetadata = () => {
    setMeta('时长', fmtDur(m.duration));
    if (tag === 'video' && m.videoWidth) { setMeta('分辨率', m.videoWidth + ' × ' + m.videoHeight); fit(m.videoWidth, m.videoHeight, t); }
  };
  m.onerror = () => card(info, '内置播放器不支持此编码，按 Enter 用默认播放器打开');
}

async function renderText(info, t) {
  const max = info.ext === 'json' || info.ext === 'ipynb' ? 8 << 20 : 2 << 20;
  const { buf, truncated } = await readHead(info.url, max);
  if (t !== token) return;
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
  await Promise.all([lib('marked.umd.js'), lib('highlight.min.js')]);
  if (t !== token) return;
  const [text, enc] = decode(buf);
  setMeta('编码', enc);
  const base = info.url.slice(0, info.url.lastIndexOf('/') + 1);
  let html = marked.parse(text);
  view.innerHTML = `<div class="mdwrap"><div class="mdbody"><article class="md"></article></div></div>`;
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
    a.style.paddingLeft = 12 + (+h.tagName[1] - top) * 14 + 'px';
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
  await docx.renderAsync(buf, host, null, { inWrapper: true, breakPages: true, ignoreLastRenderedPageBreak: false, renderHeaders: true, renderFooters: true, renderFootnotes: true, useBase64URL: false, experimental: true });
  if (t !== token) return;
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
  await fit(1280 + 64, Math.round(1280 * ratio) + 48, t);
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
  try { await ff.load(); } catch (e) { return card(info, '无法加载字体'); }
  if (t !== token) return;
  document.fonts.add(ff);
  const s = 'The quick brown fox jumps over the lazy dog 0123456789 永和九年，岁在癸丑，暮春之初';
  view.innerHTML = `<div class="font-sample" style="font-family:'peekfont${t}'">` +
    [12, 16, 24, 36, 48, 72].map(px => `<div style="font-size:${px}px">${s}</div>`).join('') +
    `<div style="font-size:32px">ABCDEFGHIJKLMNOPQRSTUVWXYZ<br>abcdefghijklmnopqrstuvwxyz<br>!@#$%^&*()[]{}</div></div>`;
}

function renderFolder(info) {
  view.innerHTML = '<div class="spin">正在从 Everything 读取文件夹大小…</div>';
}
function folderStats(s) {
  if (!cur || !cur.isDir || s.path !== cur.path) return;
  cur.folderSizeText = s.sizeText || '未知';
  if (s.files != null) setMeta('包含', `${s.files.toLocaleString()} 个文件，${s.folders.toLocaleString()} 个文件夹`);
  else setMeta('提示', 'Everything 未运行，无法统计');
  const max = Math.max(1, ...s.children.map(c => c.size));
  view.innerHTML = s.children.length ? '<table class="list">' + s.children.map(c =>
    `<tr><td class="n bar-cell"><i style="width:${(c.size / max * 100).toFixed(1)}%"></i>${c.dir ? '📁 ' : ''}${esc(c.name)}</td><td class="r">${esc(c.sizeText)}</td></tr>`).join('') + '</table>'
    : `<div class="card"><div class="big">${esc(cur.name)}</div><div class="sub">空文件夹</div></div>`;
  renderStatus(); renderProps();
}

async function renderFallback(info, t) {
  // 小文件嗅探一下是不是文本
  if (info.size != null && info.size < 8 << 20) {
    try {
      const { buf } = await readHead(info.url, 8192);
      if (t !== token) return;
      if (buf.length && looksText(buf)) return renderText(Object.assign({}, info, { kind: 'text' }), t);
    } catch (e) { }
  }
  card(info, info.ext ? info.ext.toUpperCase() + ' 文件' : '文件');
}

// ---------- 入口 ----------
const peek = window.peek = {
  async show(info) {
    const t = ++token;
    cur = info; meta = {};
    info.kind = info.isDir ? 'folder' : (KIND[info.ext] || (/^(readme|license|makefile|dockerfile|changelog)$/i.test(info.name) ? 'text' : 'other'));
    $('#name').textContent = info.name;
    this.position(info.index, info.total);
    $('#b-ocr').hidden = true; $('#ocrbox').hidden = true; $('#hash').textContent = '';
    $('#findbar').hidden = true;
    view.scrollTop = 0; view.scrollLeft = 0;
    renderStatus(); renderProps();
    // 图片、视频、PPT 拿到真实尺寸后再调；文档类用竖版纸张比例，其余用默认大小
    const k = info.kind;
    if (k === 'legacy' && /^(ppt|pps|pot|dps|dpt|odp)$/.test(info.ext)) fit(1344, 768);
    else if (k === 'pdf' || k === 'docx' || k === 'legacy') fit(...PAGE_SIZE);
    else if (k !== 'image' && k !== 'thumbimg' && k !== 'video' && k !== 'pptx') fit(...DEFAULT_SIZE);
    if (!info.exists) return card(info, '文件不存在');
    try {
      switch (info.kind) {
        case 'folder': return renderFolder(info);
        case 'image': return renderImage(info, info.url);
        case 'thumbimg': return renderImage(info, thumbUrl(info.path, 2560), true);
        case 'video': return renderMedia(info, 'video');
        case 'audio': return renderMedia(info, 'audio');
        case 'pdf': view.innerHTML = `<iframe class="full" src="${info.url}"></iframe>`; return;
        case 'html': view.innerHTML = `<iframe class="full" sandbox="" src="${info.url}"></iframe>`; return;
        case 'md': return await renderMarkdown(info, t);
        case 'csv': return await renderCsv(info, t);
        case 'docx': return await renderDocx(info, t);
        case 'sheet': return await renderSheet(info, t);
        case 'pptx': return await renderPptx(info, t);
        case 'legacy':
          card(info, '正在用 LibreOffice 转换预览…（第一次约 10 秒，之后秒开）');
          post('convert'); return;
        case 'zip': return await renderZip(info, t);
        case 'font': return await renderFont(info, t);
        case 'text': return await renderText(info, t);
        default: return await renderFallback(info, t);
      }
    } catch (e) {
      if (t === token) card(info, '预览失败：' + (e && e.message || e));
    }
  },
  converted(r) {
    if (!cur || r.path !== cur.path) return;
    if (r.error) return card(cur, r.error);
    view.innerHTML = `<iframe class="full" src="${r.url}#view=FitH"></iframe>`;
  },
  clear() { token++; view.innerHTML = ''; cur = null; },
  position(i, n) { $('#pos').textContent = n > 1 ? `${i + 1} / ${n}` : ''; if (cur) { cur.index = i; cur.total = n; } },
  folder: folderStats,
  setPinned(b) { pinned = b; $('#b-pin').classList.toggle('on', b); },
  hash(h) { $('#hash').textContent = h.error ? h.error : `MD5\n${h.md5}\nSHA256\n${h.sha256}`; $('#b-hash').disabled = false; },
  ocr(r) {
    $('#b-ocr').disabled = false;
    $('#side').hidden = false; renderProps();
    $('#ocrbox').hidden = false;
    $('#ocr').textContent = r.error ? '识别失败：' + r.error : (r.text || '（没有识别到文字）');
  },
};

// ---------- 交互 ----------
function copyPath() { if (cur) navigator.clipboard.writeText(cur.path).then(() => toast('已复制路径')); }
function toggleFind() {
  const fb = $('#findbar'); fb.hidden = !fb.hidden;
  if (!fb.hidden) { $('#find-in').focus(); $('#find-in').select(); }
}
function doFind(back) {
  const q = $('#find-in').value; if (!q) return;
  const ok = window.find(q, false, back, true, false, false, false);
  $('#find-st').textContent = ok ? '' : '未找到';
  $('#find-in').focus();
}
$('#find-in').addEventListener('keydown', e => {
  if (e.key === 'Enter') { e.preventDefault(); doFind(e.shiftKey); }
  else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); $('#findbar').hidden = true; }
  e.stopPropagation();
});
$('#b-find').onclick = toggleFind;
$('#b-copy').onclick = copyPath;
$('#b-open').onclick = () => post('open');
$('#b-pin').onclick = () => post('pin');
$('#b-info').onclick = () => { $('#side').hidden = !$('#side').hidden; $('#b-info').classList.toggle('on', !$('#side').hidden); renderProps(); };
$('#b-hash').onclick = () => { $('#b-hash').disabled = true; $('#hash').textContent = '计算中…'; post('hash'); };
$('#b-ocr').onclick = () => { $('#b-ocr').disabled = true; toast('正在识别文字…'); post('ocr'); };

// 顶栏：鼠标进入窗口顶部 40px 显示，离开顶栏区域 400 ms 后收起；移出窗口也收起
let barT = 0;
const bar = on => { clearTimeout(barT); if (on) document.body.classList.add('bar-on'); else barT = setTimeout(() => document.body.classList.remove('bar-on'), 400); };
document.addEventListener('mousemove', e => bar(e.clientY < 40 || ($('#bar').matches(':hover') && document.body.classList.contains('bar-on'))));
document.documentElement.addEventListener('mouseleave', () => bar(false));
$('#hot').addEventListener('mouseenter', () => bar(true));
$('#bar').addEventListener('mouseenter', () => bar(true));
$('#bar').addEventListener('mouseleave', e => { if (e.clientY >= 40) bar(false); });
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
