"""Build web/assets/fonts/harmony/ui.woff2: the HarmonyOS Sans SC glyphs for
every Han character and CJK punctuation mark used by the web UI, cut from
the homepage's chunk files and merged into one small font.

Run it again whenever the UI text changes.

usage: python tools/font-ui.py <harmony chunk dir> <web dir> <out.woff2> [<od.css>]
  e.g. python tools/font-ui.py web/assets/fonts/harmony web \
         web/assets/fonts/harmony/ui.woff2 web/assets/od.css
needs: pip install "fonttools[woff]"

Prints the unicode-range for the @font-face rule. When od.css is given, the
rule between /* ui-font:start */ and /* ui-font:end */ is rewritten in place
(it must stay after the chunk block: the last declared face wins where the
ranges overlap, so UI text never pulls the big chunks).
"""
import io
import re
import sys
from pathlib import Path

from fontTools.merge import Merger
from fontTools.subset import Options, Subsetter
from fontTools.ttLib import TTFont


# Latin glyphs the UI deliberately draws in HarmonyOS Sans SC: the A of the
# language mark (文A), as on the homepage.
EXTRA = 'A'


def wanted_chars(web: Path) -> set[int]:
    out: set[int] = {ord(c) for c in EXTRA}
    for p in web.rglob('*'):
        if p.suffix not in {'.html', '.js'}:
            continue
        for ch in p.read_text(encoding='utf-8'):
            cp = ord(ch)
            if 0x2E80 <= cp <= 0x9FFF or 0x3000 <= cp <= 0x303F or 0xFF00 <= cp <= 0xFFEF or cp in (0x2018, 0x2019, 0x201C, 0x201D, 0x2026):
                out.add(cp)
    return out


def ranges(cps):
    cps = sorted(cps)
    parts, i = [], 0
    while i < len(cps):
        j = i
        while j + 1 < len(cps) and cps[j + 1] == cps[j] + 1:
            j += 1
        parts.append(f'U+{cps[i]:X}' if i == j else f'U+{cps[i]:X}-{cps[j]:X}')
        i = j + 1
    return ', '.join(parts)


def write_css(css: Path, urange: str):
    rule = ("@font-face { font-family: 'HarmonyOS Sans SC'; font-weight: 100 900; "
            "font-display: swap; src: url('/assets/fonts/harmony/ui.woff2') format('woff2'); "
            f"unicode-range: {urange}; }}")
    text = css.read_text(encoding='utf-8')
    pat = re.compile(r'(/\* ui-font:start[^\n]*\*/\n).*?(\n/\* ui-font:end \*/)', re.S)
    if not pat.search(text):
        sys.exit(f'{css}: no ui-font:start / ui-font:end markers')
    text = pat.sub(lambda m: m.group(1) + rule + m.group(2), text, count=1)
    css.write_bytes(text.encode('utf-8'))
    print(f'{css} updated')


def main():
    if len(sys.argv) not in (4, 5):
        sys.exit(__doc__)
    src, web, out = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3])
    need = wanted_chars(web)
    pieces, found = [], set()
    for f in sorted(src.glob('*.woff2')):
        if f.resolve() == out.resolve():
            continue
        font = TTFont(f)
        have = need & set(font.getBestCmap()) - found
        if not have:
            continue
        opts = Options()
        opts.layout_features = ['*']
        opts.name_IDs = ['*']
        opts.notdef_outline = True
        opts.drop_tables += ['DSIG']
        sub = Subsetter(opts)
        sub.populate(unicodes=have)
        sub.subset(font)
        font.flavor = None
        buf = io.BytesIO()
        font.save(buf)
        buf.seek(0)
        pieces.append(buf)
        found |= have
    missing = need - found
    if missing:
        print('missing:', ''.join(chr(c) for c in sorted(missing)), file=sys.stderr)
    if not pieces:
        sys.exit('no glyphs found')
    merged = Merger().merge(pieces) if len(pieces) > 1 else TTFont(pieces[0])
    merged.flavor = 'woff2'
    out.parent.mkdir(parents=True, exist_ok=True)
    merged.save(out)
    print(f'{out} {out.stat().st_size} bytes, {len(found)} chars')
    urange = ranges(found)
    print(urange)
    if len(sys.argv) == 5:
        write_css(Path(sys.argv[4]), urange)


if __name__ == '__main__':
    main()
