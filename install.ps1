# Strata Universal Installer for Windows
# Usage: irm https://raw.githubusercontent.com/phfarath/Strata/main/install.ps1 | iex

$ErrorActionPreference = "Stop"

$repo = "phfarath/Strata"
$target = "x86_64-pc-windows-msvc"

Write-Host ""
Write-Host "  ___ _             _        " -ForegroundColor Cyan
Write-Host " / __| |_ _ _ __ _| |_ __ _ " -ForegroundColor Cyan
Write-Host " \__ \  _| '_/ _` |  _/ _` |" -ForegroundColor Cyan
Write-Host " |___/\__|_| \__,_|\__\__,_|" -ForegroundColor Cyan
Write-Host "  Local-First Persistent Memory Engine for AI Coding Agents"
Write-Host ""

# 1. Fetch latest release
Write-Host "Fetching latest release from GitHub..." -ForegroundColor Gray
$tag = "v0.1.0"
try {
    $releaseUrl = "https://api.github.com/repos/$repo/releases/latest"
    $response = Invoke-RestMethod -Uri $releaseUrl -UseBasicParsing -Headers @{ "User-Agent" = "Strata-Installer" }
    if ($response.tag_name) {
        $tag = $response.tag_name
    }
} catch {
    Write-Host "Using default release tag: $tag" -ForegroundColor Yellow
}

Write-Host "Installing version: $tag" -ForegroundColor Green

# 2. Download and Extract
$pkgName = "strata-$tag-$target"
$downloadUrl = "https://github.com/$repo/releases/download/$tag/$pkgName.zip"
$tempZip = Join-Path $env:TEMP "$pkgName.zip"
$tempExtract = Join-Path $env:TEMP "$pkgName"

Write-Host "Downloading $downloadUrl..." -ForegroundColor Cyan
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $downloadUrl -OutFile $tempZip -UseBasicParsing
} catch {
    Write-Host "Error downloading release binary. You can build from source via: cargo install strata-cli" -ForegroundColor Red
    exit 1
}

Write-Host "Extracting archive..." -ForegroundColor Gray
if (Test-Path $tempExtract) {
    Remove-Item -Path $tempExtract -Recurse -Force
}
Expand-Archive -Path $tempZip -DestinationPath $tempExtract -Force

# 3. Install binary to LocalAppData
$installDir = Join-Path $env:LOCALAPPDATA "Strata\bin"
if (!(Test-Path $installDir)) {
    New-Item -ItemType Directory -Force -Path $installDir | Out-Null
}

$sourceExe = Join-Path $tempExtract "$pkgName\strata.exe"
if (!(Test-Path $sourceExe)) {
    $sourceExe = Join-Path $tempExtract "strata.exe"
}

Copy-Item -Path $sourceExe -Destination (Join-Path $installDir "strata.exe") -Force

# Cleanup temp files
Remove-Item -Path $tempZip -Force -ErrorAction SilentlyContinue
Remove-Item -Path $tempExtract -Recurse -Force -ErrorAction SilentlyContinue

Write-Host ""
Write-Host "✓ Strata installed successfully to: $installDir\strata.exe" -ForegroundColor Green

# 4. Check and add to User PATH
$userPath = [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::User)
if ($userPath -split ';' -notcontains $installDir) {
    Write-Host "Adding $installDir to User PATH..." -ForegroundColor Cyan
    $newPath = "$userPath;$installDir"
    [Environment]::SetEnvironmentVariable("Path", $newPath, [EnvironmentVariableTarget]::User)
    $env:Path = "$env:Path;$installDir"
    Write-Host "✓ Added to User PATH (restart active terminals to take effect globally)." -ForegroundColor Green
}

Write-Host ""
Write-Host "Quickstart:" -ForegroundColor White
Write-Host "  strata --version"
Write-Host "  strata mcp install    # Configure Cursor, Claude Desktop, and Windsurf"
Write-Host "  strata init           # Initialize in current project"
Write-Host ""
