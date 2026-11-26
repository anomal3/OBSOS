<#
Сборка OBSOS на Windows одной командой:

    powershell -ExecutionPolicy Bypass -File tools\build-windows.ps1

Видео и звук (`<video>`, `<audio>`) — через GStreamer 1.26 (MSVC x86_64,
runtime и devel). Если он найден (переменная GSTREAMER_1_0_ROOT_MSVC_X86_64,
C:\gstreamer, F:\ObsosDeps или -GStreamerRoot), сборка идёт с ними и его DLL
копируются рядом с obsos.exe; иначе — без видео. `-NoVideo` — без него всегда.

`-Dist` — сборка для раздачи: в target-dist и без путей этого компьютера (в них
имя пользователя) внутри программы. Первый раз собирается всё заново.
#>
param(
    [string]$GStreamerRoot = '',
    [switch]$NoVideo,
    [switch]$Dist,
    [string]$Python = ''
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

$env:CC = 'clang-cl'
$env:CXX = 'clang-cl'
if (-not $Python) {
    $found = @($env:PYTHON3, 'C:\Python314\python.exe', 'C:\Python313\python.exe', 'C:\Python312\python.exe') | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
    if (-not $found) { $found = (Get-Command python -ErrorAction SilentlyContinue).Source }
    $Python = $found
}
if (-not $Python) { throw 'Не найден Python 3 (нужен Servo): укажите -Python <путь>' }
$env:PYTHON3 = $Python

$gst = $null
if (-not $NoVideo) {
    $candidates = @($GStreamerRoot, $env:GSTREAMER_1_0_ROOT_MSVC_X86_64, 'C:\gstreamer\1.0\msvc_x86_64',
        'F:\ObsosDeps\gst\PFiles64\gstreamer\1.0\msvc_x86_64', 'C:\Program Files\gstreamer\1.0\msvc_x86_64')
    $gst = $candidates | Where-Object { $_ -and (Test-Path (Join-Path $_ 'lib\pkgconfig\gstreamer-1.0.pc')) } | Select-Object -First 1
}

cargo xtask vendor
if ($LASTEXITCODE) { throw 'cargo xtask vendor не прошёл' }

$target = Join-Path $repo 'target'
if ($Dist) {
    $target = Join-Path $repo 'target-dist'
    $env:CARGO_TARGET_DIR = $target
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { "$env:USERPROFILE\.cargo" }
    $rustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { "$env:USERPROFILE\.rustup" }
    $fs = New-Object -ComObject Scripting.FileSystemObject
    # Кому что: длинный путь и короткий 8.3 (им пользуются сборки на C/C++).
    $maps = @()
    foreach ($pair in @(@($cargoHome, 'cargo'), @($rustupHome, 'rustup'), @($repo, 'obsos'))) {
        $maps += , @($pair[0], $pair[1])
        $short = $fs.GetFolder($pair[0]).ShortPath
        if ($short -ne $pair[0]) { $maps += , @($short, $pair[1]) }
    }
    $env:CARGO_ENCODED_RUSTFLAGS = ($maps | ForEach-Object { "--remap-path-prefix=$($_[0])=$($_[1])" }) -join [char]0x1f
    $c = ($maps | ForEach-Object { "/clang:-ffile-prefix-map=$($_[0])=$($_[1])" }) -join ' '
    $env:CFLAGS = $c
    $env:CXXFLAGS = $c
}

if ($gst) {
    $gst = $gst.TrimEnd('\')
    Write-Host "GStreamer: $gst"
    $env:GSTREAMER_1_0_ROOT_MSVC_X86_64 = "$gst\"
    $env:PKG_CONFIG_PATH = "$gst\lib\pkgconfig"
    $env:PKG_CONFIG = "$gst\bin\pkg-config.exe"
    $env:PATH = "$gst\bin;$env:PATH"
    cargo build --release --features video
} else {
    Write-Host 'Сборка без видео (GStreamer не найден или -NoVideo).'
    cargo build --release
}
if ($LASTEXITCODE) { throw 'сборка не прошла' }

$out = Join-Path $target 'release'
if ($gst) {
    # Servo ищет плагины GStreamer рядом с программой — по своему списку.
    $cargo = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { "$env:USERPROFILE\.cargo" }
    $lists = Get-ChildItem "$cargo\registry\src\*\servo-0.8.0\gstreamer_plugin_lists\*" -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -in 'common.rs.in', 'windows.rs.in' }
    $plugins = @(foreach ($list in $lists) { [regex]::Matches((Get-Content $list.FullName -Raw), '"([^"]+)"') | ForEach-Object { $_.Groups[1].Value } })
    if (-not $plugins) { throw 'Не прочитан список плагинов GStreamer из крейта servo-0.8.0' }
    $copied = 0
    foreach ($dll in Get-ChildItem "$gst\bin\*.dll") { Copy-Item $dll.FullName $out -Force; $copied++ }
    $missing = @()
    foreach ($name in $plugins | Sort-Object -Unique) {
        $path = "$gst\lib\gstreamer-1.0\$name.dll"
        if (Test-Path $path) { Copy-Item $path $out -Force; $copied++ } else { $missing += $name }
    }
    Write-Host "DLL GStreamer рядом с программой: $copied"
    if ($missing) { Write-Host "Плагинов из списка Servo нет в этом GStreamer: $($missing -join ', ')" }
}
Copy-Item (Join-Path $repo 'docs\NOTICE.md') $out -Force
Write-Host "Готово: $out\obsos.exe"
