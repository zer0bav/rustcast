#!/bin/sh
# rustcast plugin protocol: receive the query as $1, print JSON on stdout.
#   { "items": [ { "title", "subtitle"?, "icon"?, "action": {"kind","data"} } ] }
# action.kind is one of: copy | open | shell | launch
query="$1"
upper=$(printf '%s' "$query" | tr '[:lower:]' '[:upper:]')
cat <<JSON
{ "items": [
  { "title": "echo: $query", "subtitle": "copy the text", "action": { "kind": "copy", "data": "$query" } },
  { "title": "UPPER: $upper", "subtitle": "copy uppercased", "action": { "kind": "copy", "data": "$upper" } }
] }
JSON
