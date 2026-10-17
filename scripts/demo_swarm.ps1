# Real Multi-Process Multi-Agent Swarm Concurrency Demonstration
param(
    [string]$DbPath = "$env:TEMP\strata_live_swarm_$([guid]::NewGuid()).db"
)

$ErrorActionPreference = "Stop"
$StrataExe = ".\target\debug\strata.exe"

if (-not (Test-Path $StrataExe)) {
    Write-Host "Building strata binary..." -ForegroundColor Yellow
    cargo build -p strata-cli --bin strata
}

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "  STRATA MULTI-AGENT SWARM CONCURRENCY LIVE DEMONSTRATION  " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "Database: $DbPath" -ForegroundColor Gray

# Clean up any pre-existing db
if (Test-Path $DbPath) { Remove-Item $DbPath -Force }

Write-Host "`n[1/4] Populating Blackboard with Contested Task DAG..." -ForegroundColor Yellow

# Task 1: Auth Module (Locks file:src/auth.rs)
& $StrataExe task create "Decompose Auth Module" --id "task-auth-core" --resources "file:src/auth.rs" --db-path $DbPath --plan "live-swarm" | Out-Null

# Task 2: Session Storage (Locks file:src/session.rs)
& $StrataExe task create "Refactor Session Storage" --id "task-session-core" --resources "file:src/session.rs" --db-path $DbPath --plan "live-swarm" | Out-Null

# Task 3: Auth Interceptor (Contends on file:src/auth.rs with Task 1!)
& $StrataExe task create "Add Auth Rate Limiter" --id "task-auth-limiter" --resources "file:src/auth.rs" --db-path $DbPath --plan "live-swarm" | Out-Null

# Task 4: Unit Tests (Blocked on Task 1 and Task 2)
& $StrataExe task create "Integration Tests (Auth + Session)" --id "task-integ-tests" --dependencies "task-auth-core,task-session-core" --db-path $DbPath --plan "live-swarm" | Out-Null

# Task 5: Final Verification (Blocked on Task 3 and Task 4)
& $StrataExe task create "Swarm Deployment Verification" --id "task-verify-deploy" --dependencies "task-auth-limiter,task-integ-tests" --db-path $DbPath --plan "live-swarm" | Out-Null

Write-Host "Blackboard initialized with 5 tasks (2 root, 1 lock-contested, 2 blocked DAG dependencies)." -ForegroundColor Green

Write-Host "`n[2/4] Initial Blackboard State:" -ForegroundColor Yellow
& $StrataExe task list --db-path $DbPath

Write-Host "`n[3/4] Spawning 3 Concurrent Worker Processes in Parallel..." -ForegroundColor Yellow
$sw = [System.Diagnostics.Stopwatch]::StartNew()

$jobAlpha = Start-Job -ScriptBlock {
    param($exe, $db)
    & $exe worker --agent "worker-alpha" --db-path $db --poll-interval 1 --heartbeat-interval 1 --lease-ttl 10 --exit-when-idle --dry-run
} -ArgumentList (Resolve-Path $StrataExe), $DbPath

$jobBeta = Start-Job -ScriptBlock {
    param($exe, $db)
    & $exe worker --agent "worker-beta" --db-path $db --poll-interval 1 --heartbeat-interval 1 --lease-ttl 10 --exit-when-idle --dry-run
} -ArgumentList (Resolve-Path $StrataExe), $DbPath

$jobGamma = Start-Job -ScriptBlock {
    param($exe, $db)
    & $exe worker --agent "worker-gamma" --db-path $db --poll-interval 1 --heartbeat-interval 1 --lease-ttl 10 --exit-when-idle --dry-run
} -ArgumentList (Resolve-Path $StrataExe), $DbPath

# Wait for all 3 worker processes to complete
$null = Wait-Job $jobAlpha, $jobBeta, $jobGamma
$sw.Stop()

Write-Host "`nAll 3 worker processes finished in $($sw.ElapsedMilliseconds) ms!" -ForegroundColor Green

Write-Host "`n--- Worker Output: worker-alpha ---" -ForegroundColor Cyan
Receive-Job $jobAlpha
Write-Host "`n--- Worker Output: worker-beta ---" -ForegroundColor Cyan
Receive-Job $jobBeta
Write-Host "`n--- Worker Output: worker-gamma ---" -ForegroundColor Cyan
Receive-Job $jobGamma

Remove-Job $jobAlpha, $jobBeta, $jobGamma

Write-Host "`n[4/4] Verifying Final Blackboard State..." -ForegroundColor Yellow
& $StrataExe task list --db-path $DbPath

Write-Host "`nActive Locks Verification:" -ForegroundColor Yellow
& $StrataExe a2a leases --db-path $DbPath

# Cleanup
if (Test-Path $DbPath) { Remove-Item $DbPath -Force }
Write-Host "`nDemo completed successfully. Cleaned up temporary test database." -ForegroundColor Green
