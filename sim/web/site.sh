#!/usr/bin/env bash
# Builds the GitHub Pages site into DIR (default: sim/target/site): a
# landing page made from README.md at the top, and the simulator
# (build.sh) under sim/. The Pages workflow runs this, so a local build is
# the site as it would be published. Serve it as build.sh's, e.g.
#
#     python3 -m http.server -d target/site
#
# Needs build.sh's needs, python3, and the GitHub CLI (`gh`), logged in or
# with GH_TOKEN set: the README is rendered by GitHub's own markdown API,
# so the page reads exactly as the README does on github.com, without a
# converter of our own to pin and keep in step with GitHub's flavour.
set -euo pipefail

sim=$(cd "$(dirname "$0")/.." && pwd)
repo=$(cd "$sim/.." && pwd)
out=${1:-$sim/target/site}

# From scratch: whatever an earlier build left (an image the README no
# longer shows) would otherwise be published too.
rm -rf "$out"
mkdir -p "$out"
out=$(cd "$out" && pwd)
cd "$sim"

# The simulator, under sim/: the landing page links to it there, and its
# own URLs are all relative, so it works at any path.
web/build.sh "$out/sim"

# `markdown` mode renders as GitHub renders a file in a repository; `gfm`
# would be as a comment, where every line break is a <br>. The rendering
# goes beside the site, not in it, so it isn't published.
rendered="$sim/target/site-readme.html"
gh api --method POST /markdown -f mode=markdown -F text=@"$repo/README.md" >"$rendered"

# Into the template, with the README's images copied alongside (see
# site.py). The commit is the one the page was made from, for its footer.
commit=$(git -C "$repo" rev-parse HEAD)
python3 web/site.py "$rendered" web/site.html "$repo" "$commit" "$out"
echo "built $out"
