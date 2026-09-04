# Configuration Guide: Custom Domain & Cloud Security (Railway)

This guide documents the procedure for binding a custom domain (e.g., `strata.pedrofarath.me`) to your **Strata Cloud Server** instance on Railway, featuring automated SSL/TLS provisioning via Let's Encrypt and hardened HTTP security headers.

---

## 1. DNS Configuration on Railway

1. Open your project dashboard on **[Railway](https://railway.app)**.
2. Select the **`strata-server`** deployment.
3. Navigate to **Settings** $\rightarrow$ **Networking** $\rightarrow$ **Custom Domain**.
4. Enter your fully qualified domain name:
   ```text
   strata.pedrofarath.me
   ```
5. Railway provides an endpoint target CNAME record:
   ```text
   strata.pedrofarath.me.up.railway.app
   ```
6. Open your authoritative DNS provider dashboard (Cloudflare, Namecheap, GoDaddy, AWS Route53, etc.) and create:
   - **Record Type**: `CNAME`
   - **Host / Name**: `strata` (or `strata.pedrofarath.me`)
   - **Target / Value**: `strata.pedrofarath.me.up.railway.app` (or the target provided by Railway)
   - **Proxy Status**: DNS Only / Bypassed (if utilizing Cloudflare, to facilitate direct ACME challenge resolution)
   - **TTL**: Auto or 300 seconds

> [!NOTE]
> Automated Let's Encrypt SSL/TLS certificate issuance through Railway completes within 1 to 5 minutes following global DNS propagation.

---

## 2. Hardened HTTP Security Headers

The `strata-server` natively injects defensive HTTP security headers to mitigate common web and API attack vectors:

| Header | Default Value | Security Purpose |
|---|---|---|
| `Strict-Transport-Security` (HSTS) | `max-age=63072000; includeSubDomains; preload` | Enforces HTTPS transport for 2 years with preloading |
| `X-Content-Type-Options` | `nosniff` | Blocks MIME-type sniffing and downgrade attacks |
| `X-Frame-Options` | `DENY` | Prevents framing and clickjacking attacks |
| `Referrer-Policy` | `strict-origin-when-cross-origin` | Protects privacy during cross-origin requests |
| `Content-Security-Policy` (CSP) | `default-src 'self'; ...` | Restricts unauthorized scripts, frames, and resource loads |
| `Permissions-Policy` | `camera=(), microphone=(), ...` | Restricts access to unnecessary browser hardware APIs |

---

## 3. Environment Variables Configuration

Configure the following environment variables within the Railway dashboard under **Variables**:

```bash
# Publicly exposed domain
CUSTOM_DOMAIN="strata.pedrofarath.me"

# PostgreSQL connection string (Supabase Transaction Pooler or Direct connection)
DATABASE_URL="postgresql://postgres.[ref]:[password]@aws-0-sa-east-1.pooler.supabase.com:6543/postgres"

# Cryptographic secret for client JWT validation and Web Dashboard sessions
JWT_SECRET="your-cryptographically-secure-jwt-secret"

# Permitted CORS origins (optional, default: *)
CORS_ALLOWED_ORIGINS="https://strata.pedrofarath.me,http://localhost:54321"

# Toggle defensive HTTP security headers (default: true)
ENABLE_SECURITY_HEADERS="true"
```

---

## 4. Endpoint Verification & Latency Probing

Verify edge connectivity, SSL negotiation, and endpoint latency via CLI:

```bash
# Health check and TLS header probe
curl -s -i https://strata.pedrofarath.me/api/v1/ping

# Expected response:
# HTTP/2 200
# strict-transport-security: max-age=63072000; includeSubDomains; preload
# x-content-type-options: nosniff
# x-frame-options: DENY
#
# {
#   "status": "pong",
#   "timestamp": "2026-08-19T15:30:00.000Z",
#   "epoch_ms": 1787153400000,
#   "protocol": "strata-cloud/v1",
#   "custom_domain": "strata.pedrofarath.me",
#   "is_postgres": true,
#   "has_pgvector": true,
#   "uptime_secs": 120
# }
```

---

## 5. Local Client Integration via `strata CLI`

Connect local developer environments (Cursor / VS Code / Terminal) to the cloud instance:

```bash
# Zero-config authentication using the preconfigured official endpoint:
strata login

# Alternatively, explicitly specify the custom endpoint:
strata login --endpoint https://strata.pedrofarath.me

# Synchronize local cognitive memory state:
strata sync status
strata sync push
strata sync pull
```
