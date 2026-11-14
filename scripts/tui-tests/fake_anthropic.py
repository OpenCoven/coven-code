#!/usr/bin/env python3
"""Local stand-in for the Anthropic Messages API, for engine-local tool journeys.

Binds to 127.0.0.1 only. The first reply to a prompt asks the engine to run a
real tool call: "tool: NAME JSON" for that tool and input, "run: COMMAND" for
Bash with that command, or otherwise a Bash `touch` of a marker named by the
prompt's last word. The reply to its tool result reports what the engine sent
back, so a journey can see the outcome on screen. "flood: N" instead sends N
text deltas in one burst, to put the interactive event path under pressure;
"flood: N hold" sends them and then keeps the stream open until a file named
`release` appears in MARKER_DIR, so a journey can act while the turn is open.
Every request's messages are appended to the log for later checks.

Usage: fake_anthropic.py PORT_FILE LOG_FILE MARKER_DIR
"""
import http.server
import json
import os
import sys
import time

DENIED_TEXT = "TOOL_RESULT_ERROR"
ALLOWED_TEXT = "TOOL_RESULT_OK"
COMMAND_PREFIX = "run: "  # the rest of the prompt is the command, verbatim
TOOL_PREFIX = "tool: "  # then a tool name and its JSON input
FLOOD_PREFIX = "flood: "  # then how many text deltas to send in one burst
FLOOD_TEXT = "flood line\n"
HOLD_WORD = "hold"  # after the count: keep the stream open until released
RELEASE_MARKER = "release"
HOLD_LIMIT_SECONDS = 300


def sse(event, data):
    return f"event: {event}\ndata: {json.dumps(data)}\n\n".encode()


def message_start():
    return sse("message_start", {
        "type": "message_start",
        "message": {
            "id": "msg_fake", "type": "message", "role": "assistant", "content": [],
            "model": "fake-model", "stop_reason": None, "stop_sequence": None,
            "usage": {"input_tokens": 1, "output_tokens": 1},
        },
    })


def message_end(stop_reason):
    return (
        sse("message_delta", {
            "type": "message_delta",
            "delta": {"stop_reason": stop_reason, "stop_sequence": None},
            "usage": {"output_tokens": 1},
        })
        + sse("message_stop", {"type": "message_stop"})
    )


def text_turn(text):
    return (
        message_start()
        + sse("content_block_start", {
            "type": "content_block_start", "index": 0,
            "content_block": {"type": "text", "text": ""},
        })
        + sse("content_block_delta", {
            "type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": text},
        })
        + sse("content_block_stop", {"type": "content_block_stop", "index": 0})
        + message_end("end_turn")
    )


def flood_head(count):
    return (
        message_start()
        + sse("content_block_start", {
            "type": "content_block_start", "index": 0,
            "content_block": {"type": "text", "text": ""},
        })
        + sse("content_block_delta", {
            "type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": FLOOD_TEXT},
        }) * count
    )


def flood_tail():
    return (
        sse("content_block_stop", {"type": "content_block_stop", "index": 0})
        + message_end("end_turn")
    )


def flood_turn(count):
    return flood_head(count) + flood_tail()


class Held:
    """A reply sent in two parts: the tail waits for the release marker."""

    def __init__(self, head, tail, release):
        self.head, self.tail, self.release = head, tail, release


def tool_turn(tool_id, name, tool_input):
    return (
        message_start()
        + sse("content_block_start", {
            "type": "content_block_start", "index": 0,
            "content_block": {"type": "tool_use", "id": tool_id, "name": name, "input": {}},
        })
        + sse("content_block_delta", {
            "type": "content_block_delta", "index": 0,
            "delta": {"type": "input_json_delta", "partial_json": json.dumps(tool_input)},
        })
        + sse("content_block_stop", {"type": "content_block_stop", "index": 0})
        + message_end("tool_use")
    )


def last_tool_result(message):
    content = message.get("content")
    if isinstance(content, list):
        for block in content:
            if isinstance(block, dict) and block.get("type") == "tool_result":
                return block
    return None


def message_text(message):
    content = message.get("content")
    if isinstance(content, str):
        return content
    return " ".join(
        block.get("text", "") for block in content or []
        if isinstance(block, dict) and block.get("type") == "text"
    )


def reply_for(messages, marker_dir, turn):
    """The next assistant turn for this conversation."""
    last = messages[-1]
    result = last_tool_result(last)
    if result is not None:
        return text_turn(DENIED_TEXT if result.get("is_error") else ALLOWED_TEXT)
    text = message_text(last)
    tool_id = f"toolu_fake_{turn}"
    if text.startswith(FLOOD_PREFIX):
        words = text[len(FLOOD_PREFIX):].split()
        count = int(words[0])
        if words[1:] == [HOLD_WORD]:
            return Held(flood_head(count), flood_tail(),
                        os.path.join(marker_dir, RELEASE_MARKER))
        return flood_turn(count)
    if text.startswith(TOOL_PREFIX):
        name, _, tool_input = text[len(TOOL_PREFIX):].partition(" ")
        return tool_turn(tool_id, name, json.loads(tool_input))
    if text.startswith(COMMAND_PREFIX):
        return tool_turn(tool_id, "Bash", {"command": text[len(COMMAND_PREFIX):]})
    words = text.split()
    marker = os.path.join(marker_dir, words[-1] if words else "unnamed")
    return tool_turn(tool_id, "Bash", {"command": f"touch '{marker}'"})


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    log_path = ""
    marker_dir = ""
    turn = 0

    def log_message(self, *_):
        pass

    def respond(self, status, content_type, body):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        self.respond(200, "application/json", b'{"data":[]}')

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        request = json.loads(self.rfile.read(length) or b"{}")
        if not request.get("stream"):
            # Side requests such as title generation are not part of the turn.
            self.respond(200, "application/json", json.dumps({
                "id": "msg_side", "type": "message", "role": "assistant",
                "content": [{"type": "text", "text": "side"}], "model": "fake-model",
                "stop_reason": "end_turn", "stop_sequence": None,
                "usage": {"input_tokens": 1, "output_tokens": 1},
            }).encode())
            return
        messages = request.get("messages") or [{}]
        with open(self.log_path, "a", encoding="utf-8") as log:
            log.write(json.dumps({"last": messages[-1]}) + "\n")
        Handler.turn += 1
        reply = reply_for(messages, self.marker_dir, Handler.turn)
        if isinstance(reply, Held):
            self.respond_held(reply)
        else:
            self.respond(200, "text/event-stream", reply)

    def respond_held(self, reply):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Content-Length", str(len(reply.head) + len(reply.tail)))
        self.end_headers()
        self.wfile.write(reply.head)
        self.wfile.flush()
        deadline = time.monotonic() + HOLD_LIMIT_SECONDS
        while not os.path.exists(reply.release) and time.monotonic() < deadline:
            time.sleep(0.05)
        self.wfile.write(reply.tail)


def main():
    port_file, Handler.log_path, Handler.marker_dir = sys.argv[1:4]
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    with open(port_file + ".tmp", "w", encoding="utf-8") as f:
        f.write(str(server.server_address[1]))
    os.rename(port_file + ".tmp", port_file)
    server.serve_forever()


if __name__ == "__main__":
    main()
