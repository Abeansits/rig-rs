# ADR-0005 amendments (slice 1b)

These points revise the accepted tmux-control ADR. The rest of that decision is unchanged.

## Pane-death doorbell

tmux 3.3a does not emit `%paste-buffer-changed`. The `pane-died` hook still runs
`set-buffer -b rig-dead-#{hook_pane}` and also
`rename-window -t <bell> rig-dead-#{hook_pane}` on the hidden bell window in
`crewmux-control`.

`%window-renamed` and `%unlinked-window-renamed` count as that doorbell only when
the window id is the bell window. A rename of any other window is ignored, even
when the new name looks like `rig-dead-%N`.

Ubuntu's tmux is linked with libutempter. Closing the pane pty can swallow
`SIGCHLD`, so `pane_dead` is set while `pane_dead_status` stays empty and the
hook never runs. Upstream tmux sends itself `SIGCHLD` after those utempter
calls. On Linux, while the `-L` name is `rig-rs` or `rig-rs-*` and some pane is
dead with an empty `pane_dead_status`, this crate sends `SIGCHLD` to the server
pid directly (`rustix`, not a `kill` process) so `waitpid` records the status
and the hook fires. The pid is reread when the control client reconnects and
cleared when the server exits, so a reused pid is not signaled. Other sockets,
including `default`, are not signaled. Other platforms already reap.

## Linked windows

Seat windows are linked into `crewmux-control` so the control client receives
`%output`. `kill-session` does not destroy a window that is still linked
elsewhere, so the seat window and its pane process would survive `down`.
`kill_session` unlinks that window from `crewmux-control` and then kills the
session. The bell window is only in the control session and stays.

## list-clients format

Human-client reads use

```
list-clients -F '#{client_control_mode}|#{client_activity}|#{client_tty}|#{client_session}'
```

Session is last so a `|` inside a session name stays one field. This replaces the
space-separated `client_control_mode client_session client_activity client_tty`
order in the original ADR.
