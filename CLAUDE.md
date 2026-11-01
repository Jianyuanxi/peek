# QuickView — QuickLook 替代品（原名 Peek）

Windows 上资源管理器选中文件按空格预览。Rust（tao + wry/WebView2）单 exe，界面在 `ui/`，编译时用 `include_bytes!` 打进 exe。

## 用户定下的约束（不要回退）

- **名字是 QuickView**（用户定的；不直接叫 QuickLook，避免和原版 QL-Win 混淆）。exe、窗口标题、托盘、自启项 `QuickView`、数据目录 `%LOCALAPPDATA%\QuickView`（`info::data_dir`），启动时把旧 `Peek` 目录改名迁过来并删掉旧自启项。页面内部的 `peek` 协议、`window.peek`、`.peekfs` 不是用户可见的，没改。
- **图标**用 `assets/app.svg`（浅灰文档 + 灰蓝放大镜，低对比，用户选的 A2），`assets/app.ico` 由它渲染出 16–256 九个尺寸，`build.rs` 用 embed-resource 嵌成资源 1；窗口、任务栏、托盘都 `from_resource(1)`。
- **自启用启动文件夹快捷方式 `QuickView.lnk`，不要改回 HKCU `Run`**：本机开机时 Explorer 跳过了新写入的 Run 项（Shell-Core/Operational 日志 9707 里没有它，同次开机的 Span 等都执行了；补上 `RunNotification\StartupTNoti<名字>=1` 也照样跳过，已实测），快捷方式能正常启动（2026-10-20 重启验证）。启动时把旧的 `Run\Peek`、`Run\QuickView` 及其 `StartupApproved\Run` 记录删掉；“设置 > 启动应用”里关掉会在 `StartupApproved\StartupFolder\QuickView.lnk` 写奇数首字节，`autostart_enabled` 要认这个；首次运行只在快捷方式不存在时创建，不覆盖用户在设置里的开关。
- 开机自启、常驻，**空闲内存尽量小**：WebView2 按需创建，隐藏 `RELEASE_AFTER`（默认 300 秒）后释放。
- **速度第一，任何格式都是**（"和用 WPS 打开一样慢就没意义"）。PDF 不用 WebView2 内置查看器（要读完整个文件，80 MB 教材 5 秒），用 `native::pdf_render`（Windows.Data.Pdf）按页渲染 JPEG、只加载滚到附近的页，大文件 0.1–0.5 秒出第一页；只有系统渲染失败（加密等）才退回内置查看器。
- 空格出预览要快、不卡；Office 预览**速度优先**，清晰度可以比原版低，但要看得清内容（用户只是先看一眼，细看会用 WPS 打开）。
- 文件大小**自动换单位，1024 进制，保留两位小数**（如 `1.00 GB`，不要写成 `1024 MB`），不额外显示字节数，只在详细信息面板里显示。Rust `info::fmt_size` 和 JS `fmtSize` 两处保持一致。
- **不做文件夹大小/文件数统计**（用户嫌慢且没用，已删），不要加回来。
- **↑/↓ 切换上一个/下一个文件**（用户指定，不要改回 ←/→）。
- **空格/Esc 关闭、↑/↓ 切换在任何格式里都要生效**：PDF 查看器、视频控件拿到焦点后页面收不到按键。**QuickView 自己在前台时 Windows 不调用本进程的低级钩子**（2026-10-24 实测：别的进程也装了 LL 钩子时才会经链调到，不能指望 `kb_proc`），所以靠两层：WebView2 `AcceleratorKeyPressed`（`hook_keys`，接住 Esc/↑/↓，空格这类字符键它不触发），以及视频/音频控件用完就 `blur()`（点过进度条后控件会吞掉所有按键）。三处共用 `native::preview_key`；查找框聚焦时（`TYPING`）放行。
- **窗口大小照原版 QuickLook**，并且**在显示内容之前就调好**（先显示再调会先大后小闪一下）：Rust `info::preferred_size` 按类型给内容区尺寸和最大占比 → `App::resize`。图片/视频按真实分辨率（`native::media_size` 读系统属性）占 80%，PDF 按第一页比例占 90%，Office 1200×800 占 80%，Markdown 1000×600，CSV 1000×700，文本 800×600，压缩包 800×400，音频 500×300，文件夹/其他是 453×172 信息卡片；不放大小图，最小 300×200（原版 400 宽，窄长图两边会留白，用户要求贴合）。第一次显示居中，切换文件保持窗口中心。只有系统属性读不到尺寸的图片（如 SVG）才由 JS `fit()` 事后校正。
- 扩展名 → 预览类型的分类表只在 Rust `info::KINDS`，JS 用 `info.kind`，不要在 JS 里再维护一份。
- **图片滚轮直接缩放**（不需要 Ctrl），以鼠标位置为中心。
- Markdown 有左侧目录（≥2 个标题时显示）。
- **窗口无系统标题栏、无底部状态栏**，只有一条固定的白色顶栏（深色模式跟随系统）：最左文件名，右边图标按钮（`button.ic` + 内联 SVG，不写文字），按住空白处拖动窗口。OCR 按钮只在图片里显示。PDF 不显示工具栏/页码，docx 不显示页码。查找框、详细信息、识别结果是浮在内容右上角的卡片（`.pop`），不挤占预览区；按钮按下有缩放反馈，开关按钮打开时浅蓝底。详细信息和识别结果互斥，点原按钮或卡片外任意处关闭；查找框只能点按钮/×/Esc 关。识别文字只显示识别结果卡片，不进详细信息。
- 查找不用 `window.find`（输入框有焦点时找不到页面文字），自己遍历 `#view` 文本节点，用 CSS Custom Highlight 高亮全部/当前结果，显示 `当前/总数`（无结果 `0/0`）。PDF 是渲染出的图片，查不到字。
- **自动换行，不出横向滚动**（代码、文本、Markdown 都 `pre-wrap`；PDF 底部横向滚动条靠加高 iframe 裁掉）。
- 文档类（docx/pdf/pptx）按原字体显示；代码和纯文本照原版用 14px Consolas + 微软雅黑 UI（`<code>` 要 `font: inherit`，否则落到宋体）。
- Markdown 正文用 `vendor/github-markdown.css`（同原版），目录照原版 178px 浅灰底、树状缩进、当前章节左侧蓝条。
- `vendor/pptx-preview.umd.js` 打过补丁（搜 `Peek 补丁`）：图片 rot/flipH/flipV 是手工改的；其余 15 项由 `tools/patch_pptx.py` 打（多余的 Content_Types 部件不再让整份空白、单页出错不连坐、缺图跳过（含 svgBlip 回退）、按 sldIdLst 排页、无边框、母版非占位符形状、showMasterSp、两端对齐/默认左对齐、prstClr、母版 bodyStyle 继承、形状自带 lstStyle、项目符号大小颜色、图表照 Excel/WPS（坐标范围与非零起点、网格线、每点颜色、图例、标题字号）、SVG 的 MIME、渐变绕中心旋转）。脚本的输入是只含 rot/flip 的版本：`git show 8519eee:ui/vendor/pptx-preview.umd.js`，不要在已打过补丁的文件上再跑。升级该库：先补回 rot/flip，再跑脚本。
- `vendor/docx-preview.min.js` 打过补丁（`tools/patch_docx.py`，可重复跑，已打的跳过）：认 `firstLineChars/hangingChars/leftChars/rightChars`（中文 Word/WPS 常用，原库不认，编号段落的负首行缩进会伸出表格边框）；分节符按下一节 type 分页（原库只认 `lastRenderedPageBreak`，Word 会写、WPS 不写，WPS 文档整份并成一页），刚分过页时段首的 `lastRenderedPageBreak` 不再分（否则多出空白页）；四周型/紧密型环绕的锚定图按栏/段落偏移左浮动，整页高的（封面）不占高度并在其后分页；没写页眉页脚引用的节按类型沿用上一节。`renderDocx` 另把单元格里 `margin-left + text-indent < 0` 的段落夹回 0，并去掉目录（`toc N` 样式）里“超链接”字符样式的蓝色下划线（WPS/Word 显示黑字）；`section.docx` 用 `overflow: clip` 裁掉伸出纸面的图。仍不按高度分页：一节内容再长也是一张纸。
- 图片 OCR 用 PaddleOCR PP-OCRv6 tiny（`src/paddle.rs`，模型 `assets/ocr/det.onnx`、`rec.onnx` 共约 6 MB，来自 ModelScope `RapidAI/RapidOCR`，字表在 rec 模型元数据 `character` 里），用**系统自带的 Windows ML** 推理，不打包 onnxruntime。系统内置的 onnxruntime 1.17 只认 IR ≤ 9，模型文件第 2 个字节（ir_version）已从 10 改成 9，换模型要照做。安装后模型在 `{app}\ocr\`。每次识别加载模型、用完释放（空闲内存不变，识别时峰值约 +100～170 MB）。实测准确率 99.7%（Windows OCR 78.6%），耗时约 0.2～0.6 秒（Windows OCR 0.08～0.3 秒）。模型缺失或 Windows ML 报错时退回 Windows.Media.Ocr（小图先放大，试过加白边、灰度、英文引擎合并都无效）。`QuickView.exe --ocr <图片> <输出文件>` 单独测识别；`QUICKVIEW_OCR=paddle` 只走 Paddle 并写出错误，`=win` 强制系统 OCR。
- 开机自启时托盘可能建不起来（任务栏未就绪），托盘创建失败不能 `unwrap`，要重试；panic 写 `%LOCALAPPDATA%\QuickView\crash.log`。
- 已做的功能：Enter 打开、Ctrl+C 复制路径、窗口置顶、Ctrl+F 查找、MD5/SHA256、图片 OCR（PaddleOCR，退回 Windows.Media.Ocr）、文字可选中复制。

## 结构

- `src/native.rs`：低级键盘钩子（Explorer/桌面文件列表有焦点时拦空格；Peek 自己在前台时拦空格/Esc/↑/↓）、抢前台、自启（启动文件夹快捷方式）、系统缩略图（`/thumb`）、OCR、第二实例 WM_COPYDATA 传路径。
- `src/shell.rs`：IShellWindows 取当前标签页选中项、视图全部项、同步选中。
- `src/info.rs`：大小/时间格式化、哈希、旧版 Office 经 LibreOffice 转 PDF（缓存在 `%LOCALAPPDATA%\QuickView\cache`，`clean_cache` 按 30 天 / 300 MB 最近使用回收，残留 tmp 目录 10 分钟后删）。
- WebView2 用户数据在 `%LOCALAPPDATA%\QuickView\WebView2`：已加 `--disable-component-update` 关掉组件下载（DRM、广告过滤规则，约 57 MB），残留组件在**创建 WebView 之前**同步删除（WebView2 起来后会从 crx cache 装回并占住文件）。`with_additional_browser_args` 会替换 wry 的默认参数，默认的 `--disable-features=...` 要一并写上。
- `ui/app.js`：按扩展名分派渲染器；docx 用 docx-preview，xlsx/xls 用 SheetJS，pptx 用 pptx-preview，md 用 marked，代码用 highlight.js，PDF 用 Windows.Data.Pdf 按页渲染（`/pdfinfo`、`/pdfpage`，异步协议在后台线程处理）。
- 本地文件通过 `SetVirtualHostNameToFolderMapping` 映射成 `http://<盘符>.peekfs/...`。

## 打包与部署

- 安装包：`pwsh -File installer\build.ps1` → `dist\QuickView-<版本>-Setup.exe`（Inno Setup 6，脚本 `installer\quickview.iss`，须保持 UTF-8 BOM + CRLF，`.gitattributes` 已设 `-text`）。按用户装到 `%LOCALAPPDATA%\Programs\QuickView`，开始菜单有快捷方式；卸载结束进程树（`taskkill /t`，否则 WebView2 子进程占着数据目录删不掉）、删自启快捷方式和数据目录。
- 本机用的是**安装版**，改完要重新打包安装才生效（`/VERYSILENT` 静默装）；不再部署到 `E:\QuickLook\QuickView.exe`。自启快捷方式指向的 exe 不存在时，`ensure_autostart` 会改指到当前运行的 exe。
- 发版：改 `Cargo.toml` 版本号，打 tag `vX.Y.Z`，`gh release create` 附上安装包；安装包不进 git。

## 调试

- 页面就绪前的脚本在 `App::pending` 里**按顺序排队**：冷启动（开机后、闲置释放后第一次）时 `peek.show` 后面紧跟 `peek.position`，只留一条会把 show 覆盖、窗口空白。
- `QuickView.exe <路径>`：交给已运行实例预览；`QuickView.exe --selftest`：把所有资源管理器窗口的选中项写到 `%TEMP%\quickview-selftest.txt`。
- 带 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333` 启动可用 CDP 连页面。
- `PEEK_RELEASE_SECS` 环境变量可缩短 WebView2 释放时间。
- 页面里不能声明全局 `ipc`（与 wry 注入的 `window.ipc` 冲突），发消息用 `post()`。
