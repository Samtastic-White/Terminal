> [🇬🇧 English](./README.md) · 🇪🇸 **Español**

# ⚡ Terminal Nativa de Bloques (Rust + GPUI)

> **Traducción al español.** Este archivo traduce [`README.md`](./README.md); si hay discrepancia, prevalece la versión inglesa.

> **Reingeniería y migración nativa de la arquitectura de Wave Terminal hacia Rust y GPUI, con la experiencia visual y ergonomía de Warp.**
> Un emulador de terminal por bloques de alto rendimiento para Linux, renderizado directamente sobre GPU (Vulkan/Wayland) y diseñado bajo un consumo estricto de **menos de 80 MB de memoria RAM**, prescindiendo por completo de Electron, Node.js y motores de navegador web (cero WebKitGTK, cero Chromium).

---

## 1. La Tesis del Proyecto

El mundo de los emuladores de terminal en Linux está roto en dos extremos polarizados:

| Terminal | Modelo Visual | Editor de Entrada | Consumo de RAM | Arquitectura |
| :--- | :--- | :--- | :--- | :--- |
| **Ghostty / Kitty / Alacritty** | Grilla continua (VT100) | ZLE plano (ANSI) | 35 – 60 MB | Nativa (Zig / C) |
| **Warp Terminal** | Tarjetas / Bloques | Editor GUI enriquecido | 550 – 800 MB | Rust + Cloud / Telemetría |
| **Wave Terminal** | Bloques / Lienzo | Monaco (VS Code) | 600 – 1000 MB | Electron (Chromium) + Go |
| **Este proyecto** | **Tarjetas / Bloques** | **Editor GUI (GPUI)** | **50 – 80 MB** | **Rust puro + GPUI nativo** |

- **El modelo clásico de grilla continua** es insuperable en rendimiento, pero arrastra las limitaciones conceptuales del teletipo de hace 40 años: texto como búfer plano sin estructurar, ratón de segunda clase y un editor de entrada (ZLE) donde una selección multilínea o mover el cursor con un clic es tosco o imposible.
- **Las terminales modernas de bloques** transforman la consola en una interfaz real — cada comando es una tarjeta independiente con estado, metadatos, sticky headers y editor gráfico multilínea — pero a un precio brutal: Wave carga **Electron + Chromium + React 19 + Monaco + demonio en Go** (600 MB – 1 GB) y Warp acopla un cliente Rust a infraestructura en nube propietaria y telemetría (500 – 800 MB).

**Propuesta de valor:** la arquitectura por bloques y el espacio de trabajo de Wave, la ergonomía visual de Warp, y la ética de recursos, velocidad (120 FPS) y consumo < 80 MB de Ghostty o Zed — todo en Rust nativo acelerado por GPU.

---

## 2. La Estrategia Central: Migración Arquitectónica de Wave a GPUI

Este proyecto no es una terminal construida al azar: **es un port conceptual directo de la arquitectura de Wave Terminal**, reemplazando su motor de ejecución web por tecnologías nativas de sistemas. Wave resolvió de manera brillante cómo modelar un bloque de terminal, su controlador y su ciclo de vida; el trabajo aquí es tomar esa arquitectura probada y reconstruirla en Rust.

```text
┌──────────────────────────────────────────────────────────────────────────┐
│                        ARQUITECTURA ORIGINAL DE WAVE                     │
│   Frontend: Electron + Chromium + React 19 + Monaco (~700 MB RAM)        │
│   Backend:  Demonio local en Go (wavesrv) + WebSockets + PTY             │
└────────────────────────────────────┬─────────────────────────────────────┘
                                     ▼  (MIGRACIÓN ARQUITECTÓNICA)
┌──────────────────────────────────────────────────────────────────────────┐
│                        NUESTRA ARQUITECTURA NATIVA                       │
│   Frontend: GPUI (Zed Engine) ── renderizado directo en Vulkan (~70 MB)  │
│   Backend:  Módulos Rust en memoria (portable-pty) ── canales mpsc       │
└──────────────────────────────────────────────────────────────────────────┘
```

### Tabla de Equivalencias de la Migración

| Componente | Implementación original en Wave | Reimplementación en Rust / GPUI |
| :--- | :--- | :--- |
| Shell / Renderer | Electron + Chromium | GPUI nativo (por hardware, sin navegador) |
| Editor de Comandos | Monaco Editor (VS Code web) | Input Editor nativo en GPUI (spans de texto) |
| Modelo del Bloque | `BlockController` (TS / Jotai) | `Model<CommandBlock>` reactivo (`cx.notify()`) |
| Backend de Terminal | Servidor en Go (`wavesrv`) vía sockets | Adaptador PTY interno con `portable-pty` (RAM directa) |
| Parser ANSI / Control | `xterm.js` | Parser léxico con estados + secuencias semánticas |
| Workspace / Splits | Árbol de layouts en React | Layouts Flexbox acelerados por GPU en GPUI |

---

## 3. La Experiencia: La Elegancia de Warp

La ingeniería interna proviene de la migración de Wave; la interfaz, la estética y la interacción replican la elegancia de Warp:

- **Aislamiento físico por tarjetas** — la pantalla no es un flujo de texto continuo: cada comando crea una tarjeta visual (`CommandBlock`) con contenedor, bordes redondeados y márgenes propios.
- **El «No Scroll Infinito» (Sticky Command Header)** — en salidas de miles de líneas (logs, compilaciones), el encabezado del bloque se congela en el borde superior para no perder el contexto.
- **Copia selectiva en un clic** — botones por bloque para copiar: solo la orden escrita, solo la salida (stdout/stderr, sin prompts), o el bloque completo.
- **Input GUI vs. ZLE** — la barra de entrada es un editor de verdad: cursor con clic, selección por arrastre, atajos de sistema (`Ctrl+C/V/A`) y salto de línea limpio con `Shift+Enter`.
- **Modo Alt-Screen inmersivo** — detección de herramientas interactivas (`vim`, `htop`, `less`): el sistema suspende las tarjetas y cede el 100% del viewport al proceso para evitar redibujados rotos.
- **Tools Panel** — panel lateral con árbol de archivos del cwd actualizado en tiempo real (vía evento semántico del shell), previsualización y apertura de archivos sin teclear `ls`.

---

## 4. Integración Semántica con el Shell (Zsh + OSC 133)

Para que el motor sepa cuándo empieza un comando, cuándo termina y qué código produjo — sin heurísticas ni expresiones regulares frágiles — se inyectan ganchos semánticos estándar OSC 133 en `/bin/zsh`:

| Marcador | Significado |
| :--- | :--- |
| `\e]133;A\e\` | Inicio de prompt: el shell espera comandos |
| `\e]133;B\e\` | Fin de prompt: el usuario empieza a escribir |
| `\e]133;C\e\` | Comando ejecutado: arranca el proceso, se instancia el bloque activo |
| `\e]133;D;<code>\e\` | Comando finalizado: captura del código de salida y duración |
| `\e]133;P;Cwd=<ruta>\e\` | Sincronización de directorio para el panel lateral |

---

## 5. Principios y Restricciones No Negociables

- **Presupuesto estricto de memoria (< 80 MB RAM):** el ejecutable en reposo opera entre 30 y 50 MB; techo absoluto de 80 MB. Los bloques fuera del viewport se virtualizan.
- **Cero dependencias web:** prohibido Electron, WebKitGTK, Tauri o Chromium embebido.
- **Linux first & Zsh first:** desarrollo y soporte inicial exclusivamente en distribuciones Linux modernas sobre Wayland/X11 con shell Zsh.
- **Arquitectura limpia inviolable:** el dominio no conoce GPUI, sockets ni SO; todo se conecta vía Puertos y Adaptadores.

---

## 6. Pila Tecnológica

| Rol | Crate / Tecnología |
| :--- | :--- |
| Lenguaje | Rust (Edición 2021+) |
| Interfaz y GPU | `gpui` (Zed) — Vulkan / OpenGL en Linux |
| PTY y procesos hijos | `portable-pty` |
| Parser ANSI/VT100 | `vte` |
| Monitoreo de archivos | `notify` |
| Historial local (Fase 5+) | `rusqlite` |
| Gestión asíncrona | `smol` o el ejecutor nativo de GPUI |

---

## 7. Hoja de Ruta (Resumen)

| Fase | Entregable clave |
| :--- | :--- |
| 0 | Workspace Cargo + ventana GPUI básica en Wayland (pacing de compositor sostenido a 143.8 Hz; suelo PSS en reposo ≈ 70 MB atribuido, techo < 80 MB verificado en Fase 6; verificación X11 diferida) |
| 1 | Subsistema PTY (`portable-pty` + lector en background + `vte`) |
| 2 | Protocolo semántico OSC 133 + primer bloque visual funcional |
| 3 | Editor de entrada GUI + sticky header |
| 4 | Modo inmersivo (alternate screen) |
| 5 | Tools Panel (explorador lateral de archivos) |
| 6 | Auditoría de rendimiento y memoria (heaptrack, virtualización, < 80 MB) |

---

## 8. Entorno de Desarrollo (Docker Sandbox)

Para garantizar paridad total con los controladores de GPU (Vulkan) y Wayland sin ensuciar la instalación del anfitrión Fedora, el entorno completo está contenerizado (`compose.yml`, `dev.Dockerfile`, `mise.toml`):

```bash
mise run up      # iniciar el contenedor en segundo plano (docker compose up -d)
mise run shell   # terminal interactiva dentro del contenedor (zsh)
mise run run     # compilar y ejecutar la ventana nativa (cargo run)
mise run clean   # destruir el entorno y liberar espacio
```

Verificación estándar dentro del contenedor: `cargo check && cargo clippy -- -D warnings && cargo test`.
