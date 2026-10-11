# Sincronización en la nube

- [Descripción general](#descripción-general)
- [Protocolos compatibles](#protocolos-compatibles)
- [Datos sincronizados](#datos-sincronizados)
- [Comportamiento predeterminado](#comportamiento-predeterminado)

## Descripción general

La sincronización en la nube de Hindsight comparte registros de actividad, ajustes de aplicaciones y otros datos entre dispositivos, para reunir el uso del ordenador en distintas plataformas y equipos.

## Protocolos compatibles

Hindsight admite estos protocolos de sincronización:

- **WebDAV:** Sincronización mediante un servidor WebDAV, propio o de terceros, como Nextcloud o Nutstore.
- **Google Drive:** Sincronización mediante Google Drive para usuarios con una cuenta de Google.

## Datos sincronizados

| Datos | Sincronización | Descripción |
| --- | --- | --- |
| Registros de actividad (aplicación, título de ventana, tiempo, dominio del sitio) | ✅ Sí | - |
| Ajustes de aplicaciones (reglas de categorías de aplicaciones y sitios) | ✅ Sí | - |
| Resúmenes de IA, historial del chat, texto completo de la memoria de pantalla | ⚙️ Opcional | Cada uno debe activarse por separado en los ajustes |
| Imágenes de las capturas de pantalla | ❌ No | Siempre permanecen solo en el dispositivo |

## Comportamiento predeterminado

| Elemento | Predeterminado | Descripción |
| --- | --- | --- |
| Sincronización en la nube | Desactivada | Requiere configuración explícita |
| Intervalo de sincronización | Unos 5 minutos | Sube y descarga datos en segundo plano cada 300 segundos; también permite «Sincronizar ahora» |
| Texto de los resúmenes de IA | Desactivado | Solo se sincroniza al activar «Texto de los resúmenes de IA» |
| Historial del chat | Desactivado | Solo se sincroniza al activar «Historial del chat» |
| Memoria de pantalla (texto completo) | Desactivada | Solo se sincroniza el texto reconocido al activar «Memoria de pantalla (texto completo)» |
| Imágenes de las capturas de pantalla | No se sincronizan | Siempre permanecen solo en el dispositivo |
