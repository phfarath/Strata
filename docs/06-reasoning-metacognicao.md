# Reasoning e Metacognição

## Reasoning

Reasoning é busca guiada sobre hipóteses, ações e provas; não é apenas gerar uma resposta longa. O runtime combina geração pelo LLM, ferramentas de cálculo ou busca e um verificador independente.

## Métodos

- Decomposição em subproblemas e planos hierárquicos.
- Self-consistency para amostrar soluções e buscar concordância.
- Busca em árvore ou grafo para problemas com ramos observáveis.
- Verificadores determinísticos, testes, compiladores e regras de domínio.

## Metacognição operacional

O agente estima confiança em resposta, plano, memória e ação. Baixa confiança em etapa crítica dispara recuperação adicional, coleta de informação, verificação independente ou escalonamento humano.

## Calibração

Avaliar se probabilidades declaradas correspondem à frequência de acerto. Use Brier score, expected calibration error, taxa de abstenção correta e falsos positivos de confiança.

## Regra de decisão

Valor esperado = progresso esperado − custo − risco. Ações irreversíveis exigem limiar mais alto de confiança e autorização explícita.
