#!/bin/sh
exec docker exec -i -w /workspace terminal-gpui-dev rust-analyzer "$@"
