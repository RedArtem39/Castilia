# Castilia

Небольшое переносимое ядро браузера на Rust: `#![no_std]` + `alloc`, **ноль обязательных зависимостей**.
Хост (Linux, кастомная ОС) даёт только метрики шрифтов, сеть и отрисовку; движок разбирает HTML, применяет CSS,
выполняет JavaScript, строит раскладку и отдаёт платформенно-независимый `DisplayList`.

## Что внутри

- **HTML5**: токенизатор и построение дерева (таблицы, формы, foreign content, `<template>`), кодировки
  (BOM, `Content-Type`, `<meta charset>`, windows-1251/1252, KOI8-R, ISO-8859-5/15, UTF-16).
- **CSS**: каскад со слоями, `@media`/`@supports`/`@container`, вложенность, `var()`/`calc()`/`clamp()`,
  цвета `lab/oklab/oklch/color-mix/light-dark`, flex, grid, таблицы, float, позиционирование, `transform`,
  `object-fit`, `line-clamp`, `text-shadow`, `outline`, градиенты, тени. `@import` и `url()` обрабатываются в `cssload`.
- **JavaScript** (`src/js`): собственный движок ES2022 — парсер, интерпретатор, regexp-VM, Promise/async,
  генераторы (энергичные), Proxy, ES-модули и import maps, `with`, Intl-минимум.
- **DOM/BOM**: деревья узлов, события (capture/bubble), `MutationObserver`, `classList/dataset/style`,
  формы и ввод (фокус, набор текста, клавиши, чекбоксы, отправка), `fetch`/`XMLHttpRequest` через трейт `Host`,
  `localStorage`, `URL`, `history`, `matchMedia`, `getComputedStyle`, custom elements (базово).
- **Сеть** (`network`): трейт `Fetcher` — хост реализует GET/POST; движок сам разрешает URL, подтягивает CSS,
  скрипты и размеры изображений (с повторной раскладкой).

## Переносимость

Ядро не использует `std`. Хост реализует: `FontMetrics` (измерение текста, размеры картинок),
`network::Fetcher` и/или `js::Host` (сеть, alert, навигация). Всё остальное — чистая функция документа.
Фича `host-net` (ureq/url/base64) нужна только примерам для Linux.

## Проверка

- `cargo test --lib` — модульные тесты (в т.ч. JS, DOM, сеть).
- `python3 tools/visual_bench.py` — сравнение с Chromium на `visual_cases/` (0–2% пикселей, в основном субпиксельный текст).
- `python3 tools/build_corpus.py` + `visual_bench.py --dir corpus` — реальные страницы Bootstrap/Bulma/Tailwind/Pure/GitHub-markdown (высоты совпадают, 0–4% пикселей).
- `examples/jsrun.rs` — запуск скриптов; проверено на jQuery, lodash, React 18, Vue 3, Preact (ESM), Alpine, htmx, Bootstrap JS, moment, dayjs, handlebars, marked, axios, Backbone, Mithril.

## Честно о покрытии

Процент «всего веба» измерить нельзя (в песочнице нет реальных сайтов); прогресс меряется локальным корпусом против Chromium.
Известные пробелы: генераторы без двусторонней передачи значений (`next(v)`), нет BigInt/TypedArray на буферах,
нет bidi/RTL, `writing-mode`, `clip-path`, фильтров, iframe, canvas-рисования, Web Workers/IndexedDB/WebSocket,
интерпретатор примерно в 50–100 раз медленнее V8.
