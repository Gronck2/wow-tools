#!/usr/bin/env python3
"""Builds the landing page into one folder, the way GitHub Pages serves it.

    python3 site/build.py <out dir>

site/template.html is rendered once per language from site/locales/<lang>.toml: English at /, the others at
/<lang>/. `{{key}}` puts a locale value in as HTML, `{{key:attr}}` escaped for an attribute. The static files,
docs/images/*.png (as /img) and assets/wow-tools.ico (as /favicon.ico) are copied next to the pages.
Every language must have exactly the keys English has, and every placeholder must be filled; anything else stops
the build. Standard library only (Python 3.11+ for tomllib).
"""

import html
import re
import shutil
import sys
import tomllib
from pathlib import Path

SITE = Path(__file__).resolve().parent
ROOT = SITE.parent
BASE = "https://wow-tool.gronck.com/"
# English is the default and is served at /.
LANGS = ["en", "ru", "de", "fr", "es"]
NAMES = {"en": "English", "ru": "Русский", "de": "Deutsch", "fr": "Français", "es": "Español"}
# Screenshots of "A look inside", in order, with their pixel sizes (ui-snapshot renders at 1.5x).
SHOTS = {"addons": (540, 907), "cleanup": (540, 907), "profiles": (540, 907), "settings": (840, 1290)}
STATIC = ["style.css", "site.js", "404.html", "fonts"]

PLACEHOLDER = re.compile(r"\{\{([a-z0-9_]+)(:attr)?\}\}")


def fail(msg):
    sys.exit(f"build.py: {msg}")


def page_path(lang):
    return "" if lang == "en" else f"{lang}/"


def load(lang):
    with open(SITE / "locales" / f"{lang}.toml", "rb") as f:
        return tomllib.load(f)


def check_same_shape(lang, loc, ref):
    if set(loc) != set(ref):
        missing, extra = sorted(set(ref) - set(loc)), sorted(set(loc) - set(ref))
        fail(f"{lang}.toml: missing {missing}, extra {extra}")
    for key, val in ref.items():
        if isinstance(val, list):
            if len(loc[key]) != len(val):
                fail(f"{lang}.toml: [[{key}]] has {len(loc[key])} entries, en has {len(val)}")
            for i, (a, b) in enumerate(zip(loc[key], val)):
                if set(a) != set(b):
                    fail(f"{lang}.toml: [[{key}]] #{i + 1} has keys {sorted(a)}, en has {sorted(b)}")


def blocks(lang, loc):
    """The repeated parts and the values that depend on the language, not on the text."""
    a = lambda s: html.escape(s, quote=True)
    out = {}
    out["lang"] = lang
    out["page_url"] = BASE + page_path(lang)
    out["home"] = "/" + page_path(lang)
    out["og_image"] = f"{BASE}img/panel-{lang}-dark.png"
    out["hero_img"] = f"/img/panel-{lang}-dark.png"
    out["alternates"] = "\n".join(
        [f'<link rel="alternate" hreflang="{l}" href="{BASE}{page_path(l)}">' for l in LANGS]
        + [f'<link rel="alternate" hreflang="x-default" href="{BASE}">']
    )
    out["lang_switch"] = "".join(
        f'<span aria-current="true" title="{NAMES[l]}">{l.upper()}</span>'
        if l == lang
        else f'<a href="/{page_path(l)}" hreflang="{l}" lang="{l}" title="{NAMES[l]}">{l.upper()}</a>'
        for l in LANGS
    )
    out["feature_list"] = "\n".join(
        f'<div class="feature"><span class="n">{i:02}</span><div><h3>{f["title"]}</h3><p>{f["text"]}</p></div></div>'
        for i, f in enumerate(loc["features"], 1)
    )
    if [s["view"] for s in loc["shots"]] != list(SHOTS):
        fail(f"{lang}.toml: [[shots]] views must be {list(SHOTS)} in this order")
    out["shot_list"] = "\n".join(
        f'<figure><a class="shot zoom" href="/img/{s["view"]}-{lang}-dark.png"><img src="/img/{s["view"]}-{lang}-dark.png" '
        f'width="{SHOTS[s["view"]][0]}" height="{SHOTS[s["view"]][1]}" alt="{a(s["alt"])}" loading="lazy"></a>'
        f'<figcaption>{s["caption"]}</figcaption></figure>'
        for s in loc["shots"]
    )
    out["privacy_list"] = "\n".join(
        f'<div class="notice"><span class="icon" aria-hidden="true">ℹ</span><span><strong>{p["head"]}</strong> {p["text"]}</span></div>'
        for p in loc["privacy"]
    )
    out["faq_list"] = "\n".join(f'<details><summary>{q["q"]}</summary><p>{q["a"]}</p></details>' for q in loc["faq"])
    return out


def render(template, lang, loc):
    values = {k: v for k, v in loc.items() if isinstance(v, str)}
    for k, v in blocks(lang, loc).items():
        if k in values:
            fail(f"{lang}.toml: '{k}' is computed by build.py, remove it from the locale")
        values[k] = v
    used = set()

    def sub(m):
        key, attr = m.group(1), m.group(2)
        if key not in values:
            fail(f"template.html: {{{{{key}}}}} has no value in {lang}.toml")
        used.add(key)
        return html.escape(values[key], quote=True) if attr else values[key]

    page = PLACEHOLDER.sub(sub, template)
    unused = sorted(k for k, v in loc.items() if isinstance(v, str) and k not in used)
    if unused:
        fail(f"{lang}.toml: keys not used by template.html: {unused}")
    if "{{" in page:
        fail(f"template.html: a malformed placeholder is left in the {lang} page")
    return page


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    out = Path(sys.argv[1])
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)

    template = (SITE / "template.html").read_text(encoding="utf-8")
    ref = load("en")
    for lang in LANGS:
        loc = load(lang)
        check_same_shape(lang, loc, ref)
        dest = out / page_path(lang) / "index.html"
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(render(template, lang, loc), encoding="utf-8")

    for name in STATIC:
        src = SITE / name
        if src.is_dir():
            shutil.copytree(src, out / name)
        else:
            shutil.copy2(src, out / name)
    img = out / "img"
    img.mkdir()
    for png in sorted((ROOT / "docs" / "images").glob("*.png")):
        shutil.copy2(png, img / png.name)
    for lang in LANGS:
        for view in ["panel", *SHOTS]:
            if not (img / f"{view}-{lang}-dark.png").exists():
                fail(f"docs/images/{view}-{lang}-dark.png is missing")
    shutil.copy2(ROOT / "assets" / "wow-tools.ico", out / "favicon.ico")
    print(f"build.py: {len(LANGS)} pages in {out}")


if __name__ == "__main__":
    main()
