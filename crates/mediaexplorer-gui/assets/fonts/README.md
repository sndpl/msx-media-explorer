# Bundled font

## `unifont-msx.otf`

A subset of **GNU Unifont** used as a fallback font so the GUI can render the
non-ASCII glyphs that MSX character sets map to (half-width katakana, hiragana,
accented Latin, Greek, Cyrillic, box-drawing, block and geometric symbols).
egui's built-in fonts cover only Latin, so without this the decoded glyphs would
render as tofu (missing-glyph boxes).

- **Upstream:** GNU Unifont 16.0.04, `unifont-16.0.04.otf`
  (https://unifoundry.com/unifont/).
- **License:** dual-licensed SIL Open Font License 1.1 and GNU GPLv2+ with the
  GNU Font Embedding Exception. Compatible with this project's
  `GPL-2.0-or-later`.
- **Why a subset:** the full BMP build is ~5.3 MB, dominated by CJK unified
  ideographs and Hangul that MSX single-byte charsets never use. The subset is
  ~230 KB and covers every range the `msx-disk` charset tables map to, plus
  Cyrillic and Arabic for future regions. Korean (Hangul) would need a re-subset.

### Regenerating the subset

```sh
# fonttools provides pyftsubset
pip install fonttools brotli
curl -LO https://unifoundry.com/pub/unifont/unifont-16.0.04/font-builds/unifont-16.0.04.otf
pyftsubset unifont-16.0.04.otf \
  --output-file=unifont-msx.otf \
  --unicodes="U+0000-052F,U+0590-06FF,U+2000-27FF,U+2B00-2BFF,U+3000-30FF,U+FF00-FFEF,U+1FB00-1FBFF" \
  --layout-features='*' --no-hinting --desubroutinize
```

The `U+1FB00-1FBFF` range (Symbols for Legacy Computing — a handful of MSX
International graphic cells) is absent from Unifont's BMP build, so those few
cells still render as tofu. Everything the Japanese and the common International
glyphs need is covered.
