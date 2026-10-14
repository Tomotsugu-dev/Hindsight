# Uso de recursos

El uso de memoria y CPU de Hindsight depende principalmente de tres factores: si están activadas las capturas de pantalla, si está activado el OCR en segundo plano y si se está mostrando la ventana principal.

- [Resultados en Windows](#resultados-en-windows)
- [Resultados en macOS](#resultados-en-macos)
- [Condiciones de la prueba](#condiciones-de-la-prueba)
- [Datos detallados](#datos-detallados)
  - [Datos de Windows](#datos-de-windows)
    - [Cada configuración](#cada-configuración)
    - [Por proceso](#por-proceso)
    - [Picos breves](#picos-breves)
  - [Datos de macOS](#datos-de-macos)
    - [Cada configuración](#cada-configuración-1)
    - [Por proceso](#por-proceso-1)
    - [Picos breves](#picos-breves-1)

## Resultados en Windows

| Ajustes | Ventana principal abierta | Ventana principal cerrada |
| --- | --- | --- |
| Capturas y OCR en segundo plano desactivados (predeterminado) | Unos 145 MB de memoria y 2,0% de CPU | Unos 25 MB de memoria y 0,3% de CPU |
| Solo capturas | Unos 145 MB de memoria y 3,5% de CPU | Unos 26 MB de memoria y 2,4% de CPU |
| Capturas y OCR en segundo plano | Unos 400 MB de memoria y 4,1% de CPU | Unos 280 MB de memoria y 3,0% de CPU |

> 1. Las cifras de memoria corresponden a la columna «Memoria» del Administrador de tareas: el conjunto de trabajo privado.
> 2. El uso de CPU es el promedio; 100% equivale a un núcleo a plena carga.
> 3. No se midió la potencia porque Windows no proporciona datos de consumo eléctrico por proceso.
> 4. El uso de GPU es prácticamente nulo: menos del 0,3% en total.
> 5. Al cerrar la ventana principal, la versión de Windows suspende los procesos web sin finalizarlos. Su uso de memoria baja de unos 130 MB a menos de 10 MB en unos 30 segundos.

## Resultados en macOS

| Ajustes | Ventana principal abierta | Ventana principal cerrada |
| --- | --- | --- |
| Capturas y OCR en segundo plano desactivados (predeterminado) | Unos 180 MB de memoria, 3,6% de CPU y 5 mW de potencia | Unos 40 MB de memoria, 0,2% de CPU y menos de 1 mW de potencia |
| Solo capturas | Unos 190 MB de memoria, 3,9% de CPU y 30 mW de potencia | Unos 50 MB de memoria, 0,7% de CPU y 30 mW de potencia |
| Capturas y OCR en segundo plano | Unos 260 MB de memoria, 4,5% de CPU y 35 mW de potencia | Unos 110 MB de memoria, 1,5% de CPU y 40 mW de potencia |

> 1. El uso de CPU es el promedio; 100% equivale a un núcleo a plena carga.
> 2. La potencia es el promedio estimado por proceso que muestra macOS. Sirve para comparar configuraciones, pero no equivale al consumo eléctrico real.
> 3. El uso de GPU es prácticamente nulo: menos del 0,1% en total.

## Condiciones de la prueba

- Windows: ordenador de sobremesa con AMD Ryzen 9 9950X3D, 64 GB de memoria, Windows 11 Pro, Hindsight 0.9.4 y un intervalo de captura de 30 segundos. Durante la prueba se escribió texto automáticamente en el Bloc de notas y se alternó entre el Bloc de notas, el Explorador de archivos y la Calculadora.
- macOS: MacBook Air con Apple M5, 24 GB de memoria, macOS 27, conectado a la corriente, con Hindsight 0.9.3 y un intervalo de captura de 30 segundos. Durante la prueba se escribió texto automáticamente en TextEdit.
- Se cambió de aplicación cada 10–25 segundos.
- Cada configuración se ejecutó primero durante 2 minutos y después se midió durante 5 minutos.
- Se reinició Hindsight antes de probar cada configuración. Tras mucho tiempo de uso continuo, la memoria puede superar los valores de la tabla.

Las cifras corresponden a los equipos y las condiciones de uso de estas pruebas; pueden variar en otros ordenadores.

## Datos detallados

### Datos de Windows

#### Cada configuración

| Ajustes | Ventana principal | Memoria promedio / máximo | CPU promedio / máximo | GPU | Capturas | OCR |
| --- | --- | --- | --- | --- | --- | --- |
| Capturas y OCR en segundo plano desactivados (predeterminado) | 🟠 Cerrada | 25 / 28 MB | 0,3% / 8% | 0 | 0 | 0 |
| Solo capturas | 🟠 Cerrada | 26 / 38 MB | 2,4% / 34% | 0 | 16 | 0 |
| Capturas y OCR en segundo plano | 🟠 Cerrada | 280 / 297 MB | 3,0% / 38% | 0,25% | 16 | 16 |
| Capturas y OCR en segundo plano desactivados (predeterminado) | 🟢 Abierta | 145 / 152 MB | 2,0% / 9% | 0 | 0 | 0 |
| Solo capturas | 🟢 Abierta | 145 / 155 MB | 3,5% / 31% | 0 | 16 | 0 |
| Capturas y OCR en segundo plano | 🟢 Abierta | 400 / 405 MB | 4,1% / 31% | 0,23% | 16 | 16 |

> 1. El máximo es la lectura más alta entre las muestras tomadas cada 2 segundos.
> 2. Las dos últimas columnas indican cuántas capturas se tomaron y cuántas se procesaron con OCR durante los 5 minutos de medición.

#### Por proceso

| Proceso | Cuándo se ejecuta | Memoria promedio (máximo) | CPU promedio |
| --- | --- | --- | --- |
| Proceso principal de Hindsight | Siempre | Unos 17 MB (28 MB) | 1,8%–2,4% con las capturas activadas; unos 0,3% con las capturas desactivadas |
| Proceso de reconocimiento de texto | Con el OCR en segundo plano activado | Unos 255 MB (270 MB) | 0,9%–1,0% |
| Procesos web de la ventana principal (7 en total) | Siempre | 127–129 MB (137 MB) con la ventana abierta; 7–9 MB (12 MB) con la ventana cerrada | 1,3%–1,7% con la ventana abierta; aproximadamente 0 con la ventana cerrada |

> 1. Los procesos web pertenecen al componente WebView2 del sistema, que se encarga de mostrar la ventana principal.
> 2. El proceso de reconocimiento de texto también tiene unos 2,3 GB de memoria comprometida («Commit size» en la pestaña de detalles del Administrador de tareas). La mayor parte no ocupa memoria física.

#### Picos breves

| Actividad | Pico de CPU |
| --- | --- |
| Tomar una captura de pantalla | Un 21%–34% aprox. en el proceso principal |
| Reconocer el texto de una captura | Un 30%–38% aprox. en el proceso de reconocimiento de texto |

> Los picos son promedios de 2 segundos. Los picos instantáneos pueden ser más altos y durar menos.

### Datos de macOS

#### Cada configuración

| Ajustes | Ventana principal | Memoria promedio / máximo | CPU promedio / máximo | GPU | Potencia | Capturas | OCR |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Capturas y OCR en segundo plano desactivados (predeterminado) | 🟠 Cerrada | 40 / 54 MB | 0,2% / 5% | 0 | 0,7 mW | 0 | 0 |
| Solo capturas | 🟠 Cerrada | 48 / 64 MB | 0,7% / 7% | 0 | 29 mW | 16 | 0 |
| Capturas y OCR en segundo plano | 🟠 Cerrada | 112 / 144 MB | 1,5% / 31% | 0 | 39 mW | 16 | 16 |
| Capturas y OCR en segundo plano desactivados (predeterminado) | 🟢 Abierta | 181 / 290 MB | 3,6% / 12% | 0,06% | 5 mW | 0 | 0 |
| Solo capturas | 🟢 Abierta | 193 / 304 MB | 3,9% / 18% | 0,07% | 31 mW | 17 | 0 |
| Capturas y OCR en segundo plano | 🟢 Abierta | 256 / 347 MB | 4,5% / 25% | 0,06% | 34 mW | 16 | 16 |

> 1. El máximo es la lectura más alta entre las muestras tomadas cada 2 segundos.
> 2. Las dos últimas columnas indican cuántas capturas se tomaron y cuántas se procesaron con OCR durante los 5 minutos de medición.

#### Por proceso

| Proceso | Cuándo se ejecuta | Memoria promedio (máximo) | CPU promedio |
| --- | --- | --- | --- |
| Proceso principal de Hindsight | Siempre | 34–44 MB (50 MB) | 1,5%–1,8% con la ventana abierta; 0,2%–0,7% cerrada |
| Proceso de reconocimiento de texto | Con el OCR en segundo plano activado | 66–78 MB (166 MB) | 0,6%–0,8% |
| Proceso web de la ventana principal | Con la ventana principal abierta | 118–122 MB (186 MB) | Un 1,9% aprox. |
| Proceso gráfico de la ventana principal | Siempre | 18–21 MB (75 MB) abierta; unos 1,5 MB cerrada | Un 0,3% aprox. abierta; 0 cerrada |
| Proceso de red | Siempre | Unos 5 MB | 0 |

> Los tres últimos son procesos del sistema WebKit, que se encarga de mostrar la ventana principal.

#### Picos breves

| Actividad | Pico de CPU |
| --- | --- |
| Tomar una captura de pantalla | Un 7% aprox. |
| Reconocer el texto de una captura | 25%–31% |
| Al activar el OCR en segundo plano, reconocer las 31 capturas acumuladas | 124% (terminó en menos de 2 minutos; la potencia promedio durante esos 2 minutos fue de unos 220 mW) |

> Los picos son promedios de 2 segundos. Los picos instantáneos pueden ser más altos y durar menos.
