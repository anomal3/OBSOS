<#
Архив для выпуска OBSOS под Windows x64 из готовой сборки с видео
(`tools\build-windows.ps1 -Dist`):

    powershell -ExecutionPolicy Bypass -File tools\package-windows.ps1

В архив идут `obsos.exe`, плагины GStreamer по списку Servo и только те DLL,
от которых они зависят (не вся папка GStreamer: там есть и GPL-библиотеки,
например x264), библиотеки Visual C++ и тексты лицензий. Нужен llvm-readobj
(ставится вместе с clang).
#>
param(
    [string]$GStreamerRoot = '',
    [string]$Out = 'dist'
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo
$rel = Join-Path $repo 'target-dist\release'
if (-not (Test-Path "$rel\obsos.exe")) { throw 'Сначала соберите: tools\build-windows.ps1 -Dist' }

$readobj = (Get-Command llvm-readobj -ErrorAction SilentlyContinue).Source
if (-not $readobj) { $readobj = 'C:\Program Files\LLVM\bin\llvm-readobj.exe' }
if (-not (Test-Path $readobj)) { throw 'Не найден llvm-readobj' }

$gst = @($GStreamerRoot, $env:GSTREAMER_1_0_ROOT_MSVC_X86_64, 'C:\gstreamer\1.0\msvc_x86_64',
    'F:\ObsosDeps\gst\PFiles64\gstreamer\1.0\msvc_x86_64', 'C:\Program Files\gstreamer\1.0\msvc_x86_64') |
    Where-Object { $_ -and (Test-Path (Join-Path $_ 'bin')) } | Select-Object -First 1
if (-not $gst) { throw 'Не найден GStreamer: укажите -GStreamerRoot' }

$crt = Get-ChildItem 'C:\Program Files*\Microsoft Visual Studio\*\*\VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT' -Directory -ErrorAction SilentlyContinue |
    Sort-Object FullName | Select-Object -Last 1
if (-not $crt) { throw 'Не найдена папка библиотек Visual C++ (VC\Redist)' }

$version = [regex]::Match((Get-Content "$repo\Cargo.toml" -Raw), '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
$name = "OBSOS-$version-windows-x64"
$stage = Join-Path (Join-Path $repo $Out) $name
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force $stage | Out-Null

# Плагины, которые Servo ищет рядом с программой.
$cargo = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { "$env:USERPROFILE\.cargo" }
$plugins = @(Get-ChildItem "$cargo\registry\src\*\servo-0.8.0\gstreamer_plugin_lists\*" |
    Where-Object { $_.Name -in 'common.rs.in', 'windows.rs.in' } |
    ForEach-Object { [regex]::Matches((Get-Content $_.FullName -Raw), '"([^"]+)"') | ForEach-Object { "$($_.Groups[1].Value).dll" } } |
    Sort-Object -Unique)
$roots = @('obsos.exe') + @($plugins | Where-Object { Test-Path "$rel\$_" })
if ($roots.Count -lt 5) { throw 'Рядом с obsos.exe нет плагинов GStreamer — соберите с видео' }

# Замыкание зависимостей: что импортируют программа, плагины и их библиотеки.
$taken = @{}
$queue = New-Object System.Collections.Queue
foreach ($file in $roots) { $queue.Enqueue((Join-Path $rel $file)) }
while ($queue.Count) {
    $path = $queue.Dequeue()
    $key = (Split-Path $path -Leaf).ToLowerInvariant()
    if ($taken.ContainsKey($key)) { continue }
    $taken[$key] = $path
    $imports = & $readobj --coff-imports $path | Select-String '^\s*Name:\s*(\S+\.dll)\s*$' | ForEach-Object { $_.Matches[0].Groups[1].Value }
    foreach ($dll in $imports) {
        # Своё — рядом с программой; библиотеки Visual C++ — из Redist;
        # остальное даёт Windows.
        foreach ($dir in $rel, $crt.FullName) {
            if (Test-Path (Join-Path $dir $dll)) { $queue.Enqueue((Join-Path $dir $dll)); break }
        }
    }
}

# Библиотеки под GPL с программой не раздаём.
$gpl = @($taken.Keys | Where-Object { $_ -match '^(lib)?(x264|x265|a52|mpeg2|dvdread|dvdnav|cdio)' -or $_ -match '^gst(x264|x265|a52dec|mpeg2dec)' })
if ($gpl) { throw "В зависимостях есть GPL-библиотеки: $($gpl -join ', ')" }

foreach ($path in $taken.Values) { Copy-Item $path $stage }

# В раздаваемых файлах не должно быть путей из профиля пользователя.
$mine = "[A-Za-z]:[\\/]Users[\\/]$([regex]::Escape($env:USERNAME))[\\/]"
foreach ($file in Get-ChildItem $stage -File) {
    $text = [Text.Encoding]::GetEncoding(28591).GetString([IO.File]::ReadAllBytes($file.FullName))
    if ($text -match $mine) { throw "$($file.Name): внутри путь из профиля пользователя — соберите с -Dist" }
}
Copy-Item "$repo\docs\NOTICE.md", "$repo\LICENSE-MIT", "$repo\LICENSE-APACHE" $stage
$licenses = Join-Path $stage 'licenses'
New-Item -ItemType Directory -Force $licenses | Out-Null
$skip = 'x264', 'x265', 'x265-src', 'a52dec', 'pkg-config', 'pycairo', 'pygobject', 'gst-python-1.0', 'gst-shell', 'vsintegration-1.0', 'dump-versions', 'qrencode', 'ladspa'
Get-ChildItem "$gst\share\licenses" -Directory -ErrorAction SilentlyContinue | Where-Object { $_.Name -notin $skip } |
    ForEach-Object { Copy-Item $_.FullName $licenses -Recurse }

@"
OBSOS $version — браузер на движке Servo (Windows x64, ранняя версия)

Запуск: obsos.exe (установка не нужна, распакуйте архив целиком).
Нужна Windows 10 или 11 и OpenGL 3.2; если окно не открывается или рисует
неверно: obsos.exe --swgl (рисование процессором).

Профиль и настройки: %USERPROFILE%\.config\obsos
Журнал:              %USERPROFILE%\.config\obsos\obsos.log

Чего нет: потокового видео MSE (YouTube), WebGL. Движок Servo молодой —
часть сайтов выглядит или работает не так, как в Chrome и Firefox.

Лицензии: свой код OBSOS — MIT или Apache-2.0; сторонние компоненты
(Servo, GStreamer под LGPL и другие) — NOTICE.md и папка licenses.
Исходники: https://github.com/anomal3/OBSOS
GStreamer не изменялся: https://gstreamer.freedesktop.org (сборка MSVC).
"@ | Set-Content -Encoding UTF8 (Join-Path $stage 'README.txt')

$zip = Join-Path (Join-Path $repo $Out) "$name.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path $stage -DestinationPath $zip -CompressionLevel Optimal
$size = (Get-Item $zip).Length / 1MB
$files = (Get-ChildItem $stage -File).Count
Write-Host ("Готово: {0} ({1:N0} МБ, файлов в корне {2}, из них DLL {3})" -f $zip, $size, $files, @(Get-ChildItem $stage -Filter *.dll).Count)
