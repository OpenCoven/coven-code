#!/usr/bin/env python3
"""Throwaway (#568 evidence): apply one named mutation to the checked-out test files."""
import sys

name, variant = sys.argv[1], sys.argv[2]
OAUTH = "crates/mcp/src/oauth.rs"
ENDPOINT = "crates/mcp/src/rmcp_endpoint_privacy_tests.rs"
PARITY = "crates/tui/tests/presentation_parity.rs"
POLICY = "crates/cli/tests/workspace_policy.rs"
ADDR = 'redirect_uri.trim_start_matches("http://").trim_end_matches("/callback")'


def edit(path, old, new, count=1):
    text = open(path).read()
    found = text.count(old)
    assert found == count, f"{name}/{variant}: {path}: anchor found {found} times, want {count}: {old!r}"
    open(path, "w").write(text.replace(old, new))


if name == "A":
    # Another holder takes the callback port before the flow binds it (every
    # attempt on main, the first attempt only on the fix).
    if variant == "before":
        anchor = '        let redirect_uri = session.redirect_uri.clone();\n        let (opened, ready) = oneshot::channel();\n'
        edit(OAUTH, anchor, anchor + f'        let _steal = std::net::TcpListener::bind({ADDR}).unwrap();\n')
        # A foreign listener answers on the port the refusal tests freed.
        edit(ENDPOINT, "    drop(socket);\n",
             "    drop(socket);\n    let foreign = std::net::TcpListener::bind(addr).unwrap();\n"
             "    std::thread::spawn(move || for stream in foreign.incoming() { drop(stream); });\n")
        edit(ENDPOINT, "    drop(refused);\n",
             "    let foreign_addr = refused.local_addr().unwrap();\n    drop(refused);\n"
             "    let foreign = std::net::TcpListener::bind(foreign_addr).unwrap();\n"
             "    std::thread::spawn(move || for stream in foreign.incoming() { drop(stream); });\n")
    else:
        anchor = '            let redirect_uri = session.redirect_uri.clone();\n            let (opened, ready) = oneshot::channel();\n'
        edit(OAUTH, anchor, anchor + f'            let _steal = (attempts == 1).then(|| std::net::TcpListener::bind({ADDR}).unwrap());\n')
        anchor = '            let redirect_port = oauth_port_alloc().expect("free port");\n'
        edit(OAUTH, anchor, anchor + '            let _steal = (attempts == 1).then(|| std::net::TcpListener::bind(("127.0.0.1", redirect_port)).unwrap());\n')
elif name == "B":
    # Another holder takes the callback port for 300 ms right after the flow
    # releases it.
    anchor = '        assert!(error.to_string().contains("revoked"));\n'
    edit(OAUTH, anchor, anchor
         + f'        let held = std::net::TcpListener::bind({ADDR}).unwrap();\n'
         + '        std::thread::spawn(move || { std::thread::sleep(std::time::Duration::from_millis(300)); drop(held); });\n')
elif name == "P":
    # The two draws fall either side of a second of the running turn.
    edit(PARITY, "draw(&app, UiRenderer::Next, 120, 40),",
         "{ let frame = draw(&app, UiRenderer::Next, 120, 40); std::thread::sleep(std::time::Duration::from_millis(1100)); frame },",
         count=2)
elif name == "S":
    # The runner stalls 6 s between reading the shell's PID and the stop.
    anchor = ("    // Coven's Stop for a harness run:" if variant == "before"
              else "    // Should the delayed write be found below")
    edit(POLICY, anchor, "    thread::sleep(Duration::from_secs(6));\n" + anchor)
else:
    raise SystemExit(f"unknown mutation {name}")
