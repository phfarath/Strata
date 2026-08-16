# Memória

## Tipos

| Tipo | Conteúdo | Escrita | Recuperação |
|---|---|---|---|
| Trabalho | contexto da tarefa | a cada ciclo | estado atual |
| Episódica | eventos, decisões e resultados | após ação | similaridade + tempo |
| Semântica | fatos e relações com fontes | após validação | entidades + grafo |
| Procedural | receitas e políticas | após repetição avaliada | intenção + pré-condições |

## Modelo de registro

Cada item possui identificador, conteúdo estruturado, embedding opcional, entidades, fonte, instante, confiança, importância, acesso recente, validade e ligações de evidência/contradição.

## Recuperação híbrida

Combine relevância semântica, correspondência de entidades, recência, importância, sucesso histórico e penalidade por baixa confiança. Diversifique resultados para não preencher o contexto com cópias do mesmo episódio.

## Consolidação e esquecimento

Eventos brutos permanecem imutáveis. Consolidação cria resumos e fatos derivados, nunca substitui evidência. Esquecer reduz prioridade, expira uma crença ou arquiva conteúdo, preservando a trilha de decisões relevantes.

## Experimentos

- Recordação factual após 10, 100 e 1.000 ciclos.
- Interferência entre tarefas parecidas.
- Comparação entre RAG vetorial, grafo e recuperação híbrida.
- Taxa de memórias inválidas recuperadas.
