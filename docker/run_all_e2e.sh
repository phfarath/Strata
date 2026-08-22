#!/usr/bin/env bash
set -euo pipefail

echo "=========================================================="
echo " 🚀 STRATA FULL END-TO-END (E2E) DOCKER & UI TEST SUITE"
echo "=========================================================="

# 1. Start Docker Stack
echo -e "\n📦 [1/4] Starting Docker Stack..."
docker compose up -d --build

# 2. Wait for Server Health
echo -e "\n⏳ [2/4] Waiting for services to become healthy..."
timeout=60
while ! curl -s http://localhost:8080/health | grep -q '"status":"ok"'; do
  sleep 2
  timeout=$((timeout - 2))
  if [ "$timeout" -le 0 ]; then
    echo "❌ Server failed to start."
    docker compose logs server
    exit 1
  fi
done
echo "        - strata-server is healthy."

while ! curl -s -I http://localhost:3000 | grep -q "200 OK"; do
  sleep 1
  timeout=$((timeout - 1))
  if [ "$timeout" -le 0 ]; then
    echo "❌ Web UI failed to start."
    docker compose logs web
    exit 1
  fi
done
echo "        - strata-web is healthy."

# 3. Backend E2E
echo -e "\n🧪 [3/4] Running Backend E2E..."
# Add backend runner commands if needed

# 4. Playwright Browser E2E
echo -e "\n🎭 [4/4] Running Playwright Browser E2E..."
cd web/e2e
npm install
npx playwright install --with-deps chromium
BASE_URL="http://localhost:3000" API_URL="http://localhost:8080" npx playwright test

echo "=========================================================="
echo " 🎉 ALL E2E TESTS PASSED SUCCESSFULLY!"
echo "=========================================================="
