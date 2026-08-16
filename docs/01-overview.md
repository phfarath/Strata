# Overview

## Problema

Agentes baseados apenas em contexto de conversa têm memória limitada, não atualizam competências de forma controlada, não distinguem certeza de especulação e perdem coerência em tarefas longas. O Cognitive Agent Runtime adiciona uma camada persistente em volta do modelo.

## Objetivo

1. Manter memória com proveniência e esquecimento deliberado.
2. Planejar, agir, observar e corrigir-se em ciclos longos.
3. Aprender com experiência sem degradar capacidades anteriores.
4. Construir e atualizar crenças sobre o mundo por evidência.
5. Declarar incerteza e bloquear ações de risco elevado.
6. Operar em ferramentas digitais antes de avançar para simuladores físicos.

## Hipótese central

Separar memória, modelo do mundo, planejamento, verificação e execução reduz falhas de contexto e permite avaliar cada subsistema de forma mensurável.

## Não objetivos

- Treinar um modelo fundacional do zero.
- Autonomia irrestrita.
- Promover toda interação a memória permanente.
- Executar ações externas sem política, autorização e auditoria.
