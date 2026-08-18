# ☁️ Strata Cloud Sync Server

Servidor de sincronização em nuvem ultraleve construído em **Rust + Axum** para replicação de memórias cognitivas e deltas CDC (*Change Data Capture*) entre múltiplos dispositivos do desenvolvedor.

---

## ⚡ Recursos

- **Endpoints REST CDC**: Endpoints de `/sync/push`, `/sync/pull` e `/sync/status`.
- **Armazenamento SQLite com WAL**: Persistência atômica e veloz sem necessidade de gerenciar bancos externos complexos.
- **Notificações em Tempo Real via WebSocket**: Canal `/ws` que transmite eventos de deltas novos para clientes conectados.
- **Autenticação Segura**: Suporte a Bearer Token via `STRATA_SERVER_SECRET`.
- **Pronto para Railway**: Healthcheck em `/health`, suporte a `$PORT` dinâmico e volume persistente em `/data`.

---

## 🚀 Deploy no Railway

### 1. Criar novo serviço no Railway
Aponte o Railway para o repositório Git com este projeto.

### 2. Configurar Variáveis de Ambiente
No painel do Railway (Variables):
- `STRATA_SERVER_SECRET`: Segredo de autenticação compartilhado entre suas instâncias do Strata.
- `DATABASE_PATH`: `/data/strata_sync.db` (opcional, padrão no Docker).
- `RUST_LOG`: `strata_server=info,tower_http=info`

### 3. Montar Volume Persistente
- Crie um Volume no Railway e monte no caminho `/data`.

---

## 💻 Configuração do Cliente Local (`strata CLI`)

No seu terminal local (em cada máquina onde você utiliza Cursor, Claude, Codex ou Gemini):

```bash
# Definir variáveis de ambiente locais
export STRATA_SYNC_ENDPOINT="https://seu-app.railway.app"
export STRATA_SYNC_TOKEN="seu-segredo-aqui"

# Verificar status da sincronização
strata sync status --workspace meu-projeto

# Fazer push dos deltas locais pendentes
strata sync push --workspace meu-projeto

# Fazer pull dos deltas remotos com resolução JTMS
strata sync pull --workspace meu-projeto
```

---

## 📡 Endpoints da API

| Método | Rota | Descrição |
|---|---|---|
| `GET` | `/health` | Liveness check para o Railway |
| `POST` | `/sync/push` ou `/` | Envia deltas CDC locais (`{ workspace_id, deltas }`) |
| `GET` | `/sync/pull` ou `/pull` | Baixa deltas remotos (`?workspace_id=...&since_seq=...&limit=...`) |
| `GET` | `/sync/status` ou `/status` | Retorna total de deltas e maior número de sequência |
| `GET` | `/ws` ou `/sync/ws` | Stream WebSocket de notificações de sincronização em tempo real |
