# QuickView

Windows 上的空格预览：在资源管理器或桌面选中文件，按空格立刻看内容，再按空格关闭。思路和 [QuickLook](https://github.com/QL-Win/QuickLook) 一样，重写的目标是**快**和**常驻时占用小**。

Rust（tao + wry / WebView2）写成的单个 exe，界面文件编译时打包进去，不需要安装。

## 功能

- **空格** 预览 / 关闭，**Esc** 关闭，**↑ / ↓** 切换上一个 / 下一个文件，**Enter** 用默认程序打开
- **Ctrl+F** 查找并高亮，**Ctrl+C** 复制路径，窗口置顶，MD5 / SHA256，图片文字识别（系统自带 OCR）
- 窗口大小按内容自动算好再显示（图片、视频按真实分辨率，PDF 按页面比例），不会先大后小闪一下
- 图片滚轮直接缩放；Markdown 带目录；代码高亮、自动换行
- 跟随系统深色模式；开机自启，常驻托盘；预览窗口闲置 5 分钟后释放 WebView2，常驻内存很小

## 支持的格式

| 类型 | 扩展名 |
|---|---|
| 图片 | jpg png gif webp bmp ico svg avif 等；heic tif psd 及各家相机 raw 用系统缩略图 |
| 视频 / 音频 | mp4 webm mov mkv ts 等 / mp3 wav flac ogg m4a aac 等 |
| PDF | 用系统 Windows.Data.Pdf 按页渲染，大文件也能 0.1–0.5 秒出第一页 |
| Office | docx、xlsx / xls / ods / et、pptx；旧格式 doc ppt wps rtf odt 等需要装 LibreOffice（自动转 PDF 并缓存） |
| 文本 | Markdown、HTML、CSV / TSV、代码与纯文本（按内容自动识别） |
| 其他 | zip / jar / apk / epub 等压缩包列出内容，字体文件显示字样，文件夹和其他文件显示信息卡片 |

## 安装

从 [Releases](https://github.com/Jianyuanxi/quickview/releases) 下载 `QuickView-<版本>-Setup.exe` 运行即可：按当前用户安装到 `%LOCALAPPDATA%\Programs\QuickView`，不需要管理员权限，开始菜单里有 QuickView。第一次运行会在“启动”文件夹建快捷方式，之后开机自动运行；托盘菜单里可以关掉开机自启。卸载在“设置 > 应用”里，会一并删掉自启项和数据目录。

## 使用

在资源管理器或桌面选中文件，按空格。程序没在运行时从开始菜单打开 QuickView。

命令行：`QuickView.exe <路径>` 把文件交给已运行的实例预览。

数据目录在 `%LOCALAPPDATA%\QuickView`（转换缓存、WebView2 数据、崩溃日志）。

## 编译

需要 Rust 稳定版（MSVC 工具链）和系统自带的 WebView2 运行时。

```bash
cargo build --release
```

产物是 `target/release/quickview.exe`。打安装包需要 [Inno Setup 6](https://jrsoftware.org/isinfo.php)：

```bash
powershell -File installer/build.ps1
```

生成 `dist/QuickView-<版本>-Setup.exe`。

## 第三方库

界面用到 [docx-preview](https://github.com/VolodymyrBaydalka/docxjs)、[SheetJS](https://sheetjs.com/)、[pptx-preview](https://www.npmjs.com/package/pptx-preview)（打过图片旋转/翻转补丁）、[JSZip](https://stuk.github.io/jszip/)、[marked](https://github.com/markedjs/marked)、[highlight.js](https://highlightjs.org/)、[github-markdown-css](https://github.com/sindresorhus/github-markdown-css)，放在 `ui/vendor/`。
