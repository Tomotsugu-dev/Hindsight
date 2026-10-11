# Sincronização na nuvem

- [Visão geral](#visão-geral)
- [Protocolos compatíveis](#protocolos-compatíveis)
- [Dados sincronizados](#dados-sincronizados)
- [Comportamento padrão](#comportamento-padrão)

## Visão geral

O Hindsight sincroniza registros de atividade, configurações de aplicativos e outros dados entre dispositivos, reunindo um histórico completo do uso do computador em diferentes sistemas e aparelhos.

## Protocolos compatíveis

O Hindsight oferece os seguintes protocolos de sincronização:

- **WebDAV:** sincronização por um servidor WebDAV, próprio ou de terceiros, como Nextcloud ou Nutstore.
- **Google Drive:** sincronização pelo Google Drive para quem tem uma conta Google.

## Dados sincronizados

| Dados | Sincronizados? | Observações |
| --- | --- | --- |
| Registros de atividade (aplicativo, título da janela, horário, domínio do site) | ✅ Sim | - |
| Configurações de aplicativos (regras de categorias para aplicativos e sites) | ✅ Sim | - |
| Resumos de IA, histórico do chat, texto completo da memória de tela | ⚙️ Opcional | Cada item precisa ser ativado separadamente nos ajustes |
| Imagens das capturas de tela | ❌ Não | Sempre permanecem apenas neste dispositivo |

## Comportamento padrão

| Item | Padrão | Observações |
| --- | --- | --- |
| Sincronização na nuvem | Desativada | É necessário configurá-la |
| Intervalo de sincronização | Cerca de 5 minutos | Envia e recebe dados em segundo plano a cada 300 segundos; também é possível usar “Sincronizar agora” |
| Texto dos resumos de IA | Desativado | Só sincroniza após ativar “Texto dos resumos de IA” |
| Histórico do chat | Desativado | Só sincroniza após ativar “Histórico do chat” |
| Memória de tela (texto completo) | Desativada | Só sincroniza o texto reconhecido após ativar “Memória de tela (texto completo)” |
| Imagens das capturas de tela | Não sincronizadas | Sempre permanecem apenas neste dispositivo |
