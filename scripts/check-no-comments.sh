#!/usr/bin/env bash
set -euo pipefail
comment_at_line_start='^[[:space:]]*(//|/\*|\*/)'
trailing_comment='[^[:space:]:"][[:space:]]+//([[:space:]]|/|!|$)'
if matches=$(grep -rnE --include='*.rs' "${comment_at_line_start}|${trailing_comment}" src tests benches); then
    echo "Comments are not allowed in Rust source files:"
    echo "${matches}"
    exit 1
fi
