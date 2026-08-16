# Arquitetura Cognitiva

## Componentes

| Componente | Responsabilidade | Estado persistente |
|---|---|---|
| Orquestrador | ciclo observar–decidir–agir | execução e checkpoints |
| Memória | recuperar e consolidar experiência | episódios, fatos, habilidades |
| World model | crenças, previsões e relações causais | grafo de crenças |
| Planner | decompor metas e selecionar ações | DAG de subobjetivos |
| Reasoner/verifier | gerar e checar hipóteses | evidências e avaliações |
| Tool gateway | validar e executar ferramentas | permissões e logs |
| Learner | extrair lições e atualizar políticas | conjuntos de treino e versões |

## Ciclo de controle

1. Normalizar observação e objetivo em um estado tipado.
2. Recuperar memórias por significado, tempo, entidade e tarefa.
3. Atualizar crenças e estimar incerteza.
4. Propor plano ou próximo subobjetivo.
5. Verificar pré-condições, risco e permissão.
6. Executar ação reversível quando possível.
7. Registrar resultado, avaliar progresso e consolidar experiência.

## Invariantes

- Todo fato tem fonte, data, confiança e expiração.
- Toda ação externa possui identificador, autorização, pré-condições e resultado.
- Planos são versionados e checkpoints permitem retomada.
- O LLM recebe projeções compactas do estado, não o banco inteiro.
