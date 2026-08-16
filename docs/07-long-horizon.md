# Long-Horizon Autonomy

## Estrutura de objetivo

Representar cada objetivo como um DAG de subobjetivos com critérios de conclusão, dependências, orçamento, risco, evidências exigidas e política de retomada.

## Execução resiliente

- Checkpoint após toda transição material de estado.
- Idempotência para ações externas repetíveis.
- Detecção de bloqueio, repetição e desvio de escopo.
- Replanejamento local antes de invalidar um plano inteiro.
- Limites de tempo, custo, tentativas e permissões.

## Governança

Defina níveis: observação, simulação, ação reversível, ação externa limitada e ação irreversível. Cada nível possui autorização e mecanismo de parada.

## Métricas

Taxa de conclusão, passos por sucesso, recuperação após falha, violações de orçamento, intervenções humanas e deriva de objetivo.
