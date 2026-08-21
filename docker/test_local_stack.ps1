$ErrorActionPreference = "Stop"

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host " 🐳 STRATA LOCAL DOCKER CLOUD STACK VERIFICATION" -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan

# 1. Health check
$health = Invoke-RestMethod -Uri "http://localhost:8080/health" -Method Get
Write-Host " [1/6] Health Check OK:" -ForegroundColor Green
Write-Host "       - PostgreSQL Active: $($health.is_postgres)"
Write-Host "       - pgvector Extension: $($health.has_pgvector)"
Write-Host "       - Version: $($health.version)"

# 2. Signup User (/api/v1/auth/signup)
$bodySignup = @{
    full_name = "Pedro Farath"
    email = "pedro_local_$(Get-Random)@strata.dev"
    password = "StrongPassword123!"
} | ConvertTo-Json

$signup = Invoke-RestMethod -Uri "http://localhost:8080/api/v1/auth/signup" -Method Post -ContentType "application/json" -Body $bodySignup
$token = $signup.token
$userId = $signup.user.id
$workspaceId = $signup.workspaces[0].id
$workspaceSlug = $signup.workspaces[0].slug
Write-Host " [2/6] User Signup OK: User ID = $userId, Workspace = $workspaceSlug ($workspaceId)" -ForegroundColor Green

# 3. Verify Session Token (/api/v1/auth/me)
$me = Invoke-RestMethod -Uri "http://localhost:8080/api/v1/auth/me" -Headers @{ Authorization = "Bearer $token" }
Write-Host " [3/6] Session Auth (/api/v1/auth/me) OK: Name = $($me.user.full_name)" -ForegroundColor Green

# 4. Generate API Key (/api/v1/keys)
$bodyKey = @{ 
    workspace_id = $workspaceId
    name = "E2E Local Docker API Key" 
} | ConvertTo-Json
$keyResp = Invoke-RestMethod -Uri "http://localhost:8080/api/v1/keys" -Method Post -Headers @{ Authorization = "Bearer $token" } -ContentType "application/json" -Body $bodyKey
$apiKey = $keyResp.key
Write-Host " [4/6] API Key Generated OK: $($apiKey.Substring(0, 15))..." -ForegroundColor Green

# 5. Push & Pull Sync Deltas using API Key
$deltaId = [guid]::NewGuid().ToString()
$bodyPush = @{
    workspace_id = $workspaceId
    deltas = @(
        @{
            id = $deltaId
            workspace_id = $workspaceId
            seq = 1
            ts = (Get-Date).ToUniversalTime().ToString("o")
            kind = "semantic_fact"
            payload = @{
                statement = "Strata local Docker environment mirrors production Railway/Supabase pgvector stack exactly"
                category = "architecture"
                scope = "global"
            }
            version_hash = "hash-docker-local-1"
        }
    )
} | ConvertTo-Json -Depth 5

$pushResp = Invoke-RestMethod -Uri "http://localhost:8080/sync/push" -Method Post -Headers @{ Authorization = "Bearer $apiKey" } -ContentType "application/json" -Body $bodyPush
Write-Host " [5/6] Sync Delta Push OK: Accepted = $($pushResp.accepted), Current Sequence = $($pushResp.current_sequence)" -ForegroundColor Green

# Pull verify
$pullResp = Invoke-RestMethod -Uri "http://localhost:8080/sync/pull?workspace_id=$workspaceId&since_sequence=0" -Headers @{ Authorization = "Bearer $apiKey" }
Write-Host "       - Sync Delta Pull OK: Retrieved $($pullResp.deltas.Count) deltas from Postgres" -ForegroundColor Green

# 6. pgvector Vector Embeddings Upsert & Search
$memId = [guid]::NewGuid().ToString()
$sampleEmbedding = @(0.12, 0.45, -0.33, 0.88) + @(0.0) * 380
$bodyUpsertVec = @{
    workspace_id = $workspaceId
    memory_id = $memId
    embedding = $sampleEmbedding
    metadata = @{ content = "Vector test in local pgvector Docker" }
} | ConvertTo-Json

$upsertResp = Invoke-RestMethod -Uri "http://localhost:8080/api/v1/embeddings/upsert" -Method Post -Headers @{ Authorization = "Bearer $apiKey" } -ContentType "application/json" -Body $bodyUpsertVec
Write-Host " [6/6] pgvector Embedding Upsert OK: Memory ID = $($upsertResp.memory_id)" -ForegroundColor Green

$bodySearchVec = @{
    workspace_id = $workspaceId
    query_embedding = $sampleEmbedding
    limit = 5
} | ConvertTo-Json

$searchResp = Invoke-RestMethod -Uri "http://localhost:8080/api/v1/embeddings/search" -Method Post -Headers @{ Authorization = "Bearer $apiKey" } -ContentType "application/json" -Body $bodySearchVec
Write-Host "       - pgvector Cosine Search OK: Found $($searchResp.results.Count) matches (Top Match ID: $($searchResp.results[0].memory_id))" -ForegroundColor Green

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host " 🎉 ALL CLOUD STACK ENDPOINTS (AUTH, SYNC, PGVECTOR) VERIFIED LOCALLY ON DOCKER!" -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan
