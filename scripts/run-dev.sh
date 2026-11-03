#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
app="$PWD/target/debug/Gander Dev.app"

# Restart only this development bundle so newly built code actually runs.
# The regular app and standalone binaries are left alone.
python3 - "$app/Contents/MacOS/gander" <<'PY'
import os, signal, subprocess, sys, time
executable = sys.argv[1]
processes = subprocess.run(['ps', '-axo', 'pid=,command='], capture_output=True, text=True, check=True).stdout
stopping = []
for line in processes.splitlines():
    fields = line.strip().split(None, 1)
    if len(fields) == 2 and (fields[1] == executable or fields[1].startswith(executable + ' ')):
        pid = int(fields[0])
        try:
            if fields[1] == executable + ' --semantic-acp':
                # Inference helpers own a separate process group. Reap the whole
                # group, including Codex, even if its GUI parent already exited.
                if os.getpgid(pid) == pid:
                    os.killpg(pid, signal.SIGKILL)
            else:
                os.kill(pid, signal.SIGTERM)
                stopping.append(pid)
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
        sys.exit('Gander Dev did not exit; refusing to launch an older build.')
    if stopping:
        time.sleep(0.05)
PY
log="$PWD/target/debug/gander-dev.log"
: > "$log"
open -g "$app" --stderr "$log" --args --background "$@"
