# Ajustes de privacidad

- [Descripción general](#descripción-general)
- [Filtros de privacidad](#filtros-de-privacidad)
- [Comportamiento predeterminado](#comportamiento-predeterminado)

## Descripción general

Los ajustes de privacidad permiten excluir aplicaciones, ventanas y páginas web de las capturas de pantalla. Están en «Ajustes → Privacidad».

## Filtros de privacidad

Al añadir palabras clave, las ventanas y páginas que coincidan no se capturan.

- **Filtro de aplicaciones:** Omite la captura si el nombre de la aplicación o el título de la ventana contiene una palabra clave.
- **Filtro de navegador:** Omite la captura si la URL de la barra de direcciones contiene una palabra clave.
- **Registrar los dominios de los sitios:** Solo registra el sitio visitado (como `github.com`), no la página concreta. Por ejemplo, al abrir `https://github.com/xxx/issues/1`, solo guarda `github.com`.

## Comportamiento predeterminado

| Elemento | Predeterminado | Descripción |
| --- | --- | --- |
| Capturas de pantalla | Desactivadas | Deben activarse en «Ajustes → General» |
| Filtro de navegador | 10 palabras clave relacionadas con el inicio de sesión | Incluye `/login`, `/signin`, `/auth`, `/oauth` y `/password`; puedes añadirlas o quitarlas |
| Filtro de aplicaciones | Vacío | - |
| Registrar los dominios de los sitios | Activado | - |
