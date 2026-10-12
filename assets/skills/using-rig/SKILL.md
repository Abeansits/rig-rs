# Using rig

`rig setup` writes per-seat provider files, a tmux managed block, and this skill.
`rig doctor` reports pass, warn, fail, or skipped and does not start the daemon.

## Stopping the daemon

A graceful stop shuts down the tmux server that this daemon started. That includes the process exiting on its own and `POST /shutdown`.

`kill -9` skips that cleanup. The tmux server keeps running, reparented away from the daemon. The next daemon attaches to that same server (`new-session -A`) instead of starting a second one. A later graceful stop is what shuts it down.

If a tmux server was already running on the daemon's socket when it started, the daemon attaches and does not kill that server on the way out.
