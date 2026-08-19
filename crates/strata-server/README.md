# ☁️ Strata Cloud Sync & SaaS Server

Servidor de nuvem universal de alto desempenho construído em **Rust + Axum** para replicação de memórias cognitivas, autenticação multi-tenant, autorização de CLI/IDE no navegador e persistência agnóstica de banco de dados (**PostgreSQL com Supabase/Neon/Railway** ou **SQLite local/offline**).

---

## ⚡ Recursos Principais

- **Arquitetura Agnóstica de Banco de Dados**:
  - **PostgreSQL**: Suporte nativo e assíncrono com pool de conexões (`deadpool-postgres`), TLS puro em Rust (`rustls` + `webpki-roots`) e extensão vetorial **`pgvector`**.
  - **Compatibilidade Universal**: Supabase, Neon Serverless, Railway Postgres, AWS RDS, GCP Cloud SQL e Docker local via `DATABASE_URL`.
  - **SQLite com WAL**: Fallback automático e transparente para desenvolvimento local e ambientes sem banco externo.
- **Migrações Automáticas Atômicas**:
  - Criação automática de tabelas (`users`, `workspaces`, `api_keys`, `workspace_sequences`, `server_deltas`, `server_embeddings`) e índices na inicialização.
- **Autenticação SaaS & Dispositivos**:
  - JWT assinado e seguro para usuários e portal web.
  - API Keys com prefixo e hash seguro (`strata_live_...`) para agentes (Cursor, Claude, Codex, Gemini).
  - Fluxo OAuth-like de autorização no navegador para CLI (`/auth/cli`).
- **Endpoints REST CDC (Change Data Capture)**:
  - Endpoints atômicos de `/api/v1/sync/push`, `/api/v1/sync/pull` e `/api/v1/sync/status`.
- **Busca Vetorial Centralizada (`pgvector`)**:
  - Endpoints `/api/v1/embeddings/upsert` e `/api/v1/embeddings/search` com busca por similaridade de cosseno indexada via HNSW.
- **Notificações em Tempo Real via WebSocket**:
  - Canal `/api/v1/sync/ws` que transmite eventos instantâneos para sincronização multi-dispositivo.

---

## 🐘 Configuração com PostgreSQL (Supabase / Neon / Railway)

Basta definir a variável de ambiente **`DATABASE_URL`** no seu provedor ou no `.env`:

### 1. Supabase (Transaction Pooler ou Direct)
```bash
DATABASE_URL="postgresql://postgres.[sua-ref]:[sua-senha]@aws-0-sa-east-1.pooler.supabase.com:6543/postgres"
```

### 2. Neon Serverless Postgres
```bash
DATABASE_URL="postgresql://[user]:[password]@[ep-id].us-east-2.aws.neon.tech/neondb?sslmode=require"
```

### 3. Railway Postgres
```bash
DATABASE_URL="${{Postgres.DATABASE_URL}}"
```

### 4. Docker / Dev Local
```bash
DATABASE_URL="postgres://postgres:postgres@localhost:5432/strata?sslmode=disable"
```

---

## 🚀 Variáveis de Ambiente do Servidor

| Variável | Padrão | Descrição |
|---|---|---|
| `DATABASE_URL` | `None` | URL de conexão PostgreSQL (`postgres://...`) ou SQLite (`sqlite://...`) |
| `DATABASE_PATH` | `None` | Caminho do arquivo SQLite local (usado se `DATABASE_URL` não for fornecido) |
| `PORT` | `8080` | Porta HTTP do servidor |
| `HOST` | `0.0.0.0` | Host de escuta |
| `CUSTOM_DOMAIN` | `strata.pedrofarath.me` | Domínio público para CORS e diagnósticos |
| `CORS_ALLOWED_ORIGINS` | `*` | Origens CORS permitidas (separadas por vírgula) |
| `ENABLE_SECURITY_HEADERS` | `true` | Ativação de HSTS, CSP, X-Frame-Options, etc. |
| `JWT_SECRET` | *(auto)* | Chave secreta para assinatura dos tokens JWT |
| `STRATA_SERVER_SECRET`| `None` | Token estático legado de autorização global (opcional) |
| `RUST_LOG` | `info` | Nível de log (`strata_server=info,tower_http=info`) |

---

## 📡 Endpoints da API

| Método | Rota | Descrição |
|---|---|---|
| `GET` | `/ping` ou `/api/v1/ping` | Diagnóstico de latência, protocolo e status do banco |
| `GET` | `/health` | Liveness check (informa status, uptime, `is_postgres` e `has_pgvector`) |
| `GET` | `/auth/cli` | Interface visual de login/autorização para o terminal |
| `POST` | `/api/v1/auth/cli/authorize` | Callback de autorização de dispositivo CLI |
| `POST` | `/api/v1/auth/signup` | Criação de conta de usuário |
| `POST` | `/api/v1/auth/login` | Login com email e senha |
| `GET` | `/api/v1/auth/me` | Dados do usuário autenticado e workspaces |
| `POST` | `/api/v1/workspaces` | Criação de novo workspace |
| `GET` | `/api/v1/workspaces` | Listagem de workspaces do usuário |
| `POST` | `/api/v1/keys` | Criação de nova API Key para agente/máquina |
| `GET` | `/api/v1/keys` | Listagem de API Keys ativas |
| `DELETE`| `/api/v1/keys/{id}` | Revogação de API Key |
| `POST` | `/api/v1/sync/push` | Envia deltas CDC (`{ workspace_id, deltas }`) |
| `GET` | `/api/v1/sync/pull` | Baixa deltas remotos (`?workspace_id=...&since_seq=...`) |
| `GET` | `/api/v1/sync/status` | Retorna status da sequência e contadores |
| `POST` | `/api/v1/embeddings/upsert` | Grava vetor de embedding centralizado (`pgvector`) |
| `POST` | `/api/v1/embeddings/search` | Realiza busca semântica por similaridade de cosseno |
| `GET` | `/api/v1/sync/ws` | Stream WebSocket de sincronização em tempo real |
