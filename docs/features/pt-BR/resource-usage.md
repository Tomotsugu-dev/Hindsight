# Uso de recursos

O uso de memória e CPU do Hindsight depende principalmente de três fatores: se as capturas de tela estão ativadas, se o OCR contínuo está ativado e se a janela principal está aberta.

- [Resultados no Windows](#resultados-no-windows)
- [Resultados no macOS](#resultados-no-macos)
- [Condições do teste](#condições-do-teste)
- [Dados detalhados](#dados-detalhados)
  - [Dados do Windows](#dados-do-windows)
  - [Dados do macOS](#dados-do-macos)
    - [Cada configuração](#cada-configuração)
    - [Por processo](#por-processo)
    - [Picos breves](#picos-breves)

## Resultados no Windows

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

- MacBook Air com Apple M5, 24 GB de memória, macOS 27, conectado à tomada.
- Hindsight 0.9.3, intervalo entre capturas de 30 segundos.
- Durante o teste, o texto foi digitado automaticamente em um editor de texto, e o aplicativo ativo foi alternado a cada 10–25 segundos.
- Cada configuração foi executada por 2 minutos antes de uma medição de 5 minutos.
- O Hindsight foi reiniciado antes do teste de cada configuração. Após muito tempo de execução contínua, o uso de memória pode ficar acima dos valores da tabela.

Os números representam este dispositivo e esta situação de uso; os resultados podem variar em outros computadores.

## Dados detalhados

### Dados do Windows

### Dados do macOS

#### Cada configuração

| Configuração | Janela principal | Memória média / pico | CPU média / pico | GPU | Potência | Capturas | Reconhecimentos |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Capturas e OCR contínuo desativados (padrão) | Aberta | 181 / 290 MB | 3,6% / 12% | 0,06% | 5 mW | 0 | 0 |
| Capturas e OCR contínuo desativados (padrão) | Fechada | 40 / 54 MB | 0,2% / 5% | 0 | 0,7 mW | 0 | 0 |
| Apenas capturas ativadas | Aberta | 193 / 304 MB | 3,9% / 18% | 0,07% | 31 mW | 17 | 0 |
| Apenas capturas ativadas | Fechada | 48 / 64 MB | 0,7% / 7% | 0 | 29 mW | 16 | 0 |
| Capturas e OCR contínuo ativados | Aberta | 256 / 347 MB | 4,5% / 25% | 0,06% | 34 mW | 16 | 16 |
| Capturas e OCR contínuo ativados | Fechada | 112 / 144 MB | 1,5% / 31% | 0 | 39 mW | 16 | 16 |

> 1. O pico é o maior valor entre as amostras coletadas a cada 2 segundos.
> 2. As quantidades de capturas e reconhecimentos correspondem aos 5 minutos de medição.

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
| Capturar uma imagem | Cerca de 7% |
| Reconhecer o texto de uma captura | 25%–31% |
| Ao ativar o OCR contínuo, reconhecer as 31 capturas acumuladas | 124% (concluído em menos de 2 minutos; a potência média nesses 2 minutos foi de cerca de 220 mW) |

> Os picos são médias de 2 segundos. Os valores instantâneos podem ser maiores e durar menos.
