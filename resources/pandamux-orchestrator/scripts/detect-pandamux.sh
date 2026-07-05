#!/usr/bin/env bash
# Detect if pandamux is running and available via named pipe.
# Exit 0 + print "available" if pandamux responds to ping.
# Exit 1 + print "unavailable" if not.

if command -v pandamux &>/dev/null; then
  result=$(pandamux ping 2>/dev/null)
  if [ "$result" = "pong" ]; then
    echo "available"
    exit 0
  fi
fi

# Fallback: try connecting to the pipe directly
if [ -e "//./pipe/pandamux" ] 2>/dev/null; then
  echo "available"
  exit 0
fi

echo "unavailable"
exit 1
