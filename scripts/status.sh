#!/usr/bin/env bash
set -euo pipefail
for dir in \
  platform/ocean-os \
  apps/ocean-surface \
  packages/ocean-agents \
  services/ocean-bedrock \
  org/risingtides-agents
do
  if [[ -d "$dir" ]]; then
    printf '\n== %s ==\n' "$dir"
    git log -1 --format='commit: %h  date: %cs  subject: %s' -- "$dir"
  else
    printf '\nMISSING: %s\n' "$dir"
  fi
done
