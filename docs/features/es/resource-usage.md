# Uso de recursos

El uso de memoria y CPU de Hindsight depende principalmente de tres cosas: si están activadas las capturas de pantalla, si está activado el OCR en segundo plano y si la ventana principal está abierta.

- [Resultados en Windows](#resultados-en-windows)
- [Resultados en macOS](#resultados-en-macos)
- [Condiciones de la prueba](#condiciones-de-la-prueba)
- [Datos detallados](#datos-detallados)
  - [Datos de Windows](#datos-de-windows)
  - [Datos de macOS](#datos-de-macos)
    - [Cada configuración](#cada-configuración)
    - [Por proceso](#por-proceso)
    - [Picos breves](#picos-breves)

## Resultados en Windows

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

- MacBook Air con Apple M5, 24 GB de memoria, macOS 27 y conectado a la corriente.
- Hindsight 0.9.3, con un intervalo de captura de 30 segundos.
- Durante la prueba se escribió texto automáticamente en un editor y se cambió de aplicación cada 10–25 segundos.
- Cada configuración se ejecutó primero durante 2 minutos y después se midió durante 5 minutos.
- Se reinició Hindsight antes de probar cada configuración. Tras mucho tiempo de uso continuo, la memoria puede superar los valores de la tabla.

Las cifras corresponden a este equipo y a esta prueba; pueden variar en otros ordenadores.

## Datos detallados

### Datos de Windows

### Datos de macOS

#### Cada configuración

| Ajustes | Ventana principal | Memoria promedio / máximo | CPU promedio / máximo | GPU | Potencia | Capturas | Reconocimientos |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Capturas y OCR en segundo plano desactivados (predeterminado) | Abierta | 181 / 290 MB | 3,6% / 12% | 0,06% | 5 mW | 0 | 0 |
| Capturas y OCR en segundo plano desactivados (predeterminado) | Cerrada | 40 / 54 MB | 0,2% / 5% | 0 | 0,7 mW | 0 | 0 |
| Solo capturas | Abierta | 193 / 304 MB | 3,9% / 18% | 0,07% | 31 mW | 17 | 0 |
| Solo capturas | Cerrada | 48 / 64 MB | 0,7% / 7% | 0 | 29 mW | 16 | 0 |
| Capturas y OCR en segundo plano | Abierta | 256 / 347 MB | 4,5% / 25% | 0,06% | 34 mW | 16 | 16 |
| Capturas y OCR en segundo plano | Cerrada | 112 / 144 MB | 1,5% / 31% | 0 | 39 mW | 16 | 16 |

> 1. El máximo es la lectura más alta entre las muestras tomadas cada 2 segundos.
> 2. Las capturas y los reconocimientos son el total durante los 5 minutos de medición.

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
| Capturar una imagen | Un 7% aprox. |
| Reconocer el texto de una captura | 25%–31% |
| Al activar el OCR en segundo plano, reconocer las 31 capturas acumuladas | 124% (terminó en menos de 2 minutos; la potencia promedio durante esos 2 minutos fue de unos 220 mW) |

> Los picos son promedios de 2 segundos. Los picos instantáneos pueden ser más altos y durar menos.
