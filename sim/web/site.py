#!/usr/bin/env python3
"""Turns the README, as GitHub renders it, into the site's landing page.

    python3 web/site.py README.html TEMPLATE.html REPO COMMIT OUT

README.html is README.md through GitHub's markdown API (site.sh fetches
it), so the page reads as the README does on GitHub, and can't drift from
it: this only moves it into the site's template (web/site.html) and fixes
what doesn't work outside github.com. It writes OUT/index.html, and copies
the images the README shows into OUT, at the same paths as in the
repository (REPO), so their relative URLs keep working. Exits non-zero if
anything doesn't fit, rather than publish a page with holes in it.

Only the standard library: the workflow runs it on a stock runner.
"""

import html
import re
import shutil
import string
import subprocess
import sys
from pathlib import Path

GITHUB = "https://github.com/jonhoo/ur"


def fail(message):
    sys.exit(f"site.py: {message}")


if len(sys.argv) != 6:
    fail("usage: site.py README.html TEMPLATE.html REPO COMMIT OUT")
readme_path, template_path, repo, commit, out = sys.argv[1:]
readme = Path(readme_path).read_text(encoding="utf-8")
template = Path(template_path).read_text(encoding="utf-8")
repo = Path(repo).resolve()
out = Path(out)

# ==============================================================================
# Anchors
# ==============================================================================

# GitHub prefixes every id it generates with `user-content-` (so a README
# can't clash with the page around it), and its own script maps `#faq` to
# `#user-content-faq` on click. There's no such script here, so the ids go
# back to what the README's links say.
readme = readme.replace(' id="user-content-', ' id="')

# ==============================================================================
# Images
# ==============================================================================

# GitHub wraps every image in a link to the image itself, to open it full
# size. The screenshots are 240×240, so that would only open a small image
# on a blank page: unwrap them.
readme = re.sub(
    r'<a target="_blank" rel="noopener noreferrer" href="([^"]*)">(<img src="\1"[^>]*>)</a>',
    r"\2",
    readme,
)

# The images with relative paths are the repository's, and go with the
# site, at the same paths. Absolute ones (none, yet) are left alone: GitHub
# would have proxied them, but a browser can fetch them directly.
images = []
for source in re.findall(r'<img src="([^"]*)"', readme):
    if re.match(r"[a-z]+:|/", source):
        continue
    path = (repo / html.unescape(source)).resolve()
    if not path.is_relative_to(repo) or not path.is_file():
        fail(f"the README shows {source}, which isn't a file in the repository")
    images.append(path)


# Images are in Git LFS. A checkout without it (actions/checkout's default)
# has pointer files in their place: fetch just the ones the page shows,
# rather than every image the repository has (the simulator's approved
# screenshots are most of them), as each download counts against the
# repository's LFS bandwidth. Where they're already here, nothing is
# fetched.
def is_lfs_pointer(path):
    with path.open("rb") as file:
        return file.read(len(b"version https://git-lfs")) == b"version https://git-lfs"


pointers = [str(path.relative_to(repo)) for path in images if is_lfs_pointer(path)]
if pointers:
    subprocess.run(["git", "lfs", "pull", f"--include={','.join(pointers)}"], cwd=repo, check=True)
    still = [path for path in pointers if is_lfs_pointer(repo / path)]
    if still:
        fail(f"git lfs pull left these as LFS pointers: {', '.join(still)}")

for path in images:
    target = out / path.relative_to(repo)
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(path, target)

# ==============================================================================
# Links to the rest of the repository
# ==============================================================================

# Everything else a relative link points at (CONTRIBUTING.md, docs/, the
# licences) isn't on the site: send it to the file on GitHub, which renders
# it. `blob/` works for directories too; GitHub redirects to `tree/`.
readme = re.sub(
    r'href="(?![a-z]+:|#|/)([^"]*)"',
    lambda link: f'href="{GITHUB}/blob/main/{link[1]}"',
    readme,
)

# ==============================================================================
# The hero: the README's title and opening paragraphs
# ==============================================================================

# The README opens with its title, then the plain paragraphs that say what
# ur is, then the screenshots (paragraphs with attributes). The title and
# those paragraphs go at the top of the page, above the "Try it" button;
# the rest of the README follows, as it is.
opening = re.match(
    r'\s*<div class="markdown-heading"><h1 class="heading-element">(.*?)</h1>.*?</div>'
    r"((?:\s*<p>.*?</p>)+)",
    readme,
    re.DOTALL,
)
if not opening:
    fail("the README doesn't open with a title and a paragraph")
title, hero = opening[1], opening[2].strip()
body = readme[opening.end() :].strip()


def text(fragment):
    """The plain text of an HTML fragment, for <title> and the description."""
    return " ".join(html.unescape(re.sub(r"<[^>]*>", "", fragment)).split())


tagline = text(re.match(r"<p>(.*?)</p>", hero, re.DOTALL)[1])

page = string.Template(template).substitute(
    title=html.escape(f"{text(title)}: {tagline}"),
    name=html.escape(text(title)),
    description=html.escape(tagline),
    hero=hero,
    body=body,
    commit=commit,
    short=commit[:7],
)

# ==============================================================================
# Checks
# ==============================================================================

# Every link within the page lands somewhere: the template's navigation
# names README sections, and the README links to its own (the FAQ entries
# the watch shows codes for), so a renamed heading would otherwise leave a
# link that silently goes nowhere.
ids = set(re.findall(r'\sid="([^"]*)"', page))
missing = sorted({target for target in re.findall(r'href="#([^"]*)"', page) if target and target not in ids})
if missing:
    fail(f"links to sections that aren't on the page: {', '.join('#' + m for m in missing)}")

(out / "index.html").write_text(page, encoding="utf-8")
print(f"site.py: {out / 'index.html'}, with {len(images)} images")
