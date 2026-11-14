#!/usr/bin/env bash
# The install the README gives, against a local page that lists a release's files:
#
#   tools/check_release_install.sh DIST_DIR
#
# serves DIST_DIR with `python -m http.server` (a plain listing of links, as GitHub's
# releases/expanded_assets/<tag> page is), then in fresh virtual environments runs
#   pip install --find-links http://127.0.0.1:<port>/ <package>==<version>
# for each agent (and the extras that pull in the renderers and the font bundle), and checks that every package of this repository it pulled in came from
# the page (the version the checkout names), with third-party dependencies from PyPI.
# The server and the environments are removed at the end, also on failure.
set -euo pipefail
dist=$(cd "$1" && pwd)
root=$(cd "$(dirname "$0")/.." && pwd)
python=${PYTHON:-python3}
work=$(mktemp -d)
port=$("$python" -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
"$python" -m http.server "$port" --bind 127.0.0.1 --directory "$dist" > "$work/server.log" 2>&1 &
server=$!
trap 'kill $server 2>/dev/null || true; wait $server 2>/dev/null || true; rm -rf "$work"' EXIT
for _ in $(seq 50); do curl -sf "http://127.0.0.1:$port/" > /dev/null && break; sleep 0.2; done

version_of() {  # name -> version from the checkout's pyproject.toml
  "$python" "$root/tools/release_notes.py" check-only | awk -F'|' -v n="$1" '{gsub(/ /,"",$2); gsub(/ /,"",$3)} $2==n {print $3}'
}

# Each agent, and the extras that pull in the renderers and the font bundle.
items="docx-agent:png pptx-agent:png xlsx-agent: pptx2svg:fonts"
# ooxml-core, when the page has its files (the Rust build's)
if ls "$dist"/ooxml_core-* > /dev/null 2>&1; then items="$items ooxml-core:"; fi
for item in $items; do
  agent=${item%%:*}; extra=${item#*:}
  v=$(version_of "$agent")
  req="$agent${extra:+[$extra]}==$v"
  echo "== pip install --find-links http://127.0.0.1:$port/ $req"
  "$python" -m venv "$work/$agent"
  "$work/$agent/bin/python" -m pip install -q --upgrade pip
  "$work/$agent/bin/python" -m pip install -q --find-links "http://127.0.0.1:$port/" "$req"
  # Every distribution of this repository that got installed is the one the page has.
  "$work/$agent/bin/python" - "$root" <<'PY'
import importlib.metadata as md, subprocess, sys
root = sys.argv[1]
table = subprocess.run([sys.executable, f"{root}/tools/release_notes.py", "check-only"],
                       capture_output=True, text=True, check=True).stdout
ours = {}
for line in table.splitlines():
    cells = [c.strip() for c in line.strip("|").split("|")]
    if len(cells) == 2 and cells[1][:1].isdigit():
        ours[cells[0]] = cells[1]
found = {d.metadata["Name"]: d.version for d in md.distributions() if d.metadata["Name"] in ours}
print("  from the page:", ", ".join(f"{k} {v}" for k, v in sorted(found.items())))
wrong = {k: v for k, v in found.items() if v != ours[k]}
sys.exit(f"not the release's versions: {wrong}" if wrong else 0)
PY
  mod=${agent//-/_}
  "$work/$agent/bin/python" -c "import $mod; print('  import $mod ok')"
done
