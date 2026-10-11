# Privacidade

- [Visão geral](#visão-geral)
- [Filtros de privacidade](#filtros-de-privacidade)
- [Comportamento padrão](#comportamento-padrão)

## Visão geral

Os ajustes de privacidade permitem excluir aplicativos, janelas e páginas das capturas de tela. Acesse “Ajustes → Privacidade”.

## Filtros de privacidade

Adicione palavras-chave para excluir das capturas as janelas e páginas correspondentes.

- **Filtro de aplicativos:** não captura se o nome do aplicativo ou o título da janela contiver uma palavra-chave.
- **Filtro de navegador:** não captura se a URL na barra de endereços contiver uma palavra-chave.
- **Registrar domínios de sites:** registra apenas o site visitado, como `github.com`, sem identificar a página específica. Por exemplo, ao abrir `https://github.com/xxx/issues/1`, registra apenas `github.com`.

## Comportamento padrão

| Item | Padrão | Observações |
| --- | --- | --- |
| Capturas de tela | Desativadas | Ative-as em “Ajustes → Geral” |
| Filtro de navegador | 10 palavras-chave relacionadas a login | Inclui `/login`, `/signin`, `/auth`, `/oauth` e `/password`; você pode adicionar ou remover itens |
| Filtro de aplicativos | Vazio | - |
| Registrar domínios de sites | Ativado | - |
