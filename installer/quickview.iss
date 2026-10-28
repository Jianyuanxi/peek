; QuickView 安装包（Inno Setup 6）。用 installer\build.ps1 构建：它先 cargo build --release 再传入版本号。
;
; 按当前用户安装到 %LOCALAPPDATA%\Programs\QuickView，不需要管理员权限。
; 开机自启由 QuickView 自己在第一次运行时建（启动文件夹 QuickView.lnk），安装包不写启动项。
; 卸载：结束 QuickView，删自启快捷方式和它在“设置 > 启动应用”里的开关记录，删数据目录 %LOCALAPPDATA%\QuickView（缓存和 WebView2 数据）。

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef Exe
  #define Exe "..\target\release"
#endif

[Setup]
AppId={{6B0F3B8E-4C2A-4F1D-9E57-2C8D1A7B5E31}
AppName=QuickView
AppVersion={#AppVersion}
AppVerName=QuickView {#AppVersion}
AppPublisher=Jianyuanxi
AppPublisherURL=https://github.com/Jianyuanxi/quickview
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\QuickView
DefaultGroupName=QuickView
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir=..\dist
OutputBaseFilename=QuickView-{#AppVersion}-Setup
SetupIconFile=..\assets\app.ico
UninstallDisplayIcon={app}\QuickView.exe
UninstallDisplayName=QuickView
WizardStyle=modern
Compression=lzma2/ultra64
SolidCompression=yes
; 正在运行的 QuickView 在 PrepareToInstall 里结束
CloseApplications=no
RestartApplications=no

[Languages]
Name: "zh"; MessagesFile: "ChineseSimplified.isl"

[Messages]
RetryCancelSelectAction=请选择操作
RetryCancelRetry=重试(&T)
RetryCancelCancel=取消(&C)

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式"; GroupDescription: "快捷方式："; Flags: unchecked

[Files]
Source: "{#Exe}\quickview.exe"; DestDir: "{app}"; DestName: "QuickView.exe"; Flags: ignoreversion
Source: "..\assets\ocr\*.onnx"; DestDir: "{app}\ocr"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\QuickView"; Filename: "{app}\QuickView.exe"; Comment: "选中文件按空格预览"
Name: "{userdesktop}\QuickView"; Filename: "{app}\QuickView.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\QuickView.exe"; Description: "启动 QuickView"; Flags: postinstall nowait skipifsilent

[UninstallDelete]
Type: files; Name: "{userstartup}\QuickView.lnk"
Type: filesandordirs; Name: "{localappdata}\QuickView"
Type: dirifempty; Name: "{app}"

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\StartupFolder"; ValueName: "QuickView.lnk"; Flags: uninsdeletevalue dontcreatekey; ValueType: none

[Code]
procedure KillQuickView();
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/f /t /im QuickView.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  { WebView2 子进程跟着退出要一点时间，等它们放开数据目录 }
  Sleep(800);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  KillQuickView();
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    KillQuickView();
  { [UninstallDelete] 删数据目录时 WebView2 可能还没完全退出，最后再删一次 }
  if CurUninstallStep = usPostUninstall then
    DelTree(ExpandConstant('{localappdata}\QuickView'), True, True, True);
end;
