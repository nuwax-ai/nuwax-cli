#!/usr/bin/env bash
# Isolated, ordinary-user permission regression. The argument is a native Linux
# nuwax_cli unit-test executable produced by cargo test -p nuwax-cli --lib --no-run.
set -euo pipefail

if [[ $(uname -s) != Linux || $(id -u) == 0 ]]; then
    echo 'ERROR: this regression requires native Linux and a non-root caller' >&2
    exit 1
fi
if [[ $# != 1 || ! -x "$1" ]]; then
    echo 'Usage: test-package-preservation-linux.sh /absolute/path/to/nuwax_cli-unit-test-binary' >&2
    exit 1
fi
test_binary=$(realpath "$1")
sudo -n true
fixture=$(mktemp -d /tmp/nuwax-package-permission.XXXXXXXX)
export NUWAX_TEST_ROOT_OWNED_FIXTURE="$fixture/docker"
touch "$fixture/.nuwax-owned-permission-fixture"

cleanup() {
    if [[ -f "$fixture/.nuwax-owned-permission-fixture" && "$fixture" == /tmp/nuwax-package-permission.* ]]; then
        sudo -n rm -rf -- "$fixture"
    fi
}
trap cleanup EXIT

python3 - <<'PY'
import os
from pathlib import Path
root = Path(os.environ['NUWAX_TEST_ROOT_OWNED_FIXTURE'])
for name in ('data/mysql', 'project_workspace', 'logs/rcoder', 'upload'):
    (root / name).mkdir(parents=True, exist_ok=True)
for name, value in {
    'data/mysql/row': 'persistent database contents',
    'project_workspace/project': 'workspace',
    'logs/rcoder/api.log': 'existing log',
    '.env': "# operator\nA='value # literal'\n",
    'docker-compose.yml': 'old compose',
}.items():
    (root / name).write_text(value)
rejected = root.parent / 'rejected/docker'
(rejected / 'unwritable-managed').mkdir(parents=True)
(rejected / 'unwritable-managed/file').write_text('old managed config')
(rejected / 'docker-compose.yml').write_text('old compose')
(rejected / '.env').write_text('A=operator\n')
PY

# Only this newly-created, marked fixture is changed, never a deployment tree.
sudo -n chown -R 0:0 -- "$fixture/docker/data" "$fixture/docker/project_workspace" "$fixture/docker/logs" "$fixture/rejected/docker/unwritable-managed"
sudo -n chmod 0755 -- "$fixture/docker/data" "$fixture/docker/project_workspace" "$fixture/docker/logs" "$fixture/rejected/docker/unwritable-managed"
sudo -n chmod 0000 -- "$fixture/docker/data/mysql/row" "$fixture/docker/logs/rcoder/api.log"

sudo -n python3 - "$fixture" <<'PY'
import hashlib, json, sys
from pathlib import Path
fixture = Path(sys.argv[1])
result = {}
for relative in ('', 'data', 'data/mysql', 'data/mysql/row', 'project_workspace', 'project_workspace/project', 'logs', 'logs/rcoder', 'logs/rcoder/api.log', 'upload'):
    path = fixture / 'docker' / relative
    state = path.stat()
    result[relative] = [state.st_dev, state.st_ino, state.st_uid, state.st_gid,
                        state.st_mode, hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None]
(fixture / 'before-protected.json').write_text(json.dumps(result))
PY

"$test_binary" --exact utils::package_replace::tests::native_root_owned_fixture --ignored --nocapture

sudo -n python3 - "$fixture" <<'PY'
import hashlib, json, sys
from pathlib import Path
fixture = Path(sys.argv[1])
before = json.loads((fixture / 'before-protected.json').read_text())
for relative, expected in before.items():
    path = fixture / 'docker' / relative
    state = path.stat()
    current = [state.st_dev, state.st_ino, state.st_uid, state.st_gid,
               state.st_mode, hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None]
    if current != expected:
        raise SystemExit(f'FAIL: protected metadata/content changed: {relative}')
print('PASS: native non-root replacement preserved root-owned trees and unreadable contents; unwritable managed parent refused before apply')
PY
