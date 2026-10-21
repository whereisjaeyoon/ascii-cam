param([string]$RepoPath = "C:\Users\jkl27\ascii-cam")
$ErrorActionPreference = "Stop"
$repo = (Resolve-Path -LiteralPath $RepoPath).Path
if (!(Test-Path -LiteralPath (Join-Path $repo "controller-ui\Cargo.toml"))) { throw "Controller project not found under $repo" }
$destination = Join-Path $repo "installer"
if ([IO.Path]::GetFullPath($destination).TrimEnd('\') -eq [IO.Path]::GetFullPath($PSScriptRoot).TrimEnd('\')) {
    Write-Host "Installer sources are already in the project."
    exit 0
}
$relativeFiles = @("Cargo.toml", "Cargo.lock", "build.rs", "app.manifest", ".gitignore", "README.md", "build-installer.ps1", "Apply-Installer.ps1", "src\main.rs", "src\model.rs", "src\ui.rs", "src\pe.rs", "src\transaction.rs", "src\windows.rs")
foreach ($name in $relativeFiles) { if (!(Test-Path -LiteralPath (Join-Path $PSScriptRoot $name))) { throw "Package is incomplete: $name" } }
$backup = Join-Path (Split-Path $repo -Parent) ("ascii-cam-installer-backup-" + (Get-Date -Format "yyyyMMdd-HHmmss-fff"))
New-Item -ItemType Directory -Force -Path $backup | Out-Null
foreach ($name in $relativeFiles) {
    $old = Join-Path $destination $name
    if (Test-Path -LiteralPath $old) {
        $saved = Join-Path $backup $name
        New-Item -ItemType Directory -Force -Path (Split-Path $saved -Parent) | Out-Null
        Copy-Item -LiteralPath $old -Destination $saved
    }
}
foreach ($name in $relativeFiles) {
    $target = Join-Path $destination $name
    New-Item -ItemType Directory -Force -Path (Split-Path $target -Parent) | Out-Null
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination $target -Force
}
Write-Host "Graphical installer sources applied. Backup: $backup" -ForegroundColor Green
Write-Host "No DLL was installed and no registry settings were changed."
Write-Host "Next: build the distributable using installer\build-installer.ps1."
