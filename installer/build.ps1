# 构建安装包：cargo build --release，再用 Inno Setup 打包到 dist\QuickView-<版本>-Setup.exe。
# 用法：powershell -File installer\build.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'cargo build --release 失败' }

$iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe") |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw '找不到 Inno Setup 6（winget install JRSoftware.InnoSetup）' }
& $iscc /Qp "/DAppVersion=$version" "/DExe=$root\target\release" installer\quickview.iss
if ($LASTEXITCODE -ne 0) { throw 'ISCC 打包失败' }
Get-Item "dist\QuickView-$version-Setup.exe"
