#!/bin/sh
# Execute a stable launcher while reading each newly written scenario as data.
exec /bin/sh "${0%/*}/claude" "$@"
