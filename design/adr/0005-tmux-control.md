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

## list-clients format

Human-client reads use

```
list-clients -F '#{client_control_mode}|#{client_activity}|#{client_tty}|#{client_session}'
```

Session is last so a `|` inside a session name stays one field. This replaces the
space-separated `client_control_mode client_session client_activity client_tty`
order in the original ADR.
