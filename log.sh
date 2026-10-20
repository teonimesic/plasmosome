#!/bin/sh
echo "WRAPPER RAN: $*" >> "$GITHUB_STEP_SUMMARY"
echo "WRAPPER RAN: $*" >&2
exec "$@"
