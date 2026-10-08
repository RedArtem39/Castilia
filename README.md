# Castilia

Переносимое ядро браузера на Rust: `#![no_std]` + `alloc`, **ноль обязательных зависимостей**.
Хост даёт метрики шрифтов, сеть и отрисовку; движок разбирает HTML, применяет CSS, выполняет
JavaScript, строит раскладку и отдаёт платформенно-независимый `DisplayList`.

## Режимы исполнения JS

`EngineConfig::exec_mode` (`src/config.rs`):

| Режим | Что делает | Требования |
|---|---|---|
| `ExecMode::Interpreter` | байткод-VM | ничего — работает на любом CPU и ОС, в т.ч. с запретом исполняемой памяти (iOS, консоли, W^X, wasm) |
| `ExecMode::Jit` | VM + базовый JIT для горячих функций (x86-64) | фича `jit` (по умолчанию) и `platform::CodeMemory` от хоста; иначе тихо работает как интерпретатор |
| `ExecMode::AstWalker` | эталонный обход AST | только для отладки и сравнения |

Сборка `--no-default-features` — чистый интерпретатор без JIT-кода.

- **Байткод-VM** (`src/js/vm`): регистры для локальных переменных, настоящие приостанавливаемые генераторы
  и `async`/`await` с правильным порядком микрозадач, вызовы JS→JS без рекурсии Rust-стека, пул фреймов.
- **JIT** (`src/js/jit`): в духе V8 Sparkplug — числа, сравнения, переходы и регистры в машинном коде,
  всё остальное через обработчики интерпретатора (фрейм общий, выход в интерпретатор бесплатный).
- **Сборщик циклов** (`src/js/gc.rs`) поверх `Rc`: пробное удаление как в CPython, консервативные корни.
- **Сборка DOM** (`src/js/domgc.rs`): недостижимые отсоединённые узлы собираются, их id переиспользуются
  (страницы, перестраивающие `innerHTML` в цикле, больше не растут без предела).

## Порт на новую платформу

Всё в `src/platform.rs` и `src/config.rs`:

| Сервис | Трейт | Обязателен |
|---|---|---|
| Измерение текста, размеры картинок, веб-шрифты (`add_font_face`) | `layout::FontMetrics` | да |
| HTTP(S), разрешение URL | `network::Fetcher` | для сетевых страниц |
| `fetch`/XHR, диалоги, навигация | `js::Host` | для интерактивных страниц |
| Исполняемая память (W^X) | `platform::CodeMemory` | только для JIT |
| Остановка зависших скриптов | `platform::Interrupt` | нет (время сети не засчитывается) |

Профили `EngineConfig::desktop()`, `interpreter()`, `small_device()`; лимиты: бюджет задачи, глубина
нативной рекурсии, число фреймов, объектов на задачу, длина строки, порог GC, размеры скриптов.
Время виртуальное — запуск детерминирован.

## Что поддерживается

- **HTML5**: дерево, таблицы, формы, foreign content, `<template>`, ложные комментарии, кодировки.
- **Шрифты**: `@font-face` (WOFF/WOFF2/TTF, `unicode-range`, вариативные оси `wght`/`opsz`), подбор по
  правилам CSS, загружаются только начертания, которыми реально набран текст; хост-пример меряет текст
  через `ttf-parser` (без разбора контуров) и ищет системные шрифты по имени.
- **CSS**: каскад со слоями, `text-wrap: balance`, `@media` (Level 4: диапазоны, `or`, `not`), `@supports`, `@container`,
  вложенность, `var()`/`calc()`/`min()`/`clamp()`, flex, grid (в т.ч. `minmax(min(...))`), таблицы,
  float, позиционирование (вложенные `fixed`), трансформации, `::placeholder`, Shadow DOM со стилями
  и `adoptedStyleSheets`.
- **JavaScript** ES2023: классы с приватными полями, генераторы, async-генераторы, `BigInt`,
  Proxy/Reflect, ES-модули и import maps, `eval`, регулярные выражения с `\p{...}`, Intl (базово).
- **Web API**: DOM, события, MutationObserver, custom elements (апгрейд в теневых деревьях,
  `whenDefined`), живые TreeWalker/NodeIterator, `ArrayBuffer`/TypedArray/`DataView`, Streams,
  TextEncoder/Decoder, fetch/XHR, storage, IndexedDB (в памяти), CSSOM (`sheet`, `styleSheets`,
  `insertRule`), `getComputedStyle` по реальному каскаду, отдельные документы `DOMParser`,
  Trusted Types, iframe-контекст (упрощённо), Performance.

## Проверка

- `cargo test --lib` — модульные тесты; JS-тесты гоняются в AST-, байткод- и нативном режимах.
- `python3 tools/libcheck.py LIBDIR --mode interp --mode jit` — 21 реальная библиотека
  (React, Vue, Preact, jQuery, Lodash, d3, three.js, mobx, redux, rxjs, zod, katex, formatjs, ...).
- `cargo run --release --features host-net --example sitecheck` — реальные сайты из `tools/sites.txt`,
  каждый в своём процессе с лимитом памяти; `tools/shot.py URL NAME` — скриншот рядом с Chromium.
- `python3 tools/webcheck.py tools/crux_sample.txt` — 500 популярных сайтов (выборка CrUX) против
  Chromium: доля сайтов, где видимого текста столько же, сколько у Chromium (антибот-стены исключаются);
  `tools/textdiff.py URL` — какой текст рисуем мы и не показывает Chromium (и наоборот).
- `browse` поддерживает `CASTILIA_TRACE`, `CASTILIA_CONSOLE`, `CASTILIA_EVAL`, `CASTILIA_HEAP`,
  `CASTILIA_NO_JS`, `CASTILIA_NETLOG` (журнал запросов), `CASTILIA_BLOCK` (блокировать URL),
  `CASTILIA_OOM_TRACE`/`CASTILIA_ALLOC_SAMPLE` (где кончилась/расходуется память), `CASTILIA_SEH`
  (кадры при переполнении стека, Windows).

## Честно о пробелах

Объекты «толстые» (≈70 байт на свойство, нет скрытых классов), AST функций хранится вместе с байткодом —
тяжёлые SPA занимают сотни мегабайт; конкатенация строк в цикле квадратичная (нет ropes);
интерпретатор ~20–30 нс на простую итерацию, вызов функции ~200 нс; JIT только для x86-64; нет
WebAssembly, Workers, canvas-рисования, `CompressionStream`; часть SPA (страница репозитория GitHub,
Twitch, Pinterest, Instagram) пока не дорисовывается; часть сайтов отказывает нашему HTTP-клиенту
по отпечатку TLS (это забота хоста, не движка).

## Лицензия

Castilia распространяется на условиях **GNU GPL версии 2 или GNU GPL версии 3** — на выбор
получателя (SPDX: `GPL-2.0-only OR GPL-3.0-only`). Тексты лицензий: [`LICENSE-GPL-2.0`](LICENSE-GPL-2.0),
[`LICENSE-GPL-3.0`](LICENSE-GPL-3.0).
