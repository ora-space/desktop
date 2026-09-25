#!/usr/bin/env bash

logdy_port=${1:-8090}
set -- .data/logs/ora.log.*
exec logdy follow --full-read "$@" --ui-ip 127.0.0.1 --port "$logdy_port" --no-analytics --no-updates --config logdy.config.json
