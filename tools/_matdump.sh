#!/bin/sh
# dev: re-export the lobby with every material's cooked params dumped to $1/mats.jsonl
mkdir -p "$1"
export SCENE_MATDUMP="$1/mats.jsonl"
cd C:/code/bodycam-player/.claude/worktrees/agent-a68d0ff5c8634ecd6/tools/ue-export
sh run.sh "$1" scene Bodycam/Content/Map/Lobby/Lobby > "$1/log.txt" 2>&1
tail -3 "$1/log.txt"
