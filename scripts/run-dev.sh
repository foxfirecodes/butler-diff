#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
app="$PWD/target/debug/Butler Diff Dev.app"

# Restart only this development bundle so newly built code actually runs.
# The regular app and standalone binaries are left alone.
python3 - "$app/Contents/MacOS/butler-diff" <<'PY'
import os, signal, subprocess, sys, time
executable = sys.argv[1]
processes = subprocess.run(['ps', '-axo', 'pid=,command='], capture_output=True, text=True, check=True).stdout
stopping = []
for line in processes.splitlines():
    fields = line.strip().split(None, 1)
    if len(fields) == 2 and (fields[1] == executable or fields[1].startswith(executable + ' ')):
        try:
            os.kill(int(fields[0]), signal.SIGTERM)
            stopping.append(int(fields[0]))
        except ProcessLookupError:
            pass
deadline = time.monotonic() + 5
while stopping:
    for pid in stopping[:]:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            stopping.remove(pid)
    if stopping and time.monotonic() >= deadline:
        sys.exit('Butler Diff Dev did not exit; refusing to launch an older build.')
    if stopping:
        time.sleep(0.05)
PY
open -g "$app" --args --background "$@"
