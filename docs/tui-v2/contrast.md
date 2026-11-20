# Next contrast record

For [#409](https://github.com/OpenCoven/coven-code/issues/409). These ratios are
WCAG contrast measured from the colors Next draws, against the Brand canvas
(`#050409` dark, `#FCFAFE` light) or the role's own fill. A test compares this
table with the code, so it cannot drift:

```text
UPDATE_GOLDENS=1 cargo test -p claurst-tui --test next_primitives
```

The tests require 7:1 for `Text` in truecolor, 4.5:1 for other text, and 3:1 for
`Identity`, which marks rather than carries body text. `Border` is not text and
is not listed.

<!-- measured: begin -->
| Role | Read against | Truecolor dark | Truecolor light | 256 dark | 256 light |
|---|---|---:|---:|---:|---:|
| Text | the canvas | 18.64 | 17.48 | 20.03 | 17.04 |
| TextMuted | the canvas | 11.61 | 7.08 | 11.72 | 6.39 |
| TextSubtle | the canvas | 7.63 | 5.13 | 6.59 | 4.54 |
| Identity | the canvas | 4.15 | 6.15 | 4.87 | 6.04 |
| Action | its own fill | 4.93 | 6.38 | 4.52 | 6.04 |
| Focus | the canvas | 10.17 | 7.19 | 10.93 | 8.51 |
| Presence | the canvas | 8.33 | 9.31 | 7.37 | 11.44 |
| Selection | its own fill | 15.36 | 14.54 | 15.13 | 14.69 |
| Link | the canvas | 10.17 | 7.19 | 10.93 | 8.51 |
| Verified | its own fill | 10.59 | 6.11 | 10.04 | 6.46 |
| Pending | its own fill | 11.12 | 7.10 | 12.43 | 6.58 |
| Blocked | its own fill | 9.86 | 8.07 | 9.74 | 8.92 |
| Evidence | its own fill | 13.23 | 8.58 | 13.40 | 8.39 |
| DiffAdded | the canvas | 13.83 | 6.58 | 11.80 | 7.49 |
| DiffRemoved | the canvas | 12.04 | 8.94 | 11.45 | 10.34 |
| Sensitive | the canvas | 12.04 | 8.94 | 11.45 | 10.34 |
<!-- measured: end -->

## What this does not certify

- **Your terminal's background.** Next never paints the canvas, so these ratios
  hold only on a background close to the Brand canvas. With `theme` set to `auto`
  and no `COLORFGBG`, Next does not know the background and uses the terminal's
  own palette. An explicit `theme` that does not match the terminal is not
  corrected.
- **16-color and terminal-native output.** The terminal's theme decides those
  colors, so no ratio is claimed. Status and selection keep a text or shape cue
  and a text attribute, which the tests check.
- **No color.** Nothing is colored, and the same cues carry the meaning.
- **Real emulators.** These are calculated values. Nobody has judged them by eye
  in a terminal, and native Windows has not been checked.
