$ErrorActionPreference = "Stop"

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host " STRATA FULL END-TO-END (E2E) DOCKER AND UI TEST SUITE" -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

# -----------------------------------------------------------------------------
# 1. Start Docker Containers (Postgres pgvector + Axum Server + React Web UI)
# -----------------------------------------------------------------------------
Write-Host "`n[1/4] Starting Docker Stack (PostgreSQL 16 pgvector, Axum Server and Web UI)..." -ForegroundColor Yellow
docker compose up -d --build

# -----------------------------------------------------------------------------
# 2. Wait for Service Healthchecks
# -----------------------------------------------------------------------------
Write-Host "`n[2/4] Waiting for services to become healthy..." -ForegroundColor Yellow
$maxAttempts = 30
$attempt = 0
$serverHealthy = $false

while ($attempt -lt $maxAttempts) {
    Start-Sleep -Seconds 2
    $attempt++
    try {
        $health = Invoke-RestMethod -Uri "http://localhost:8080/health" -Method Get -TimeoutSec 2
        if ($health.status -eq "ok") {
            $serverHealthy = $true
            Write-Host "        - strata-server is healthy (version: $($health.version), pgvector: $($health.has_pgvector))" -ForegroundColor Green
            break
        }
    } catch {
        Write-Host "        - Waiting for server... (attempt $attempt/$maxAttempts)" -ForegroundColor Gray
    }
}

if (-not $serverHealthy) {
    docker compose logs server
    throw "Strata Server failed to become healthy within timeout."
}

# Check Web UI
$webHealthy = $false
$attempt = 0
while ($attempt -lt 15) {
    Start-Sleep -Seconds 1
    $attempt++
    try {
        $web = Invoke-WebRequest -Uri "http://localhost:3000" -Method Get -UseBasicParsing -TimeoutSec 2
        if ($web.StatusCode -eq 200) {
            $webHealthy = $true
            Write-Host "        - strata-web is healthy (HTTP 200 OK on port 3000)" -ForegroundColor Green
            break
        }
    } catch {
        Write-Host "        - Waiting for web UI... (attempt $attempt/15)" -ForegroundColor Gray
    }
}

if (-not $webHealthy) {
    docker compose logs web
    throw "Strata Web UI failed to become healthy within timeout."
}

# -----------------------------------------------------------------------------
# 3. Execute Backend E2E Suite (APIs, Multitenancy, CDC, pgvector)
# -----------------------------------------------------------------------------
Write-Host "`n[3/4] Executing Backend and Cloud Sync E2E Suite..." -ForegroundColor Yellow
& (Join-Path $PSScriptRoot "test_local_stack.ps1")
if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne $null) {
    throw "Backend E2E test suite failed."
}

# -----------------------------------------------------------------------------
# 4. Execute Playwright Browser Automation E2E Suite (Headless Chromium)
# -----------------------------------------------------------------------------
Write-Host "`n[4/4] Executing Playwright Browser Automation E2E Suite..." -ForegroundColor Yellow
Push-Location (Join-Path $PSScriptRoot "..\web\e2e")
try {
    $env:BASE_URL = "http://localhost:3000"
    $env:API_URL = "http://localhost:8080"
    npx playwright test
    
    if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE -ne $null) {
        throw "Playwright E2E tests failed."
    }
} finally {
    Pop-Location
}

Write-Host "`n==========================================================" -ForegroundColor Green
Write-Host " ALL E2E BACKEND AND BROWSER TESTS PASSED (100% SUCCESS)!" -ForegroundColor Green
Write-Host "==========================================================" -ForegroundColor Green
