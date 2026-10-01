# WeFlow

[English](../weflow-readme.md) | [简体中文](../zh-CN/weflow-readme.md) | **Español**

> **Copia de referencia.** Traducción al español de una versión anterior del README del proyecto WeFlow original (aplicación de escritorio), aportada por webbrain-one (#1); puede estar desactualizada.
> Proyecto original: https://github.com/hicccc77/WeFlow

WeFlow es una herramienta **completamente local** para visualizar, analizar y exportar el historial de chat de WeChat en **tiempo real**. Puede obtener tus registros de chat de WeChat en tiempo real y exportarlos, así como generar informes de análisis únicos basados en tu historial.

---

<p align="center">
  <img src="../../app.jpg" alt="WeFlow 应用预览" width="90%">
</p>

<p align="center">
  <a href="https://github.com/hicccc77/WeFlow/stargazers"><img src="https://img.shields.io/github/stars/hicccc77/WeFlow?style=flat&label=Stars&labelColor=1F2937&color=2563EB" alt="Stargazers"></a>
  <a href="https://github.com/hicccc77/WeFlow/network/members"><img src="https://img.shields.io/github/forks/hicccc77/WeFlow?style=flat&label=Forks&labelColor=1F2937&color=7C3AED" alt="Forks"></a>
  <a href="https://github.com/hicccc77/WeFlow/issues"><img src="https://img.shields.io/github/issues/hicccc77/WeFlow?style=flat&label=Issues&labelColor=1F2937&color=D97706" alt="Issues"></a>
  <a href="https://github.com/hicccc77/WeFlow/releases"><img src="https://img.shields.io/github/downloads/hicccc77/WeFlow/total?style=flat&label=Downloads&labelColor=1F2937&color=059669" alt="Downloads"></a>
  <br><br>
  <a href="https://t.me/weflow_cc"><img src="https://img.shields.io/badge/Telegram-频道-1D9BF0?style=flat&logo=telegram&logoColor=white&labelColor=1F2937&color=1D9BF0" alt="Telegram Channel" style="height: 22px; vertical-align: middle;"></a>
  <a href="https://www.star-history.com/hicccc77/weflow"><img src="https://api.star-history.com/badge?repo=hicccc77/WeFlow&theme=dark" alt="Star History Rank" style="height: 32px; vertical-align: middle;"></a>
</p>

> [!TIP]
> Si deseas analizar en profundidad el contenido de tus chats después de exportarlos, prueba [ChatLab](https://chatlab.fun/)

> [!NOTE]
> Solo es compatible con WeChat **versión 4.0 y superiores**. Por favor, asegúrate de que tu versión de WeChat cumpla con los requisitos.

## Funciones Principales

- Visualización del historial de chat localmente y en tiempo real.
- Vista previa y descifrado de fotos, videos y **Live Photos** de Momentos.
- Análisis estadístico y perfiles de chats grupales.
- Informes anuales y resúmenes visuales.
- Exportación del historial de chat a formatos como HTML, entre otros.
- Interfaz HTTP API (para integración de desarrolladores).
- Ver lista completa de capacidades: [Funciones Detalladas](#detalles-de-las-funciones)

## Plataformas y Dispositivos Soportados

| Plataforma | Dispositivo/Arquitectura | Paquete |
|------------|------------------------|---------|
| Windows    | Windows 10+, x64 (amd64)| `.exe`   |
| macOS      | Apple Silicon (M series, arm64) | `.dmg` |
| Linux      | Dispositivos x64 (amd64) | `.AppImage`, `.tar.gz` |

## Inicio Rápido

Si solo deseas utilizar la versión compilada, ve a [Releases](https://github.com/hicccc77/WeFlow/releases) para descargar e instalar.

> Los usuarios de ArchLinux pueden instalar rápidamente con `yay -S weflow`

## Detalles de las Funciones

La versión actual soporta las siguientes capacidades:

| Módulo de Función | Descripción |
|-------------------|-------------|
| **Chat** | Descifra imágenes, videos y Live Photos en los chats (solo soporta Live Photos capturadas con el protocolo de Google); permite **modificar** y eliminar mensajes **locales**; actualización en tiempo real de los mensajes más recientes sin generar bases de datos intermedias descifradas |
| **Anti-Retirada** | Evita que los mensajes enviados por otros sean retirados |
| **Notificaciones en Tiempo Real** | Notificaciones emergentes de escritorio cuando llegan nuevos mensajes, facilitando la visualización oportuna de conversaciones importantes, con funcionalidad de lista blanca/negra |
| **Análisis de Chat Privado** | Estadísticas de cantidad de mensajes entre amigos; análisis de tipos de mensajes y proporciones de envío; visualización de la distribución temporal de los mensajes, etc. |
| **Análisis de Chat Grupal** | Ver información detallada de los miembros del grupo; analizar rankings de participación en el grupo, periodos de actividad y contenido multimedia |
| **Informe Anual** | Genera informes anuales estadísticos por año, o informes históricos a largo plazo que abarquen varios años |
| **Informe Dual** | Selecciona un amigo específico y genera un informe de análisis exclusivo basado en el historial de chat mutuo |
| **Exportación de Mensajes** | Exporta el historial de chat de WeChat a múltiples formatos: JSON, HTML, TXT, Excel, CSV, PGSQL, formato exclusivo de ChatLab, etc. |
| **Momentos** | Descifra fotos, videos y Live Photos de Momentos; exporta el contenido de Momentos; intercepta operaciones de eliminación y ocultación en Momentos; evade restricciones de acceso basadas en el tiempo |
| **Contactos** | Exporta información de amigos de WeChat, chats grupales y cuentas oficiales; intenta recuperar amigos eliminados (función en desarrollo) |
| **Mapeo HTTP API** | Mapea las capacidades de mensajes locales a una HTTP API para facilitar la integración con sistemas externos, scripts de automatización y desarrollo secundario |

## HTTP API

> [!WARNING]
> Esta función se encuentra actualmente en sus etapas iniciales y la interfaz puede cambiar. Mantente atento a futuras actualizaciones.

WeFlow proporciona un servicio de HTTP API local que permite consultar datos de mensajes a través de interfaces, lo cual puede ser utilizado para la integración con otras herramientas o desarrollo secundario.

- **Método de Activación**: Ajustes → Servicio API → Iniciar Servicio
- **Puerto Predeterminado**: 5031
- **Dirección de Acceso**: `http://127.0.0.1:5031`
- **Formatos Soportados**: JSON puro o formato estándar de [ChatLab](https://chatlab.fun/)

Documentación completa de la API: [Haga clic para ver](../HTTP-API.md)

## Para Desarrolladores

Si deseas construir desde el código fuente o contribuir al proyecto, sigue estos pasos:

```bash
# 1. Clonar el proyecto localmente
git clone https://github.com/hicccc77/WeFlow.git
cd WeFlow

# 2. Instalar dependencias del proyecto
npm install

# 3. Ejecutar la aplicación (modo desarrollo)
npm run dev
```

## Agradecimientos

- [CipherTalk](https://github.com/ILoveBingLu/miyu) proporcionó el marco básico para este proyecto.
- [WeChat-Channels-Video-File-Decryption](https://github.com/Evil0ctal/WeChat-Channels-Video-File-Decryption) proporcionó referencias técnicas para el descifrado de videos.

## Apóyanos

Si WeFlow realmente te ha ayudado, considera invitarnos a un café:

> TRC20 **Address:** `TZCtAw8CaeARWZBfvjidCnTcfnAtf6nvS6`

## Star History

<a href="https://www.star-history.com/#hicccc77/WeFlow&type=date&legend=top-left">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=hicccc77/WeFlow&type=date&theme=dark&legend=top-left" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=hicccc77/WeFlow&type=date&legend=top-left" />
    <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=hicccc77/WeFlow&type=date&legend=top-left" />
  </picture>
</a>

<div align="center">

---

**Por favor, utiliza esta herramienta de manera responsable y cumple con las leyes y regulaciones pertinentes.**

</div>
