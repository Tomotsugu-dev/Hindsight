# Resource Usage

Hindsight’s memory and CPU usage mainly depend on three things: whether screenshots are enabled, whether background OCR is enabled, and whether the main window is being displayed.

- [Windows results](#windows-results)
- [macOS results](#macos-results)
- [Test conditions](#test-conditions)
- [Detailed data](#detailed-data)
  - [Windows data](#windows-data)
    - [Each configuration](#each-configuration)
    - [Per process](#per-process)
    - [Short peaks](#short-peaks)
  - [macOS data](#macos-data)
    - [Each configuration](#each-configuration-1)
    - [Per process](#per-process-1)
    - [Short peaks](#short-peaks-1)

## Windows results

| Settings | Main window open | Main window closed |
| --- | --- | --- |
| Screenshots and background OCR off (default) | About 145 MB memory, 2.0% CPU | About 25 MB memory, 0.3% CPU |
| Screenshots only | About 145 MB memory, 3.5% CPU | About 26 MB memory, 2.4% CPU |
| Screenshots and background OCR | About 400 MB memory, 4.1% CPU | About 280 MB memory, 3.0% CPU |

> 1. Memory figures use the same metric as Task Manager’s “Memory” column: the private working set.
> 2. CPU usage is the average; 100% means one core is fully utilized.
> 3. Power was not measured because Windows does not provide per-process power consumption data.
> 4. GPU usage is negligible, less than 0.3% in total.
> 5. When the main window is closed, the Windows version keeps its web processes in a suspended state. Their memory usage drops from around 130 MB to under 10 MB within about 30 seconds.

## macOS results

| Settings | Main window open | Main window closed |
| --- | --- | --- |
| Screenshots and background OCR off (default) | About 180 MB memory, 3.6% CPU, 5 mW power | About 40 MB memory, 0.2% CPU, under 1 mW power |
| Screenshots only | About 190 MB memory, 3.9% CPU, 30 mW power | About 50 MB memory, 0.7% CPU, 30 mW power |
| Screenshots and background OCR | About 260 MB memory, 4.5% CPU, 35 mW power | About 110 MB memory, 1.5% CPU, 40 mW power |

> 1. CPU usage is the average; 100% means one core is fully utilized.
> 2. Power is the average per-process estimate reported by macOS. It is useful for comparing configurations, but does not equal actual power consumption.
> 3. GPU usage is negligible, less than 0.1% in total.

## Test conditions

- Windows: desktop PC with an AMD Ryzen 9 9950X3D, 64 GB memory, Windows 11 Pro, Hindsight 0.9.4, and a screenshot interval of 30 seconds. During the test, text was typed automatically in Notepad, with the active app switching between Notepad, File Explorer, and Calculator.
- macOS: MacBook Air with Apple M5, 24 GB memory, macOS 27, plugged in, running Hindsight 0.9.3 with a screenshot interval of 30 seconds. During the test, text was typed automatically in TextEdit.
- The active app was switched every 10–25 seconds.
- Each configuration ran for 2 minutes before measurements were taken for 5 minutes.
- Hindsight was restarted before each configuration. Memory usage may be higher after a long continuous run.

These figures reflect the devices and usage patterns tested here; results will vary on other computers.

## Detailed data

### Windows data

#### Each configuration

| Settings | Main window | Memory average / peak | CPU average / peak | GPU | Screenshots | OCR |
| --- | --- | --- | --- | --- | --- | --- |
| Screenshots and background OCR off (default) | 🟠 Closed | 25 / 28 MB | 0.3% / 8% | 0 | 0 | 0 |
| Screenshots only | 🟠 Closed | 26 / 38 MB | 2.4% / 34% | 0 | 16 | 0 |
| Screenshots and background OCR | 🟠 Closed | 280 / 297 MB | 3.0% / 38% | 0.25% | 16 | 16 |
| Screenshots and background OCR off (default) | 🟢 Open | 145 / 152 MB | 2.0% / 9% | 0 | 0 | 0 |
| Screenshots only | 🟢 Open | 145 / 155 MB | 3.5% / 31% | 0 | 16 | 0 |
| Screenshots and background OCR | 🟢 Open | 400 / 405 MB | 4.1% / 31% | 0.23% | 16 | 16 |

> 1. Peaks are the highest readings from samples taken every 2 seconds.
> 2. The last two columns count screenshots captured and screenshots processed by OCR during the 5-minute measurement period.

#### Per process

| Process | When it runs | Average memory (peak) | Average CPU |
| --- | --- | --- | --- |
| Hindsight main process | Always | About 17 MB (28 MB) | 1.8%–2.4% with screenshots enabled; about 0.3% when disabled |
| Text recognition process | When background OCR is enabled | About 255 MB (270 MB) | 0.9%–1.0% |
| Main window web processes (7 in total) | Always | 127–129 MB (137 MB) with the window open; 7–9 MB (12 MB) when closed | 1.3%–1.7% with the window open; about 0 when closed |

> 1. The web processes belong to the system’s WebView2 runtime, which displays the main window.
> 2. The text recognition process also has a commit size of about 2.3 GB, shown on Task Manager’s “Details” tab. Most of this does not occupy physical memory.

#### Short peaks

| Activity | CPU peak |
| --- | --- |
| Capturing one screenshot | About 21%–34% in the main process |
| Recognizing text in one screenshot | About 30%–38% in the text recognition process |

> Peaks are averages over 2 seconds. Instantaneous peaks can be higher and last for less time.

### macOS data

#### Each configuration

| Settings | Main window | Memory average / peak | CPU average / peak | GPU | Power | Screenshots | OCR |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Screenshots and background OCR off (default) | 🟠 Closed | 40 / 54 MB | 0.2% / 5% | 0 | 0.7 mW | 0 | 0 |
| Screenshots only | 🟠 Closed | 48 / 64 MB | 0.7% / 7% | 0 | 29 mW | 16 | 0 |
| Screenshots and background OCR | 🟠 Closed | 112 / 144 MB | 1.5% / 31% | 0 | 39 mW | 16 | 16 |
| Screenshots and background OCR off (default) | 🟢 Open | 181 / 290 MB | 3.6% / 12% | 0.06% | 5 mW | 0 | 0 |
| Screenshots only | 🟢 Open | 193 / 304 MB | 3.9% / 18% | 0.07% | 31 mW | 17 | 0 |
| Screenshots and background OCR | 🟢 Open | 256 / 347 MB | 4.5% / 25% | 0.06% | 34 mW | 16 | 16 |

> 1. Peaks are the highest readings from samples taken every 2 seconds.
> 2. The last two columns count screenshots captured and screenshots processed by OCR during the 5-minute measurement period.

#### Per process

| Process | When it runs | Average memory (peak) | Average CPU |
| --- | --- | --- | --- |
| Hindsight main process | Always | 34–44 MB (50 MB) | 1.5%–1.8% with the main window open; 0.2%–0.7% when closed |
| Text recognition process | When background OCR is enabled | 66–78 MB (166 MB) | 0.6%–0.8% |
| Main window web content process | When the main window is open | 118–122 MB (186 MB) | About 1.9% |
| Main window graphics process | Always | 18–21 MB (75 MB) when open; about 1.5 MB when closed | About 0.3% when open; 0 when closed |
| Network process | Always | About 5 MB | 0 |

> The last three are WebKit system processes used to display the main window.

#### Short peaks

| Activity | CPU peak |
| --- | --- |
| Capturing one screenshot | About 7% |
| Recognizing text in one screenshot | 25%–31% |
| On first enabling background OCR, recognizing 31 queued screenshots | 124% (finished in under 2 minutes; average power was about 220 mW during those 2 minutes) |

> Peaks are averages over 2 seconds. Instantaneous peaks can be higher and last for less time.
