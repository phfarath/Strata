# Três Fronteiras: Memória Compartilhada sobre MCP e A2A

## Contexto

Esta especificação sintetiza o relatório anexado sobre arquiteturas de memória compartilhada. Distingue infraestrutura viável agora de hipóteses de pesquisa. MCP e A2A são mecanismos de interoperabilidade; não resolvem, por si só, consolidação, confiança, controle de acesso ou qualidade de memória.

## Fronteira 1 — MCP como hipocampo externo

### Proposta

Um servidor MCP de memória recebe episódios de alta frequência e os consolida, por processos offline, em memória semântica e procedural mais estável. A inspiração é a teoria de sistemas de memória complementares: aprendizagem rápida episódica e actualização lenta de conhecimento geral.

### Capacidades

- `write_episode`: grava evento, estado, resultado e proveniência.
- `consolidate`: produz candidatos a fatos, regras e habilidades.
- `replay`: reavalia episódios selecionados por saliência e valor.
- `decay`: reduz prioridade ou expira dados obsoletos.
- `check_conflict`: encontra afirmações incompatíveis antes da promoção.

### Estrutura de dados

| Registro | Campos críticos |
|---|---|
| Episódio | agente, sessão, eventos, instante, saliência, snapshot |
| Fato semântico | conteúdo, episódios-fonte, confiança, estabilidade, validade |
| Procedimento | objetivo, sequência de ferramentas, condições, métricas de desempenho |

### MVP e risco

Começar apenas com `write_episode` e `consolidate`, usando um backend temporal/grafo. Comparar tarefas multissessão com e sem consolidação offline. O risco principal é construir uma metáfora biológica cara que não gere ganho mensurável; a analogia é inspiração, não requisito de fidelidade ao cérebro.

## Fronteira 2 — Memória transativa A2A

### Proposta

Agentes especializados anunciam capacidade de memória e atendem tarefas de armazenar ou recuperar registros para outros agentes. O sistema mantém um índice transativo: além de saber um conteúdo, sabe qual agente ou serviço é responsável por ele.

### Extensão experimental de Agent Card

```json
{
  "memory_capabilities": ["episodic", "semantic", "procedural"],
  "memory_persistence": "longterm",
  "memory_tenant_scopes": ["per_user", "per_org"],
  "memory_privacy_mode": "isolated"
}
```

Esses campos são uma convenção de experimento, não uma extensão formal já padronizada do A2A.

### Contratos de dados

`MemoryRecord` contém identificador, proprietário, sujeito, tipo, conteúdo, ACL e proveniência. `TransactiveIndex` contém tópico, agentes especialistas e confiança. Toda transferência deve preservar tenant, escopo, quem pode ler, quem pode escrever e quem aprovou o compartilhamento.

### MVP e risco

Implementar um agente-memory A2A com `store_memory` e `query_memory`, mais dois agentes clientes em um domínio controlado. Os riscos dominantes são vazamento por ACL, lock-in e diagnóstico difícil de transações distribuídas. Memória compartilhada não deve implicar exposição de estado interno de agentes.

## Fronteira 3 — Metacognição para controlar memória

### Proposta

Uma camada separada estima valor, confiança, obsolescência e conflito de cada memória. Ela decide o que guardar, consolidar, testar, revalidar, reduzir em prioridade ou expirar.

### Capacidades

- `evaluate_memory`: calcula importância, confiança e obsolescência contextual.
- `schedule_retrieval_test`: agenda revalidação de memórias críticas.
- `resolve_conflict`: preserva hipóteses concorrentes e registra a resolução.
- `estimate_obsolescence`: usa idade, fonte, mudanças no domínio e taxa de erro.

### Metadados mínimos

`memory_id`, importância, confiança, último acesso, última verificação, contagem de erro, fontes e resultados de uso. A confiança é uma estimativa calibrável, não um atributo absoluto.

### MVP e risco

Adicionar confiança e `last_success` aos registros existentes; implementar `evaluate_memory`; rebaixar ou revalidar memórias ligadas a falhas repetidas. O maior risco é uma política mal calibrada apagar informação útil ou criar custo excessivo de verificação.

## Fundamentos transversais de protocolo

| Tema | Convenção proposta |
|---|---|
| Discovery MCP | recurso `memory/capabilities` e tags de tipo de memória |
| ACL | proprietário, leitores, escritores, tenant e escopo |
| Proveniência | ferramenta-fonte, agente-fonte, instante, confiança, verificações |
| Reconsolidação | criar versão nova e manter vínculo com a evidência anterior |
| Observabilidade | eventos imutáveis, correlação por tarefa e auditoria de acesso |

## Sequência recomendada

1. Construir a fronteira 1 no runtime: episódios, proveniência e consolidação.
2. Adicionar a fronteira 3: qualidade, conflito e expiração sob métricas.
3. Expor a fronteira 2 depois que isolamento, ACL e auditoria estiverem testados.

Essa ordem reduz risco: não se deve compartilhar entre agentes uma memória cuja qualidade e controle de acesso ainda não foram demonstrados.
