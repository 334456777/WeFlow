"""PGO training of `weflow serve` (#500), run by pgo-build.sh on its synthetic account.

usage: pgo-serve.py <weflow> <group id> <private chat id>   (WEFLOW_HOME names the account's home)

Starts the server, calls every GET route of the command API's in-process reads (`/api/`, from `/openapi.json`)
twice with the ids of the account, a few with their other options too (parsed rows, the next page, thumbnails), then
stops the server the way Ctrl+C does so that it writes its profile: a killed process leaves none. Routes that go to
the network, wait for events or need what the account lacks are left out. A failure is printed as a warning; the
build goes on without what it would have counted.
"""
import json
import os
import signal
import subprocess
import sys
import threading
import urllib.error
import urllib.parse
import urllib.request

TOKEN = "pgo-training"
SKIPPED = {
    "/api/push/messages",  # an event stream that never ends
    "/api/sns/media",  # downloads from WeChat's servers
    "/api/insight/get",  # needs an insight the account has none of
    "/api/db/detect",  # looks for WeChat's folders on the machine
    "/api/db/wxid",
}


def warn(message):
    print(f"::warning::PGO serve training: {message}", flush=True)


def main():
    weflow, group, friend = sys.argv[1:4]
    weflow = os.path.abspath(weflow)  # Windows looks a relative path with `/` up nowhere
    windows = os.name == "nt"
    if windows:
        import ctypes

        # whether Ctrl+C is ignored passes to the child: the server must not inherit it
        ctypes.windll.kernel32.SetConsoleCtrlHandler(None, False)
    server = subprocess.Popen(
        [weflow, "--json", "serve", "--port", "0", "--api-token", TOKEN, "--api-only"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
        # a console of its own, for the Ctrl+C of `stop` to reach the server alone
        creationflags=subprocess.CREATE_NEW_CONSOLE if windows else 0,
    )
    try:
        base = None
        for line in server.stderr:
            try:
                event = json.loads(line)
            except ValueError:
                continue
            if isinstance(event, dict) and event.get("type") == "server_started":
                base = event["url"].rstrip("/")
                break
        if base is None:
            warn("the server did not start")
            return
        threading.Thread(target=lambda: [None for _ in server.stderr], daemon=True).start()
        train(base, group, friend)
    finally:
        stop(server, windows)


def get(base, path, params=None, raw=False, quiet=False):
    url = base + path + ("?" + urllib.parse.urlencode(params) if params else "")
    request = urllib.request.Request(url, headers={"Authorization": f"Bearer {TOKEN}"})
    try:
        with urllib.request.urlopen(request, timeout=120) as reply:
            body = reply.read()
    except (urllib.error.URLError, TimeoutError) as err:
        if not quiet:
            warn(f"{path}: {err}")
        return None
    if raw or reply.headers.get_content_type() != "application/json":
        return body
    return json.loads(body).get("data")


def train(base, group, friend):
    spec = json.loads(get(base, "/openapi.json", raw=True) or b"{}")
    rows = get(base, "/api/chat/messages", {"session-id": group, "limit": 3000, "parsed": "true"}) or []
    messages = rows.get("messages", []) if isinstance(rows, dict) else rows

    def media_call(path, kind):
        # not every message has its file on disk: the first that answers
        for m in [m for m in messages if m.get("localType") == kind][:10]:
            params = {"session-id": group, "msg-id": m["localId"], "create-time": m["createTime"]}
            if get(base, path, params, quiet=True) is not None:
                return (path, params)
        return None

    members = get(base, "/api/group/members", {"chatroom-id": group}) or []
    member = next((m.get("username") for m in members if isinstance(m, dict) and m.get("username")), friend)
    accounts = get(base, "/api/biz/accounts") or []
    account = next((a.get("username") for a in accounts if isinstance(a, dict) and a.get("username")), None)
    media = {"/api/chat/image-data": 3, "/api/chat/image-info": 3, "/api/chat/voice-data": 34, "/api/chat/video-data": 43}

    calls = []
    for path, methods in spec.get("paths", {}).items():
        if "get" not in methods or not path.startswith("/api/") or path in SKIPPED:
            continue
        params = {}
        for p in methods["get"].get("parameters", []):
            if "$ref" in p:
                p = spec["components"]["parameters"][p["$ref"].rsplit("/", 1)[-1]]
            if not p.get("required"):
                continue
            name = p["name"]
            if name in ("session-id", "chatroom-id"):
                params[name] = group
            elif name == "keyword":
                params[name] = "好"
            elif name == "username":
                params[name] = account if path.startswith("/api/biz/") else member if path.startswith("/api/group/") else friend
            else:
                params = None
                break
        if path in media:
            call = media_call(path, media[path])
            if call:
                calls.append(call)
        elif params is not None and all(v is not None for v in params.values()):
            if path == "/api/chat/message" and messages:
                params.update({"local-id": messages[0]["localId"], "create-time": messages[0]["createTime"]})
            calls.append((path, params))
    for session in (group, friend):
        calls += [
            ("/api/chat/messages", {"session-id": session, "limit": 200}),
            ("/api/chat/messages", {"session-id": session, "limit": 200, "parsed": "true"}),
            ("/api/chat/messages", {"session-id": session, "limit": 200, "offset": 1000, "parsed": "true"}),
            ("/api/chat/search", {"keyword": "的", "session-id": session, "parsed": "true"}),
            ("/api/chat/detail", {"session-id": session, "extra": "true"}),
        ]
    if messages and messages[-1].get("_cursor"):
        calls.append(("/api/chat/messages", {"session-id": group, "limit": 200, "before": messages[-1]["_cursor"]}))
    image = next((params for path, params in calls if path == "/api/chat/image-data"), None)
    if image:
        calls.append(("/api/chat/image-data", {**image, "thumb": "true"}))
    calls.append(("/api/chat/contacts", {"parsed": "true"}))
    for _ in range(2):
        for path, params in calls:
            get(base, path, params)
    print(f"PGO serve training: {len(calls)} reads, twice", flush=True)


def stop(server, windows):
    if server.poll() is not None:
        warn(f"the server exited early ({server.returncode})")
        return
    if windows:
        # a Ctrl+C can only be sent to a console: a helper attaches to the server's and sends one there
        subprocess.run([sys.executable, __file__, "--ctrl-c", str(server.pid)], check=False)
    else:
        server.send_signal(signal.SIGINT)
    try:
        server.wait(timeout=60)
    except subprocess.TimeoutExpired:
        warn("the server did not stop: killed, its profile is lost")
        server.kill()
        server.wait()


def ctrl_c(pid):
    import ctypes

    kernel = ctypes.windll.kernel32
    kernel.FreeConsole()
    if not kernel.AttachConsole(pid):
        sys.exit(f"cannot attach to the console of {pid}")
    kernel.SetConsoleCtrlHandler(None, True)  # this helper ignores the event it sends
    kernel.GenerateConsoleCtrlEvent(0, 0)  # CTRL_C_EVENT to every process of the console


if __name__ == "__main__":
    if sys.argv[1:2] == ["--ctrl-c"]:
        ctrl_c(int(sys.argv[2]))
    else:
        main()
