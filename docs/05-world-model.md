# World Model

## Função

O world model mantém crenças sobre entidades, estados, ações e efeitos. O primeiro runtime deve começar com um modelo explícito, probabilístico e orientado a eventos; é mais auditável que uma representação latente.

## Representação inicial

Um grafo de crenças: nós para entidades e estados; arestas para relações, causas, pré-condições e efeitos. Cada afirmação inclui confiança, evidências, escopo temporal e hipóteses concorrentes.

## Atualização

Após observação, o sistema associa entidades, compara previsão ao resultado, atualiza confiança, registra erro de previsão e abre investigação quando há contradição relevante.

## Planejamento contrafactual

Para uma ação candidata, estimar pré-condições satisfeitas, efeitos esperados, risco, reversibilidade, custo e informação obtida. Preferir ações que avancem a meta ou reduzam incerteza crítica.

## Evolução

Migrar para modelos de estado latente, como RSSM/Dreamer, quando o domínio exigir previsão contínua de alta dimensão, como visão e controle robótico.
