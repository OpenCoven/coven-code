# CLI diagnostics

When the interactive TUI owns terminal stderr, tracing diagnostics go to a private
`logs/session-<UUID>.log` under the engine configuration directory. The transcript
shows the full path at startup, including when resuming a session.
`COVEN_CODE_HOME` overrides the engine directory; otherwise normal Coven home
resolution applies.

Files contain timestamps and no ANSI colors. Existing verbosity and `RUST_LOG`
filters still apply. Headless, print, positional-prompt, context and
`--dump-system-prompt` invocations retain stderr logging. Redirecting interactive
stderr also keeps diagnostics on that stream, for example:

```sh
coven-code 2>diagnostics.log
```

Each file is capped at 1 MiB. When the next event would exceed the cap, older
content is discarded and a marker precedes the new output. An individual event
larger than the limit is replaced with an omission marker. At startup, pruning
keeps the ten newest completed logs. Active sessions are locked and preserved;
unrecognized files are left alone. This is a completed-file retention policy,
not a total directory size limit when many sessions are active.

On Unix the directory is owner-only (0700), and files are created owner-only
(0600). Windows uses inherited filesystem ACLs. Treat logs as private: debug
messages can include paths and request details; inspect them before sharing.

Failure to open the log stops startup with its directory in the error, before
terminal setup. A later write failure stops file logging and adds a warning with
the path to the TUI transcript. The file sink never falls back to stderr.
