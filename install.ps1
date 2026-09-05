# Strata Universal Installer for Windows
# Usage: irm https://raw.githubusercontent.com/phfarath/Strata/main/install.ps1 | iex

$ErrorActionPreference = "Stop"

$repo = "phfarath/Strata"

Write-Host ""
Write-Host "  ___ _             _        " -ForegroundColor Cyan
Write-Host " / __| |_ _ _ __ _| |_ __ _ " -ForegroundColor Cyan
Write-Host " \__ \  _| '_/ _` |  _/ _` |" -ForegroundColor Cyan
Write-Host " |___/\__|_| \__,_|\__\__,_|" -ForegroundColor Cyan
Write-Host "  Local-First Persistent Memory Engine for AI Coding Agents"
Write-Host ""

# 1. Detect Architecture (x64 vs arm64)
$rawArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLower()
if ($rawArch -like "*arm64*") {
    Write-Host "Detected architecture: ARM64" -ForegroundColor Gray
    # Releases currently package x86_64-pc-windows-msvc which runs smoothly via Windows Prism emulation
    $target = "x86_64-pc-windows-msvc"
} elseif ($rawArch -like "*x64*" -or $rawArch -like "*amd64*") {
    Write-Host "Detected architecture: x64" -ForegroundColor Gray
    $target = "x86_64-pc-windows-msvc"
} else {
    Write-Host "Detected architecture: $rawArch (defaulting to x86_64)" -ForegroundColor Yellow
    $target = "x86_64-pc-windows-msvc"
}

# 2. Fetch latest release tag from GitHub API
Write-Host "Fetching latest release tag from GitHub..." -ForegroundColor Gray
$tag = "v0.1.1"
try {
    $releaseUrl = "https://api.github.com/repos/$repo/releases/latest"
    $response = Invoke-RestMethod -Uri $releaseUrl -UseBasicParsing -Headers @{ "User-Agent" = "Strata-Installer" }
    if ($response.tag_name) {
        $tag = $response.tag_name
    }
} catch {
    Write-Host "Could not fetch latest tag via GitHub API. Using fallback release tag: $tag" -ForegroundColor Yellow
}

Write-Host "Installing Strata version: $tag ($target)" -ForegroundColor Green

# 3. Download and Extract
$pkgName = "strata-$tag-$target"
$downloadUrl = "https://github.com/$repo/releases/download/$tag/$pkgName.zip"
$tempZip = Join-Path $env:TEMP "$pkgName.zip"
$tempExtract = Join-Path $env:TEMP "$pkgName"

Write-Host "Downloading $downloadUrl..." -ForegroundColor Cyan
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12 -bor [Net.SecurityProtocolType]::Tls13
    Invoke-WebRequest -Uri $downloadUrl -OutFile $tempZip -UseBasicParsing
} catch {
    Write-Host "Error downloading release binary from $downloadUrl." -ForegroundColor Red
    Write-Host "You can build from source via: cargo install strata-cli" -ForegroundColor Yellow
    exit 1
}

Write-Host "Extracting archive..." -ForegroundColor Gray
if (Test-Path $tempExtract) {
    Remove-Item -Path $tempExtract -Recurse -Force
}
Expand-Archive -Path $tempZip -DestinationPath $tempExtract -Force

# 4. Determine install directory (~/.cargo/bin or ~/AppData/Local/Programs/strata)
$cargoBin = Join-Path $HOME ".cargo\bin"
if (Test-Path $cargoBin) {
    $installDir = $cargoBin
} else {
    $installDir = Join-Path $env:LOCALAPPDATA "Programs\strata"
    if (!(Test-Path $installDir)) {
        New-Item -ItemType Directory -Force -Path $installDir | Out-Null
    }
}

# Find strata.exe inside extracted files
$sourceExe = Join-Path $tempExtract "$pkgName\strata.exe"
if (!(Test-Path $sourceExe)) {
    $sourceExe = Join-Path $tempExtract "strata.exe"
}
if (!(Test-Path $sourceExe)) {
    $found = Get-ChildItem -Path $tempExtract -Filter "strata.exe" -Recurse -File | Select-Object -First 1
    if ($found) {
        $sourceExe = $found.FullName
    } else {
        Write-Host "Error: Could not locate strata.exe inside downloaded archive." -ForegroundColor Red
        exit 1
    }
}

$destExe = Join-Path $installDir "strata.exe"
try {
    Copy-Item -Path $sourceExe -Destination $destExe -Force
} catch {
    # If the file is currently running/locked by another process, rename it first (allowed on NTFS)
    $oldExe = Join-Path $installDir "strata.exe.old"
    if (Test-Path $oldExe) {
        Remove-Item -Path $oldExe -Force -ErrorAction SilentlyContinue
    }
    try {
        Move-Item -Path $destExe -Destination $oldExe -Force
        Copy-Item -Path $sourceExe -Destination $destExe -Force
        Remove-Item -Path $oldExe -Force -ErrorAction SilentlyContinue
    } catch {
        Write-Host "Warning: Could not overwrite running strata.exe. Please terminate running strata processes and retry." -ForegroundColor Yellow
    }
}


# Cleanup temp files
Remove-Item -Path $tempZip -Force -ErrorAction SilentlyContinue
Remove-Item -Path $tempExtract -Recurse -Force -ErrorAction SilentlyContinue

Write-Host ""
Write-Host "Strata installed successfully to: $destExe" -ForegroundColor Green

# 5. Ensure install directory is in PATH
$currentPathParts = $env:Path -split ';' | Where-Object { $_ -ne "" }
if ($currentPathParts -notcontains $installDir) {
    $env:Path = "$installDir;$($env:Path)"
}

$userPath = [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::User)
$userPathParts = @()
if ($userPath) {
    $userPathParts = $userPath -split ';' | Where-Object { $_ -ne "" }
}
if ($userPathParts -notcontains $installDir) {
    Write-Host "Adding $installDir to User PATH..." -ForegroundColor Cyan
    $newPath = $installDir
    if ($userPath) {
        $newPath = "$installDir;$userPath"
    }
    [Environment]::SetEnvironmentVariable("Path", $newPath, [EnvironmentVariableTarget]::User)
    Write-Host "Added to User PATH (available across all new terminal sessions)." -ForegroundColor Green
}

# 6. Run strata --version and print next steps
Write-Host ""
try {
    & $destExe --version
} catch {
    Write-Host "Strata CLI installed." -ForegroundColor Gray
}

Write-Host ""
Write-Host "Welcome to Strata!" -ForegroundColor Green
Write-Host "To initialize persistent memory in your repository, run:" -ForegroundColor White
Write-Host "  strata init" -ForegroundColor Cyan
Write-Host ""
Write-Host "Next steps:" -ForegroundColor Gray
Write-Host "  strata mcp install    # Auto-configure Cursor, Claude Code, and Windsurf"
Write-Host "  strata doctor         # Run diagnostic health check on local engine"
Write-Host ""
