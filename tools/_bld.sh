#!/bin/sh
# build + copy exactly this worktree's exe (the shared target's game.exe is overwritten by other worktrees)
R=$(cd "$(dirname "$0")/.." && pwd)
export CARGO_TARGET_DIR=$R/target
exe=$(cargo build -p game --message-format=json 2>/tmp/att_build.err | python -c "
import sys,json
p=''
for l in sys.stdin:
    try: m=json.loads(l)
    except: continue
    if m.get('executable') and m.get('target',{}).get('name')=='game': p=m['filenames'][0]
print(p)")
grep -E "^error" -A9 /tmp/att_build.err | head -40
[ -n "$exe" ] && cp "$exe" $R/target/debug/game-att.exe && echo "copied $exe"
