# Aprendizado Contínuo

## Problema

Atualizar o comportamento do agente pode causar esquecimento catastrófico, reforçar erros e introduzir regressões silenciosas.

## Estratégia em camadas

1. **Não paramétrico:** escrever episódios, fatos validados e procedimentos.
2. **Extração de habilidades:** converter trajetórias bem-sucedidas em receitas com condições de aplicabilidade.
3. **Replay curado:** equilibrar tarefas antigas, novas e casos de falha.
4. **Atualização paramétrica opcional:** adaptar modelos somente após avaliação offline, versionamento e rollback.

## Proteções

- Separar dado observado, inferência e preferência.
- Exigir evidência repetida ou revisão antes de promover uma lição a procedimento.
- Rodar regressões por competência antes de publicar versão nova.
- Manter baseline e rollback atômico.

## Métricas

Retenção de tarefas antigas, ganho em tarefas novas, transferência positiva, taxa de regressão, custo de dados e taxa de correção humana.
