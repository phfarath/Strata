# Strata Multi-Agent Stigmergic Coordination Test
# Simulates two concurrent agents (Cursor IDE + Claude Code Terminal)
$ErrorActionPreference = "Stop"

Write-Host ""
Write-Host "=======================================================" -ForegroundColor Cyan
Write-Host "[STRATA] MULTI-AGENT CONCURRENCY & STIGMERGY TEST" -ForegroundColor Cyan
Write-Host "=======================================================" -ForegroundColor Cyan

# 1. Heartbeat - Agent 1: Cursor IDE
Write-Host ""
Write-Host "[1/5] Agente 1 (Cursor IDE) registra presenca no workspace via MCP..." -ForegroundColor Yellow
$hb1 = '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"agent_heartbeat","arguments":{"agent_id":"cursor-ide","host":"cursor","pid":1042,"active_task":"Refatorando crates/strata-cli/src/main.rs"}}}'
$resp1 = $hb1 | strata mcp | ConvertFrom-Json
Write-Host "  -> Cursor registrou presenca: $($resp1.result.content[0].text)" -ForegroundColor Green

# 2. Heartbeat - Agent 2: Claude Code
Write-Host ""
Write-Host "[2/5] Agente 2 (Claude Code) registra presenca concorrente no workspace via MCP..." -ForegroundColor Yellow
$hb2 = '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"agent_heartbeat","arguments":{"agent_id":"claude-code","host":"claude-code","pid":2084,"active_task":"Criando testes de integracao E2E"}}}'
$resp2 = $hb2 | strata mcp | ConvertFrom-Json
Write-Host "  -> Claude registrou presenca: $($resp2.result.content[0].text)" -ForegroundColor Green

# 3. Status - Inspect active presence
Write-Host ""
Write-Host "[3/5] Consultando estado estigmergico do workspace ('strata a2a who')..." -ForegroundColor Yellow
strata a2a who

# 4. Exclusive Lease Acquisition by Cursor
Write-Host ""
Write-Host "[4/5] Cursor adquire lease exclusivo sobre 'crate:strata-cli' por 8 segundos..." -ForegroundColor Yellow
$acq1 = '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"lease_acquire","arguments":{"resource_id":"crate:strata-cli","agent_id":"cursor-ide","ttl_seconds":8,"metadata":"Refatorando comandos CLI"}}}'
$resp_acq1 = $acq1 | strata mcp | ConvertFrom-Json
Write-Host "  -> Cursor adquiriu lease: $($resp_acq1.result.content[0].text)" -ForegroundColor Green

# 5. Collision Detection by Claude
Write-Host ""
Write-Host "[5/5] Claude tenta adquirir o MESMO recurso ('crate:strata-cli')..." -ForegroundColor Yellow
$acq2 = '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"lease_acquire","arguments":{"resource_id":"crate:strata-cli","agent_id":"claude-code","ttl_seconds":15}}}'
$resp_acq2 = $acq2 | strata mcp | ConvertFrom-Json
$conflict_raw = $resp_acq2.result.content[0].text
Write-Host "  -> Resposta do Strata: $conflict_raw" -ForegroundColor Magenta

if ($conflict_raw -match "conflict" -and $conflict_raw -match "cursor-ide") {
    Write-Host "  [OK] SUCESSO: Conflito detectado deterministicamente! Claude impedido de sobrescrever trabalho do Cursor." -ForegroundColor Green
} else {
    Write-Host "  [FALHA]: Conflito nao detectado!" -ForegroundColor Red
    exit 1
}

# 6. Autonomous Pivot - Claude works on another module in parallel
Write-Host ""
Write-Host "[6/5] Claude redireciona trabalho para 'crate:strata-evals' em paralelo..." -ForegroundColor Yellow
$acq3 = '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"lease_acquire","arguments":{"resource_id":"crate:strata-evals","agent_id":"claude-code","ttl_seconds":20,"metadata":"Executando cenarios de avaliacao"}}}'
$resp_acq3 = $acq3 | strata mcp | ConvertFrom-Json
Write-Host "  -> Claude adquiriu lease em paralelo: $($resp_acq3.result.content[0].text)" -ForegroundColor Green

# 7. Workspace Status with both leases
Write-Host ""
Write-Host "Status consolidado dos dois agentes e seus locks:" -ForegroundColor Yellow
strata a2a status

# 8. Release and Clean Recovery
Write-Host ""
Write-Host "[7/5] Cursor finaliza edicao e libera 'crate:strata-cli'..." -ForegroundColor Yellow
$rel = '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"lease_release","arguments":{"resource_id":"crate:strata-cli","agent_id":"cursor-ide"}}}'
$resp_rel = $rel | strata mcp | ConvertFrom-Json
Write-Host "  -> Cursor liberou recurso: $($resp_rel.result.content[0].text)" -ForegroundColor Green

# Clean up remaining Claude lease
strata a2a release crate:strata-evals --agent claude-code | Out-Null

Write-Host ""
Write-Host "=======================================================" -ForegroundColor Green
Write-Host "[OK] TODOS OS TESTES DE CONCORRENCIA A2A PASSARAM COM SUCESSO!" -ForegroundColor Green
Write-Host "=======================================================" -ForegroundColor Green
Write-Host ""
