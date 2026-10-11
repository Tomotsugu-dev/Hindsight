# Gestión de datos

- [Descripción general](#descripción-general)
- [Exportación de datos](#exportación-de-datos)
- [Limpieza de datos](#limpieza-de-datos)
- [Comportamiento predeterminado](#comportamiento-predeterminado)

## Descripción general

Hindsight permite ajustar la retención de los datos y exportar estadísticas en varios formatos para analizarlas después.

## Exportación de datos

Exporta las estadísticas diarias, semanales y mensuales a archivos. Elige un intervalo de fechas y un formato en «Ajustes → Datos → Exportar los datos de uso». Admite HTML, Markdown, Excel y JSON.

## Limpieza de datos

- **Limpieza automática:** Las capturas se eliminan al superar el plazo de retención. Los registros de actividad y el texto de pantalla reconocido no se borran automáticamente.
- **Borrar las capturas de pantalla:** Elimina todos los archivos de captura guardados; conserva los registros de actividad y el texto de pantalla reconocido.
- **Borrar los datos:** Vacía los registros de actividad, resúmenes de IA, texto de pantalla e historial del chat de la base de datos local. Conserva la configuración de aplicaciones (categorías, grupos de aplicaciones y ajustes). No afecta a los datos de la nube.
- **Quitar este dispositivo:** Borra los datos locales y elimina de la nube los registros de este dispositivo; sirve para dispositivos que ya no utilices.

## Comportamiento predeterminado

| Elemento | Predeterminado | Descripción |
| --- | --- | --- |
| Carpeta de datos | Carpeta `Hindsight` del directorio de datos de aplicaciones del sistema | `~/Library/Application Support/Hindsight` en macOS y `%APPDATA%\Hindsight` en Windows |
| Carpeta de capturas de pantalla | Carpeta `screenshots` dentro de la carpeta de datos | - |
| Carpeta de modelos de IA | Carpeta `ai/models` dentro de la carpeta de datos | - |
| Retención de las capturas de pantalla | 7 días | Se puede configurar entre 1–120 días; los archivos se eliminan al vencer el plazo |
| Retención de registros de actividad | Permanente | No se borran automáticamente |
