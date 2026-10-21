param(
    [string]$RepoPath = (Split-Path $PSScriptRoot -Parent),
    [string]$CameraDll,
    [string]$CameraHost,
    [string]$RuntimeDirectory,
    [string]$OpenCvBin = "C:\opencv\build\x64\vc16\bin",
    [switch]$SkipNativeBuild
)
$ErrorActionPreference = "Stop"
$repo = (Resolve-Path -LiteralPath $RepoPath).Path
$installer = $PSScriptRoot
$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
$vs = $null
if (Test-Path -LiteralPath $vswhere) {
    $vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath | Select-Object -First 1
}
function Run-Checked([string]$Command, [string[]]$Arguments) {
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Command failed (exit $LASTEXITCODE). Packaging stopped." }
}
if (!$SkipNativeBuild) {
    if (!$vs) { throw "Visual Studio C++ tools not found. Open Visual Studio Installer and install Desktop development with C++." }
    $msbuild = Join-Path $vs "MSBuild\Current\Bin\amd64\MSBuild.exe"
    if (!(Test-Path -LiteralPath $msbuild)) { $msbuild = Join-Path $vs "MSBuild\Current\Bin\MSBuild.exe" }
    Write-Host "Building camera DLL (Release x64)..."
    Run-Checked $msbuild @((Join-Path $repo "ascii-cam-source\ascii-cam.sln"), "/m", "/t:Build", "/p:Configuration=Release", "/p:Platform=x64", "/nologo")
    Write-Host "Building camera host (Release x64)..."
    Run-Checked $msbuild @((Join-Path $repo "ascii-cam\ascii-cam\ascii-cam.sln"), "/m", "/t:Build", "/p:Configuration=Release", "/p:Platform=x64", "/nologo")
}
if (!$CameraDll) { $CameraDll = Join-Path $repo "ascii-cam-source\x64\Release\ascii-cam.dll" }
if (!$CameraHost) { $CameraHost = Join-Path $repo "ascii-cam\ascii-cam\x64\Release\ascii-cam.exe" }
foreach ($file in @($CameraDll, $CameraHost)) {
    if (!(Test-Path -LiteralPath $file -PathType Leaf)) { throw "Required binary missing: $file. Build Release x64 or pass -CameraDll / -CameraHost." }
}
Write-Host "Building the controller client..."
Run-Checked "cargo" @("build", "--locked", "--release", "--target", "x86_64-pc-windows-msvc", "--target-dir", (Join-Path $repo "controller-ui\target"), "--manifest-path", (Join-Path $repo "controller-ui\Cargo.toml"))
$client = Join-Path $repo "controller-ui\target\x86_64-pc-windows-msvc\release\ascii-cam-ui.exe"
if (!(Test-Path -LiteralPath $client)) { throw "Controller output not found at $client. Check any custom CARGO_TARGET_DIR setting." }

# Include app-local x64 CRT libraries, so the target PC does not need Visual Studio.
if (!$RuntimeDirectory -and $env:VCToolsRedistDir) {
    $match = Get-ChildItem -LiteralPath (Join-Path $env:VCToolsRedistDir "x64") -Directory -ErrorAction SilentlyContinue | Where-Object Name -Like 'Microsoft.VC*.CRT' | Select-Object -First 1
    if ($match) { $RuntimeDirectory = $match.FullName }
}
if (!$RuntimeDirectory -and $vs) {
    $redist = Join-Path $vs "VC\Redist\MSVC"
    foreach ($version in @(Get-ChildItem -LiteralPath $redist -Directory -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending)) {
        $match = Get-ChildItem -LiteralPath (Join-Path $version.FullName "x64") -Directory -ErrorAction SilentlyContinue | Where-Object Name -Like 'Microsoft.VC*.CRT' | Select-Object -First 1
        if ($match) { $RuntimeDirectory = $match.FullName; break }
    }
}
if (!$RuntimeDirectory -or !(Test-Path -LiteralPath (Join-Path $RuntimeDirectory "vcruntime140.dll"))) {
    throw "x64 VC++ runtime folder not found. Pass -RuntimeDirectory pointing to Microsoft.VC*.CRT under Visual Studio VC\Redist\MSVC."
}
$dumpbinCommand = Get-Command dumpbin.exe -ErrorAction SilentlyContinue
$dumpbin = if ($dumpbinCommand) { $dumpbinCommand.Source } else { $null }
if (!$dumpbin -and $vs) {
    foreach ($version in @(Get-ChildItem -LiteralPath (Join-Path $vs "VC\Tools\MSVC") -Directory | Sort-Object LastWriteTime -Descending)) {
        $candidate = Join-Path $version.FullName "bin\Hostx64\x64\dumpbin.exe"
        if (Test-Path -LiteralPath $candidate) { $dumpbin = $candidate; break }
    }
}
if (!$dumpbin) { throw "dumpbin.exe not found. Run this script from Developer PowerShell for Visual Studio." }
$stage = Join-Path $installer ("payload-stage-" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $stage | Out-Null
$previousPayload = $env:ASCII_CAM_PAYLOAD_DIR
$previousRustFlags = $env:RUSTFLAGS
$previousEncodedFlags = $env:CARGO_ENCODED_RUSTFLAGS
try {
    Copy-Item -LiteralPath $client -Destination (Join-Path $stage "ascii-cam-ui.exe")
    Copy-Item -LiteralPath $CameraHost -Destination (Join-Path $stage "ascii-cam-host.exe")
    Copy-Item -LiteralPath $CameraDll -Destination (Join-Path $stage "ascii-cam.dll")
    Get-ChildItem -LiteralPath $RuntimeDirectory -Filter '*.dll' -File | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $stage }
    function Dependencies([string]$File) {
        $output = & $dumpbin /nologo /dependents $File
        if ($LASTEXITCODE -ne 0) { throw "Cannot inspect dependencies: $File" }
        foreach ($line in $output) { if ($line -match '^\s+([A-Za-z0-9_.-]+\.dll)\s*$') { $Matches[1].ToLowerInvariant() } }
    }
    $needsOpenCv = $false
    foreach ($file in @(Get-ChildItem -LiteralPath $stage -File)) {
        if (@(Dependencies $file.FullName | Where-Object { $_ -like 'opencv_world*.dll' }).Count -gt 0) { $needsOpenCv = $true }
    }
    if ($needsOpenCv) {
        if (!(Test-Path -LiteralPath $OpenCvBin)) { throw "OpenCV is imported, but $OpenCvBin does not exist. Pass -OpenCvBin." }
        Get-ChildItem -LiteralPath $OpenCvBin -Filter '*.dll' -File | Where-Object { $_.Name -match '^opencv_(world|videoio_)' -and $_.BaseName -notmatch 'd$' } | ForEach-Object { Copy-Item -LiteralPath $_.FullName -Destination $stage }
    }
    foreach ($file in @(Get-ChildItem -LiteralPath $stage -File)) {
        $headers = & $dumpbin /nologo /headers $file.FullName
        if ($LASTEXITCODE -ne 0 -or !($headers -match '8664 machine')) { throw "Expected x64 PE binary: $($file.Name)" }
        foreach ($dep in @(Dependencies $file.FullName)) {
            if (Test-Path -LiteralPath (Join-Path $stage $dep)) { continue }
            if ($dep -match '^(vcruntime|msvcp|concrt|opencv_)') { throw "Runtime missing from package: $dep" }
            if ($dep -like 'api-ms-*' -or $dep -like 'ext-ms-*') { continue }
            if (!(Test-Path -LiteralPath (Join-Path "$env:SystemRoot\System32" $dep))) { throw "Unresolved dependency $dep imported by $($file.Name)" }
        }
    }
    $exports = & $dumpbin /nologo /exports (Join-Path $stage "ascii-cam.dll")
    if ($LASTEXITCODE -ne 0) { throw "Could not inspect camera DLL exports." }
    foreach ($symbol in @("DllCanUnloadNow", "DllGetClassObject", "DllRegisterServer", "DllUnregisterServer")) {
        if (!(($exports -join "`n") -match ("\b" + $symbol + "\b"))) {
            throw "Camera DLL export missing: $symbol. Configure ascii-cam.def and rebuild Release x64."
        }
    }
    $env:ASCII_CAM_PAYLOAD_DIR = $stage
    # Bootstrap must run before any app-local runtimes have been extracted.
    # Link the installer CRT statically; clients use the bundled CRT DLLs.
    $env:RUSTFLAGS = "$previousRustFlags -C target-feature=+crt-static"
    Remove-Item Env:CARGO_ENCODED_RUSTFLAGS -ErrorAction SilentlyContinue
    Write-Host "Building the single-file graphical installer..."
    Run-Checked "cargo" @("build", "--locked", "--release", "--target", "x86_64-pc-windows-msvc", "--features", "bundled", "--target-dir", (Join-Path $installer "target"), "--manifest-path", (Join-Path $installer "Cargo.toml"))
    $setup = Join-Path $installer "target\x86_64-pc-windows-msvc\release\ascii-cam-setup.exe"
    foreach ($dependency in @(Dependencies $setup)) {
        if ($dependency -match '^(vcruntime|msvcp|concrt|ucrtbase)') {
            throw "Setup still imports a dynamic CRT ($dependency). Check custom Rust linker settings."
        }
    }
    # Verify the linked resource, not just the source manifest.
    $mtCommand = Get-Command mt.exe -ErrorAction SilentlyContinue
    $mt = if ($mtCommand) { $mtCommand.Source } else { $null }
    if (!$mt) {
        $kitRoot = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows Kits\Installed Roots' -ErrorAction SilentlyContinue).KitsRoot10
        if ($kitRoot) {
            foreach ($sdk in @(Get-ChildItem -LiteralPath (Join-Path $kitRoot 'bin') -Directory | Sort-Object Name -Descending)) {
                $candidate = Join-Path $sdk.FullName 'x64\mt.exe'
                if (Test-Path -LiteralPath $candidate) { $mt = $candidate; break }
            }
        }
    }
    if (!$mt) { throw 'Windows SDK mt.exe not found; cannot verify the administrator manifest.' }
    $linkedManifest = Join-Path $stage 'setup.manifest'
    Run-Checked $mt @('-nologo', "-inputresource:$setup;#1", "-out:$linkedManifest")
    [xml]$linkedXml = Get-Content -Raw -LiteralPath $linkedManifest
    $level = $linkedXml.SelectSingleNode("//*[local-name()='requestedExecutionLevel']")
    if (!$level -or $level.GetAttribute('level') -ne 'requireAdministrator') {
        throw 'The linked EXE is missing its requireAdministrator manifest.'
    }
    $dist = Join-Path $installer "dist"
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    Copy-Item -LiteralPath $setup -Destination (Join-Path $dist "ASCII-Cam-Setup.exe") -Force
    Write-Host ""
    Write-Host "Ready: $(Join-Path $dist 'ASCII-Cam-Setup.exe')" -ForegroundColor Green
    Write-Host "Share this ONE executable. It embeds the client, camera host, DLL and runtime libraries."
} finally {
    $env:ASCII_CAM_PAYLOAD_DIR = $previousPayload
    $env:RUSTFLAGS = $previousRustFlags
    $env:CARGO_ENCODED_RUSTFLAGS = $previousEncodedFlags
    # Only our GUID-named temporary staging directory is removed.
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
}
