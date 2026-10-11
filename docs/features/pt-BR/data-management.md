# Gerenciamento de dados

- [Visão geral](#visão-geral)
- [Exportação de dados](#exportação-de-dados)
- [Limpeza de dados](#limpeza-de-dados)
- [Comportamento padrão](#comportamento-padrão)

## Visão geral

O Hindsight permite definir prazos de retenção e exportar estatísticas em vários formatos para análise posterior.

## Exportação de dados

Exporte estatísticas diárias, semanais e mensais para arquivos. Em “Ajustes → Dados → Exportar dados de uso”, escolha o período e o formato: HTML, Markdown, Excel ou JSON.

## Limpeza de dados

- **Limpeza automática:** exclui capturas de tela após o prazo de retenção. Registros de atividade e textos reconhecidos da tela não são removidos automaticamente.
- **Limpar capturas de tela:** exclui todas as capturas salvas e mantém os registros de atividade e os textos reconhecidos.
- **Limpar dados:** apaga os registros de atividade, resumos de IA, textos da tela e histórico do chat do banco de dados local. Mantém as configurações de aplicativos (categorias, grupos de aplicativos e ajustes). Os dados na nuvem não são afetados.
- **Remover este dispositivo:** limpa os dados locais e remove da nuvem os registros deste dispositivo, para quando você deixar de usá-lo.

## Comportamento padrão

| Item | Padrão | Observações |
| --- | --- | --- |
| Pasta de dados | Pasta `Hindsight` no diretório de dados de aplicativos do sistema | No macOS: `~/Library/Application Support/Hindsight`; no Windows: `%APPDATA%\Hindsight` |
| Pasta de capturas de tela | Pasta `screenshots` dentro da pasta de dados | - |
| Pasta de modelos de IA | Pasta `ai/models` dentro da pasta de dados | - |
| Retenção de capturas de tela | 7 dias | Pode ser definida entre 1 e 120 dias; os arquivos são excluídos automaticamente ao expirar |
| Retenção de registros de atividade | Permanente | Não são removidos automaticamente |
