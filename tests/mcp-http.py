"""Exercise the desktop app and its HTTP endpoint against a temporary notebook."""

import json
import os
from pathlib import Path
import secrets
import subprocess
import sys
import tempfile
import time
from urllib.error import HTTPError
from urllib.request import Request, urlopen


URL = "http://127.0.0.1:8374/mcp"
VERSION = "2025-11-25"


def check_mcp(binary, root):
    data_dir = root / "think"
    data_dir.mkdir()
    notebook = data_dir / "notebook.txt"
    notebook.write_text("Starting note\n", encoding="utf-8")
    token = secrets.token_hex(32)
    token_file = data_dir / "mcp-token"
    token_file.touch(mode=0o600)
    token_file.write_text(token, encoding="ascii")
    env = {
        **os.environ,
        "XDG_DATA_HOME": str(root),
        "DBUS_SESSION_BUS_ADDRESS": f"unix:path={root}/no-session-bus",
        "WINIT_X11_SCALE_FACTOR": "1",
        "LIBGL_ALWAYS_SOFTWARE": "1",
    }
    env.pop("WAYLAND_DISPLAY", None)
    env.pop("WAYLAND_SOCKET", None)

    session = None
    request_id = 0

    def rpc(method, params, *, authorization=token, extra=None, status=200):
        nonlocal session, request_id
        request_id += 1
        headers = {
            "Accept": "application/json, text/event-stream",
            "Content-Type": "application/json",
            "MCP-Protocol-Version": VERSION,
        }
        if authorization is not None:
            headers["Authorization"] = f"Bearer {authorization}"
        if session is not None:
            headers["Mcp-Session-Id"] = session
        headers.update(extra or {})
        payload = {"jsonrpc": "2.0", "method": method, "params": params}
        if not method.startswith("notifications/"):
            payload["id"] = request_id
        req = Request(URL, json.dumps(payload).encode(), headers)
        try:
            response = urlopen(req, timeout=5)
        except HTTPError as error:
            response = error
        with response:
            assert response.status == status, (method, status, response.status)
            if status != 200:
                return None
            session = response.headers.get("Mcp-Session-Id", session)
            if response.headers.get_content_type() == "text/event-stream":
                for line in response:
                    if line.startswith(b"data:") and line[5:].strip():
                        message = json.loads(line[5:])
                        if message.get("id") == request_id:
                            break
                else:
                    raise AssertionError(f"No response for {method}")
            else:
                message = json.load(response)
            assert "error" not in message, message
            return message["result"]

    def tool(name, **arguments):
        return rpc("tools/call", {"name": name, "arguments": arguments})

    with subprocess.Popen([binary], env=env) as app:
        try:
            window = subprocess.check_output(
                [
                    "xdotool", "search", "--sync", "--onlyvisible", "--all",
                    "--pid", str(app.pid), "--name", "^Think$",
                ],
                text=True,
                timeout=15,
            ).strip()
            initialize = {
                "protocolVersion": VERSION,
                "capabilities": {},
                "clientInfo": {"name": "think-desktop-test", "version": "1"},
            }
            rpc("initialize", initialize, authorization=None, status=401)
            rpc("initialize", initialize, authorization="wrong", status=401)
            rpc("initialize", initialize, extra={"Origin": "https://example.invalid"}, status=403)
            rpc("initialize", initialize, extra={"Host": "example.invalid:8374"}, status=403)
            rpc("initialize", initialize)
            rpc("notifications/initialized", {}, status=202)

            initial = tool("read_notebook")["structuredContent"]
            assert initial["text"] == "Starting note\n", initial
            subprocess.run(
                ["xdotool", "windowfocus", "--sync", window, "mousemove", "--window", window, "70", "65", "click", "1"],
                check=True,
                timeout=5,
            )
            subprocess.run(["xdotool", "key", "ctrl+End"], check=True, timeout=5)
            subprocess.run(["xdotool", "type", "Human note"], check=True, timeout=5)

            for _ in range(30):
                live = tool("read_notebook")["structuredContent"]
                if live["text"] == "Starting note\nHuman note":
                    break
                time.sleep(0.1)
            else:
                raise AssertionError("MCP did not see text typed in the editor")

            stale = tool("insert_text", line=1, text="MCP note", expected_revision=initial["revision"])
            assert stale["isError"] is True, stale
            assert "Read it again" in stale["content"][0]["text"], stale
            inserted = tool("insert_text", line=1, text="MCP note", expected_revision=live["revision"])
            assert not inserted.get("isError"), inserted
            expected = "Starting note\nMCP note\nHuman note"
            assert notebook.read_text(encoding="utf-8") == expected
            assert tool("read_notebook")["structuredContent"]["text"] == expected

            subprocess.run(["xdotool", "key", "ctrl+End"], check=True, timeout=5)
            subprocess.run(["xdotool", "type", " plus more"], check=True, timeout=5)
            for _ in range(50):
                if notebook.read_text(encoding="utf-8") == expected + " plus more":
                    break
                time.sleep(0.1)
            else:
                raise AssertionError("Typing after the MCP insert did not retain both edits")
        finally:
            app.terminate()
            app.wait(timeout=5)


if __name__ == "__main__":
    with tempfile.TemporaryDirectory() as directory:
        check_mcp(sys.argv[1], Path(directory))
    print("MCP authorization, stale revision rejection, and overlapping editor input passed.")
