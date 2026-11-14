# CLI diagnostics

Interactive sessions keep a bounded 64 KiB diagnostic tail in memory by default. No diagnostic directory or file is created. Open `/status logs` to read and search recent diagnostics in either Classic or Next; Escape returns to the conversation. The reader is a snapshot of local diagnostics, separate from saved conversation messages and provider input. Earlier bytes are discarded when the tail fills, with an omission count.

To save diagnostics for one interactive session, start Coven Code with `--diagnostics`. Files are written to `logs/session-<UUID>.log` under the engine configuration directory. The terminal shows the path at startup, including when resuming a session. `COVEN_CODE_HOME` overrides the engine directory; otherwise normal Coven home resolution applies. Persistence requires this explicit opt-in on each invocation.

Files contain timestamps and no ANSI colors. Captured control characters are escaped for safe terminal display. Existing verbosity and `RUST_LOG` filters still apply. Headless, print, positional-prompt, context and `--dump-system-prompt` invocations retain stderr logging. Redirecting interactive stderr also keeps diagnostics on that caller-controlled stream while retaining the memory tail, for example:

```sh
coven-code 2>diagnostics.log
```

Each opted-in file is capped at 1 MiB. When the next event would exceed the cap, older content is discarded and a marker precedes the new output. An individual event larger than the limit, including after control-character escaping, is replaced with an omission marker. At opted-in startup, pruning keeps the ten newest completed logs. Active sessions are locked and preserved; unrecognized files are left alone. This is a completed-file retention policy, not a total directory size limit when many sessions are active.

On Unix the directory is owner-only (0700), and files are created owner-only (0600). Windows uses inherited filesystem ACLs. Treat diagnostics as private: debug messages can include paths and request details; inspect them before copying or sharing them.

Failure to open an explicitly requested log stops startup with its directory in the error, before terminal setup. A later write failure stops file logging and adds a warning with the path to the terminal. The bounded memory tail remains available through `/status logs`. The file sink never falls back to terminal stderr.
