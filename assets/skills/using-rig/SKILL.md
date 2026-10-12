# Using rig

`rig setup` writes per-seat provider files, a tmux managed block, and this skill.
`rig doctor` reports pass, warn, fail, or skipped and does not start the daemon.

## Stopping the daemon

A graceful stop shuts down the tmux server that this daemon started. That includes the process exiting on its own and `POST /shutdown`. The whole server stops, including sessions a person added after the daemon started.

The daemon treats that server as its own only when the server is not older than the daemon's tmux client. `new-session -A` attaches when a server is already listening. An older server, including one already on the shared `rig-rs` socket, stays up on the way out.

`kill -9` skips that cleanup. The tmux server keeps running, reparented away from the daemon. The next daemon attaches to that same server and leaves it up, because the server is older than the new client.
