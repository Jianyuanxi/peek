# Peek — QuickLook 替代品

Windows 上资源管理器选中文件按空格预览。Rust（tao + wry/WebView2）单 exe，界面在 `ui/`，编译时用 `include_bytes!` 打进 exe。

## 用户定下的约束（不要回退）

- 开机自启、常驻，**空闲内存尽量小**：WebView2 按需创建，隐藏 `RELEASE_AFTER`（默认 300 秒）后释放。
- 空格出预览要快、不卡；Office 预览**速度优先**，清晰度可以比原版低，但要看得清内容（用户只是先看一眼，细看会用 WPS 打开）。
- 文件大小**自动换单位，1024 进制，保留两位小数**（如 `1.00 GB`，不要写成 `1024 MB`），不额外显示字节数。Rust `info::fmt_size` 和 JS `fmtSize` 两处保持一致。
- 文件夹大小、文件数走 Everything（`es.exe`），不自己遍历。
- **↑/↓ 切换上一个/下一个文件**（用户指定，不要改回 ←/→）。
- **窗口大小跟随内容比例**（图片/视频按真实尺寸，PPT 按 `sldSz`，文档类竖窗口），JS `fit()` → Rust `App::fit`，最大占工作区 88%，不放大小图。
- **图片滚轮直接缩放**（不需要 Ctrl），以鼠标位置为中心。
- Markdown 有左侧目录（≥2 个标题时显示）。
- 顶栏平时隐藏，鼠标移到窗口顶部才滑出；功能按钮只用图标（`button.ic` + 内联 SVG），不写文字。文件名在窗口标题上。
- 开机自启时托盘可能建不起来（任务栏未就绪），托盘创建失败不能 `unwrap`，要重试；panic 写 `%LOCALAPPDATA%\Peek\crash.log`。
- 已做的功能：Enter 打开、Ctrl+C 复制路径、窗口置顶、Ctrl+F 查找、MD5/SHA256、图片 OCR（Windows.Media.Ocr）、文字可选中复制。

## 结构

- `src/native.rs`：低级键盘钩子（只在 Explorer/桌面文件列表有焦点时拦空格）、抢前台、自启注册表、系统缩略图（`/thumb`）、OCR、第二实例 WM_COPYDATA 传路径。
- `src/shell.rs`：IShellWindows 取当前标签页选中项、视图全部项、同步选中。
- `src/info.rs`：大小/时间格式化、Everything 文件夹统计、哈希、旧版 Office 经 LibreOffice 转 PDF（缓存在 `%LOCALAPPDATA%\Peek\cache`）。
- `ui/app.js`：按扩展名分派渲染器；docx 用 docx-preview，xlsx/xls 用 SheetJS，pptx 用 pptx-preview，md 用 marked，代码用 highlight.js，PDF 用 WebView2 内置查看器。
- 本地文件通过 `SetVirtualHostNameToFolderMapping` 映射成 `http://<盘符>.peekfs/...`。

## 调试

- `peek.exe <路径>`：交给已运行实例预览；`peek.exe --selftest`：把所有资源管理器窗口的选中项写到 `%TEMP%\peek-selftest.txt`。
- 带 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333` 启动可用 CDP 连页面。
- `PEEK_RELEASE_SECS` 环境变量可缩短 WebView2 释放时间。
- 页面里不能声明全局 `ipc`（与 wry 注入的 `window.ipc` 冲突），发消息用 `post()`。
