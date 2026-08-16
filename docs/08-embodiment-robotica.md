# Embodiment e Robótica

## Princípio

Embodiment exige ciclo fechado percepção–estado–ação–feedback. Linguagem pode especificar metas, mas não substitui estimação de estado, controle de baixo nível e barreiras de segurança.

## Caminho técnico

1. Ferramentas digitais com efeitos observáveis.
2. Ambiente simulado com tarefas, física e sensores sintéticos.
3. Política de alto nível que emite habilidades parametrizadas.
4. Controlador especializado para trajetórias e limites físicos.
5. Hardware com supervisão, parada de emergência e zona segura.

## Arquitetura

O agente cognitivo é deliberativo: seleciona metas e habilidades. O controlador local é reativo: estabiliza movimento, trata latência e rejeita comandos inseguros. O world model estima estado e prevê consequências.

## Avaliação

Sucesso por tarefa, violações de segurança, robustez a perturbação, generalização a cenários novos e diferença simulação–mundo real.
