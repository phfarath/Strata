# 🌐 Guia de Configuração: Domínio Próprio & Segurança Cloud (Railway)

Este documento descreve como vincular seu domínio próprio (ex: **`strata.pedrofarath.me`**) à sua instância do **Strata Cloud Server** no Railway com renovação automática de SSL/TLS via Let's Encrypt e proteção avançada de headers HTTP.

---

## 🗺️ 1. Vinculação de DNS no Railway

1. Acesse o dashboard do seu projeto no **[Railway](https://railway.app)**.
2. Clique no serviço do **`strata-server`**.
3. Vá na aba **Settings** $\rightarrow$ seção **Networking** $\rightarrow$ **Custom Domain**.
4. Digite seu domínio:
   ```text
   strata.pedrofarath.me
   ```
5. O Railway gerará um registro DNS de destino no formato CNAME:
   ```text
   strata.pedrofarath.me.up.railway.app
   ```
6. Acesse o painel do seu provedor de DNS (Cloudflare, Namecheap, GoDaddy, Route53, etc.) e adicione:
   - **Tipo**: `CNAME`
   - **Nome / Host**: `strata` (ou `strata.pedrofarath.me`)
   - **Target / Valor**: `strata.pedrofarath.me.up.railway.app` (ou o CNAME indicado pelo Railway)
   - **Proxy status**: DNS Only / Desativado (se estiver no Cloudflare para permitir validação ACME direta)
   - **TTL**: Auto ou 300s

> [!NOTE]
> A emissão do certificado SSL/TLS Let's Encrypt pelo Railway ocorre automaticamente em 1 a 5 minutos após a propagação do DNS.

---

## 🔒 2. Headers de Segurança HTTP Ativos

O `strata-server` injeta automaticamente headers de segurança para mitigar ataques comuns da web:

| Header | Valor Padrão | Finalidade |
|---|---|---|
| `Strict-Transport-Security` (HSTS) | `max-age=63072000; includeSubDomains; preload` | Força conexões HTTPS durante 2 anos |
| `X-Content-Type-Options` | `nosniff` | Impede ataques de interpretação MIME incorreta |
| `X-Frame-Options` | `DENY` | Protege contra ataques de Clickjacking |
| `Referrer-Policy` | `strict-origin-when-cross-origin` | Preserva privacidade ao navegar entre origens |
| `Content-Security-Policy` (CSP) | `default-src 'self'; ...` | Restringe execução de scripts não autorizados |
| `Permissions-Policy` | `camera=(), microphone=(), ...` | Desativa APIs de sensores desnecessárias |

---

## ⚙️ 3. Variáveis de Ambiente no Railway

No painel do Railway (**Variables**):

```bash
# Domínio público configurado
CUSTOM_DOMAIN="strata.pedrofarath.me"

# Banco PostgreSQL (Supabase Transaction Pooler ou Direct)
DATABASE_URL="postgresql://postgres.[ref]:[password]@aws-0-sa-east-1.pooler.supabase.com:6543/postgres"

# Chave secreta para JWT dos clientes e Web Dashboard
JWT_SECRET="seu-jwt-secret-longo-e-seguro"

# Origens permitidas para CORS (opcional, padrão: *)
CORS_ALLOWED_ORIGINS="https://strata.pedrofarath.me,http://localhost:54321"

# Ativação de headers de segurança (padrão: true)
ENABLE_SECURITY_HEADERS="true"
```

---

## 🧪 4. Verificação de Conectividade e Latência

Você pode testar seu domínio a qualquer momento via terminal:

```bash
# Teste de Ping e latência
curl -s -i https://strata.pedrofarath.me/api/v1/ping

# Resposta esperada:
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

## 💻 5. Uso no `strata CLI` Local

No seu ambiente local (Cursor / VS Code / Terminal):

```bash
# Login direto no navegador via seu domínio próprio:
strata login --server https://strata.pedrofarath.me

# Ou configurando variáveis de ambiente:
export STRATA_SYNC_ENDPOINT="https://strata.pedrofarath.me"
export STRATA_SYNC_TOKEN="strata_live_..."

# Sincronização automática:
strata sync status
```
