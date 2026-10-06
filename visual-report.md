# Visual benchmark

Reference: Chromium at 800 x 300 CSS pixels. Each case is rendered from the same HTML by Chromium and Castilia. A pixel is counted as different when any RGB channel differs by more than 5.

| Case | Different pixels | Image area |
|---|---:|---:|
| boxes | 0 | 0.00% |
| flex | 0 | 0.00% |
| hidden | 0 | 0.00% |
| image | 0 | 0.00% |
| margins | 0 | 0.00% |
| media | 0 | 0.00% |
| percent | 0 | 0.00% |
| text | 1192 | 0.50% |

Generated images are in `visual_out/`. This is a focused visual regression suite, not a full browser conformance score.
