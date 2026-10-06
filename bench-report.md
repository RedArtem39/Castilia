# Castilia MVP benchmark

This is a small capability probe, not a web standards conformance score. PASS means one narrowly defined behavior worked. FAIL is an expected MVP limitation for the cases marked unsupported.

## Local capability checks

| Case | Scope | Result |
|---|---|---|
| Basic HTML text | MVP | PASS |
| Broken paragraph recovery | MVP | PASS |
| CSS class cascade | MVP | PASS |
| Text wrapping | MVP | PASS |
| Link target | MVP | PASS |
| Image placement | MVP | PASS |
| JavaScript DOM update | MVP | PASS |
| JavaScript condition and DOM insertion | MVP | PASS |
| Basic flex row | MVP | PASS |
| Form control | Future | FAIL |
| Linear table rows | MVP | PASS |
| CSS media query | MVP | PASS |

Supported checks passed: 11/11.

## Local rendering speed

100 paragraphs, 40 warm runs, network excluded. Median: 0.13 ms; p95: 0.18 ms. These numbers depend on this machine and build mode.

## Live pages

Content checks are limited to named default sites. PASS means expected words appear in the drawing list, not that the layout or interactions match a browser. CSS counts are approximate.

| URL | Fetch | Expected content | HTML | CSS loaded | DOM | Draw commands | JS run/errors | External JS loaded | JS skipped | Scripts | Forms | Tables | CSS ignored (selectors / declarations / at-rules) | Elapsed |
|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| https://httpbin.org/html | OK | PASS | 3741 | 0/0 | 11 | 605 | 0/0 | 0 | 0 | 0 | 0 | 0 | 0 / 0 / 0 | 525 ms |
| https://en.wikipedia.org/wiki/World_Wide_Web | OK | PASS | 763411 | 2/2 | 12136 | 12607 | 0/3 | 0 | 2 | 5 | 2 | 10 | 1055 / 2320 / 114 | 923 ms |

- https://en.wikipedia.org/wiki/World_Wide_Web: script node 7: unsupported JavaScript token: ?

- https://en.wikipedia.org/wiki/World_Wide_Web: script node 9: unsupported JavaScript token: [

- https://en.wikipedia.org/wiki/World_Wide_Web: script node 12132: unsupported JavaScript token: [
| https://rust-lang.org/ | OK | PASS | 18594 | 4/4 | 580 | 531 | 1/3 | 3 | 0 | 4 | 0 | 0 | 320 / 943 / 56 | 525 ms |

- https://rust-lang.org/: script node 46: unsupported JavaScript token: [

- https://rust-lang.org/: script node 48: unknown identifier: hljs

- https://rust-lang.org/: script node 578: unsupported JavaScript token: `
| https://www.w3.org/ | OK | PASS | 48654 | 3/3 | 868 | 430 | 0/4 | 1 | 0 | 4 | 0 | 0 | 625 / 1617 / 101 | 247 ms |

- https://www.w3.org/: script node 14: unsupported JavaScript token: [

- https://www.w3.org/: script node 16: unsupported JavaScript token: [

- https://www.w3.org/: script node 34: unsupported JavaScript token: [

- https://www.w3.org/: script node 866: unsupported writable property: src
| https://www.python.org/ | OK | PASS | 52280 | 4/4 | 1930 | 967 | 0/9 | 7 | 2 | 11 | 1 | 1 | 1666 / 2535 / 25 | 964 ms |

- https://www.python.org/: script node 4: unsupported JavaScript token: :

- https://www.python.org/: script node 18: unsupported JavaScript token: ~

- https://www.python.org/: script node 1916: unsupported JavaScript token: [

- https://www.python.org/: script node 1918: unsupported DOM method: write

- https://www.python.org/: script node 1920: unsupported JavaScript token: ?

- https://www.python.org/: script node 1922: unsupported DOM method: write

- https://www.python.org/: script node 1924: unsupported JavaScript token: [

- https://www.python.org/: script node 1926: expected ','

- https://www.python.org/: 1 more warnings
| https://news.ycombinator.com/ | OK | PASS | 34173 | 1/1 | 1296 | 724 | 0/1 | 1 | 0 | 1 | 1 | 4 | 37 / 60 / 12 | 1110 ms |

- https://news.ycombinator.com/: script node 1294: unsupported JavaScript token: ?
