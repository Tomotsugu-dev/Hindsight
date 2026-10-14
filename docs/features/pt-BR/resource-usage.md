# Uso de recursos

O uso de memória e CPU do Hindsight depende principalmente de três fatores: se as capturas de tela estão ativadas, se o OCR contínuo está ativado e se a janela principal está sendo exibida.

- [Resultados no Windows](#resultados-no-windows)
- [Resultados no macOS](#resultados-no-macos)
- [Condições do teste](#condições-do-teste)
- [Dados detalhados](#dados-detalhados)
  - [Dados do Windows](#dados-do-windows)
    - [Cada configuração](#cada-configuração)
    - [Por processo](#por-processo)
    - [Picos breves](#picos-breves)
  - [Dados do macOS](#dados-do-macos)
    - [Cada configuração](#cada-configuração-1)
    - [Por processo](#por-processo-1)
    - [Picos breves](#picos-breves-1)

## Resultados no Windows

| Configuração | Janela principal aberta | Janela principal fechada |
| --- | --- | --- |
| Capturas e OCR contínuo desativados (padrão) | Cerca de 145 MB de memória e 2,0% de CPU | Cerca de 25 MB de memória e 0,3% de CPU |
| Apenas capturas ativadas | Cerca de 145 MB de memória e 3,5% de CPU | Cerca de 26 MB de memória e 2,4% de CPU |
| Capturas e OCR contínuo ativados | Cerca de 400 MB de memória e 4,1% de CPU | Cerca de 280 MB de memória e 3,0% de CPU |

> 1. Os valores de memória usam a mesma métrica da coluna “Memória” do Gerenciador de Tarefas: o conjunto de trabalho privado.
> 2. O uso de CPU é uma média; 100% significa que um núcleo está totalmente ocupado.
> 3. A potência não foi medida porque o Windows não fornece dados de consumo de energia por processo.
> 4. O uso de GPU é quase nulo, menos de 0,3% no total.
> 5. Ao fechar a janela principal, a versão para Windows suspende os processos web sem encerrá-los. O uso de memória desses processos cai de cerca de 130 MB para menos de 10 MB em aproximadamente 30 segundos.

## Resultados no macOS

| Configuração | Janela principal aberta | Janela principal fechada |
| --- | --- | --- |
| Capturas e OCR contínuo desativados (padrão) | Cerca de 180 MB de memória, 3,6% de CPU e 5 mW de potência | Cerca de 40 MB de memória, 0,2% de CPU e menos de 1 mW de potência |
| Apenas capturas ativadas | Cerca de 190 MB de memória, 3,9% de CPU e 30 mW de potência | Cerca de 50 MB de memória, 0,7% de CPU e 30 mW de potência |
| Capturas e OCR contínuo ativados | Cerca de 260 MB de memória, 4,5% de CPU e 35 mW de potência | Cerca de 110 MB de memória, 1,5% de CPU e 40 mW de potência |

> 1. O uso de CPU é uma média; 100% significa que um núcleo está totalmente ocupado.
> 2. A potência é a estimativa média por processo informada pelo macOS. Ela serve para comparar configurações, mas não equivale ao consumo real de energia.
> 3. O uso de GPU é quase nulo, menos de 0,1% no total.

## Condições do teste

- Windows: computador de mesa com AMD Ryzen 9 9950X3D, 64 GB de memória, Windows 11 Pro, Hindsight 0.9.4 e intervalo entre capturas de 30 segundos. Durante o teste, o texto foi digitado automaticamente no Bloco de Notas, alternando entre o Bloco de Notas, o Explorador de Arquivos e a Calculadora.
- macOS: MacBook Air com Apple M5, 24 GB de memória, macOS 27, conectado à tomada, com Hindsight 0.9.3 e intervalo entre capturas de 30 segundos. Durante o teste, o texto foi digitado automaticamente no TextEdit.
- O aplicativo ativo foi alternado a cada 10–25 segundos.
- Cada configuração foi executada por 2 minutos antes de uma medição de 5 minutos.
- O Hindsight foi reiniciado antes do teste de cada configuração. Após muito tempo de execução contínua, o uso de memória pode ficar acima dos valores da tabela.

Os números representam os dispositivos e as condições de uso destes testes; os resultados podem variar em outros computadores.

## Dados detalhados

### Dados do Windows

#### Cada configuração

| Configuração | Janela principal | Memória média / pico | CPU média / pico | GPU | Capturas | OCR |
| --- | --- | --- | --- | --- | --- | --- |
| Capturas e OCR contínuo desativados (padrão) | 🟠 Fechada | 25 / 28 MB | 0,3% / 8% | 0 | 0 | 0 |
| Apenas capturas ativadas | 🟠 Fechada | 26 / 38 MB | 2,4% / 34% | 0 | 16 | 0 |
| Capturas e OCR contínuo ativados | 🟠 Fechada | 280 / 297 MB | 3,0% / 38% | 0,25% | 16 | 16 |
| Capturas e OCR contínuo desativados (padrão) | 🟢 Aberta | 145 / 152 MB | 2,0% / 9% | 0 | 0 | 0 |
| Apenas capturas ativadas | 🟢 Aberta | 145 / 155 MB | 3,5% / 31% | 0 | 16 | 0 |
| Capturas e OCR contínuo ativados | 🟢 Aberta | 400 / 405 MB | 4,1% / 31% | 0,23% | 16 | 16 |

> 1. O pico é o maior valor entre as amostras coletadas a cada 2 segundos.
> 2. As duas últimas colunas mostram quantas capturas foram feitas e quantas foram processadas pelo OCR durante os 5 minutos de medição.

#### Por processo

| Processo | Quando é executado | Memória média (pico) | CPU média |
| --- | --- | --- | --- |
| Processo principal do Hindsight | Sempre | Cerca de 17 MB (28 MB) | 1,8%–2,4% com as capturas ativadas; cerca de 0,3% com as capturas desativadas |
| Processo de reconhecimento de texto | Quando o OCR contínuo está ativado | Cerca de 255 MB (270 MB) | 0,9%–1,0% |
| Processos web da janela principal (7 no total) | Sempre | 127–129 MB (137 MB) com a janela aberta; 7–9 MB (12 MB) com a janela fechada | 1,3%–1,7% com a janela aberta; aproximadamente 0 com a janela fechada |

> 1. Os processos web pertencem ao WebView2 do sistema, responsável por exibir a janela principal.
> 2. O processo de reconhecimento de texto também tem cerca de 2,3 GB de memória comprometida (“Commit size”, na aba de detalhes do Gerenciador de Tarefas). A maior parte não ocupa memória física.

#### Picos breves

| Atividade | Pico de CPU |
| --- | --- |
| Fazer uma captura de tela | Cerca de 21%–34% no processo principal |
| Reconhecer o texto de uma captura | Cerca de 30%–38% no processo de reconhecimento de texto |

> Os picos são médias de 2 segundos. Os valores instantâneos podem ser maiores e durar menos.

### Dados do macOS

#### Cada configuração

| Configuração | Janela principal | Memória média / pico | CPU média / pico | GPU | Potência | Capturas | OCR |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Capturas e OCR contínuo desativados (padrão) | 🟠 Fechada | 40 / 54 MB | 0,2% / 5% | 0 | 0,7 mW | 0 | 0 |
| Apenas capturas ativadas | 🟠 Fechada | 48 / 64 MB | 0,7% / 7% | 0 | 29 mW | 16 | 0 |
| Capturas e OCR contínuo ativados | 🟠 Fechada | 112 / 144 MB | 1,5% / 31% | 0 | 39 mW | 16 | 16 |
| Capturas e OCR contínuo desativados (padrão) | 🟢 Aberta | 181 / 290 MB | 3,6% / 12% | 0,06% | 5 mW | 0 | 0 |
| Apenas capturas ativadas | 🟢 Aberta | 193 / 304 MB | 3,9% / 18% | 0,07% | 31 mW | 17 | 0 |
| Capturas e OCR contínuo ativados | 🟢 Aberta | 256 / 347 MB | 4,5% / 25% | 0,06% | 34 mW | 16 | 16 |

> 1. O pico é o maior valor entre as amostras coletadas a cada 2 segundos.
> 2. As duas últimas colunas mostram quantas capturas foram feitas e quantas foram processadas pelo OCR durante os 5 minutos de medição.

#### Por processo

| Processo | Quando é executado | Memória média (pico) | CPU média |
| --- | --- | --- | --- |
| Processo principal do Hindsight | Sempre | 34–44 MB (50 MB) | 1,5%–1,8% com a janela aberta; 0,2%–0,7% fechada |
| Processo de reconhecimento de texto | Quando o OCR contínuo está ativado | 66–78 MB (166 MB) | 0,6%–0,8% |
| Processo de conteúdo web da janela principal | Quando a janela principal está aberta | 118–122 MB (186 MB) | Cerca de 1,9% |
| Processo gráfico da janela principal | Sempre | 18–21 MB (75 MB) aberta; cerca de 1,5 MB fechada | Cerca de 0,3% aberta; 0 fechada |
| Processo de rede | Sempre | Cerca de 5 MB | 0 |

> Os três últimos são processos do sistema WebKit, usados para exibir a janela principal.

#### Picos breves

| Atividade | Pico de CPU |
| --- | --- |
| Fazer uma captura de tela | Cerca de 7% |
| Reconhecer o texto de uma captura | 25%–31% |
| Ao ativar o OCR contínuo, reconhecer as 31 capturas acumuladas | 124% (concluído em menos de 2 minutos; a potência média nesses 2 minutos foi de cerca de 220 mW) |

> Os picos são médias de 2 segundos. Os valores instantâneos podem ser maiores e durar menos.
