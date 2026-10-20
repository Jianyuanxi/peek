# Peek — QuickLook 替代品

Windows 上资源管理器选中文件按空格预览。Rust（tao + wry/WebView2）单 exe，界面在 `ui/`，编译时用 `include_bytes!` 打进 exe。

## 用户定下的约束（不要回退）

- 开机自启、常驻，**空闲内存尽量小**：WebView2 按需创建，隐藏 `RELEASE_AFTER`（默认 300 秒）后释放。
- 空格出预览要快、不卡；Office 预览**速度优先**，清晰度可以比原版低，但要看得清内容（用户只是先看一眼，细看会用 WPS 打开）。
- 文件大小**自动换单位，1024 进制，保留两位小数**（如 `1.00 GB`，不要写成 `1024 MB`），不额外显示字节数，只在详细信息面板里显示。Rust `info::fmt_size` 和 JS `fmtSize` 两处保持一致。
- **不做文件夹大小/文件数统计**（用户嫌慢且没用，已删），不要加回来。
- **↑/↓ 切换上一个/下一个文件**（用户指定，不要改回 ←/→）。
- **空格/Esc 关闭、↑/↓ 切换在任何格式里都要生效**：PDF 查看器、视频控件拿到焦点后页面收不到按键，所以 Peek 窗口在前台时由低级键盘钩子直接拦截（`native::kb_proc`）；查找框聚焦时（`TYPING`）放行。
- **窗口大小照原版 QuickLook**，并且**在显示内容之前就调好**（先显示再调会先大后小闪一下）：Rust `info::preferred_size` 按类型给内容区尺寸和最大占比 → `App::resize`。图片/视频按真实分辨率（`native::media_size` 读系统属性）占 80%，PDF 按第一页比例占 90%，Office 1200×800 占 80%，Markdown 1000×600，CSV 1000×700，文本 800×600，压缩包 800×400，音频 500×300，文件夹/其他是 453×172 信息卡片；不放大小图，最小 400×200。第一次显示居中，切换文件保持窗口中心。只有系统属性读不到尺寸的图片（如 SVG）才由 JS `fit()` 事后校正。
- 扩展名 → 预览类型的分类表只在 Rust `info::KINDS`，JS 用 `info.kind`，不要在 JS 里再维护一份。
- **图片滚轮直接缩放**（不需要 Ctrl），以鼠标位置为中心。
- Markdown 有左侧目录（≥2 个标题时显示）。
- **窗口无系统标题栏、无底部状态栏**，只有一条固定的深色顶栏：最左文件名，右边图标按钮（`button.ic` + 内联 SVG，不写文字），按住空白处拖动窗口。OCR 按钮只在图片里显示。PDF 不显示工具栏/页码，docx 不显示页码。
- **自动换行，不出横向滚动**（代码、文本、Markdown 都 `pre-wrap`；PDF 底部横向滚动条靠加高 iframe 裁掉）。
- 文档类（docx/pdf/pptx）按原字体显示；代码和纯文本照原版用 14px Consolas + 微软雅黑 UI（`<code>` 要 `font: inherit`，否则落到宋体）。
- Markdown 正文用 `vendor/github-markdown.css`（同原版），目录照原版 178px 浅灰底、树状缩进、当前章节左侧蓝条。
- `vendor/pptx-preview.umd.js` 打过补丁（搜 `Peek 补丁`）：原库不处理图片的 rot/flipH/flipV。升级该库要重新打。
- 开机自启时托盘可能建不起来（任务栏未就绪），托盘创建失败不能 `unwrap`，要重试；panic 写 `%LOCALAPPDATA%\Peek\crash.log`。
- 已做的功能：Enter 打开、Ctrl+C 复制路径、窗口置顶、Ctrl+F 查找、MD5/SHA256、图片 OCR（Windows.Media.Ocr）、文字可选中复制。

## 结构

- `src/native.rs`：低级键盘钩子（Explorer/桌面文件列表有焦点时拦空格；Peek 自己在前台时拦空格/Esc/↑/↓）、抢前台、自启注册表、系统缩略图（`/thumb`）、OCR、第二实例 WM_COPYDATA 传路径。
- `src/shell.rs`：IShellWindows 取当前标签页选中项、视图全部项、同步选中。
- `src/info.rs`：大小/时间格式化、哈希、旧版 Office 经 LibreOffice 转 PDF（缓存在 `%LOCALAPPDATA%\Peek\cache`）。
- `ui/app.js`：按扩展名分派渲染器；docx 用 docx-preview，xlsx/xls 用 SheetJS，pptx 用 pptx-preview，md 用 marked，代码用 highlight.js，PDF 用 WebView2 内置查看器。
- 本地文件通过 `SetVirtualHostNameToFolderMapping` 映射成 `http://<盘符>.peekfs/...`。

## 调试

- `peek.exe <路径>`：交给已运行实例预览；`peek.exe --selftest`：把所有资源管理器窗口的选中项写到 `%TEMP%\peek-selftest.txt`。
- 带 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333` 启动可用 CDP 连页面。
- `PEEK_RELEASE_SECS` 环境变量可缩短 WebView2 释放时间。
- 页面里不能声明全局 `ipc`（与 wry 注入的 `window.ipc` 冲突），发消息用 `post()`。
