#!/usr/bin/env python3
"""Phase 1 spike: drive `codex app-server` over stdio (no Node adapter).

Usage: codex_spike.py <scenario> <workspace> <transcript.jsonl>
Scenarios: hello | list | two-threads | interrupt
Everything sent/received is written as JSON lines {dir, t, msg}; e-mail addresses
are redacted. Uses the machine's existing Codex login; prompts are tiny and
read-only. This is a research tool, not product code.
"""
import json, re, subprocess, sys, threading, time, queue

scenario, workspace, out_path = sys.argv[1:4]
import os
HOME = os.path.expanduser("~")
EMAIL = re.compile(r"[\w.+-]+@[\w-]+\.[\w.-]+")
out = open(out_path, "w")
t0 = time.time()

SENSITIVE_KEYS = {"installationId", "chatgptAccountId", "environmentId", "serverName", "backendOrigin", "email"}

def scrub(value):
    if isinstance(value, dict):
        return {k: ("<redacted>" if k in SENSITIVE_KEYS and v else scrub(v)) for k, v in value.items()}
    if isinstance(value, list):
        return [scrub(v) for v in value]
    return value

def log(direction, msg):
    text = EMAIL.sub("<email>", json.dumps(scrub(msg))).replace(HOME, "~")
    out.write(json.dumps({"dir": direction, "t": round(time.time() - t0, 3), "msg": json.loads(text)}) + "\n")
    out.flush()

proc = subprocess.Popen(["codex", "app-server"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE, text=True, bufsize=1)
inbox = queue.Queue()

def reader():
    for line in proc.stdout:
        line = line.strip()
        if line:
            msg = json.loads(line)
            log("recv", msg)
            inbox.put(msg)
    inbox.put(None)

def err_reader():
    for line in proc.stderr:
        out.write(json.dumps({"dir": "stderr", "t": round(time.time() - t0, 3), "msg": EMAIL.sub("<email>", line.strip())[:300]}) + "\n")

threading.Thread(target=reader, daemon=True).start()
threading.Thread(target=err_reader, daemon=True).start()
next_id = [0]

def send(msg):
    log("send", msg)
    proc.stdin.write(json.dumps(msg) + "\n")
    proc.stdin.flush()

def request(method, params=None):
    next_id[0] += 1
    rid = next_id[0]
    send({"id": rid, "method": method, **({"params": params} if params is not None else {})})
    return rid

def pump(until, timeout=90, on_server_request=None):
    """Read messages until `until(msg)` is true; answer server requests via the callback."""
    deadline = time.time() + timeout
    seen = []
    while time.time() < deadline:
        try:
            msg = inbox.get(timeout=1)
        except queue.Empty:
            continue
        if msg is None:
            raise SystemExit("app-server exited")
        seen.append(msg)
        if "method" in msg and "id" in msg and on_server_request:
            on_server_request(msg)
        if until(msg):
            return seen
    raise SystemExit(f"timeout; last messages: {seen[-3:]}")

def response_for(rid):
    return lambda m: m.get("id") == rid and "method" not in m

rid = request("initialize", {"clientInfo": {"name": "lumen-spike", "title": "Lumen spike", "version": "0"}})
init = pump(response_for(rid))[-1]
send({"method": "initialized"})

MODEL = None  # set by the "models" step

def start_thread(ephemeral=True):
    params = {"cwd": workspace, "sandbox": "read-only", "approvalPolicy": "on-request", "ephemeral": ephemeral}
    if MODEL:
        params["model"] = MODEL
    rid = request("thread/start", params)
    resp = pump(response_for(rid))[-1]
    return resp["result"]["thread"]["id"], resp

def run_turn(thread_id, text, on_server_request=None, interrupt_after=None):
    rid = request("turn/start", {"threadId": thread_id, "input": [{"type": "text", "text": text}]})
    started = pump(response_for(rid))[-1]
    turn_id = started["result"]["turn"]["id"]
    if interrupt_after is not None:
        time.sleep(interrupt_after)
        request("turn/interrupt", {"threadId": thread_id, "turnId": turn_id})
    msgs = pump(lambda m: m.get("method") == "turn/completed", on_server_request=on_server_request)
    return turn_id, msgs

def pick_model():
    global MODEL
    rid = request("model/list", {})
    resp = pump(response_for(rid))[-1]["result"]
    models = resp.get("data") or resp.get("models") or []
    # Prefer the first model that is not hidden and not marked unsupported.
    for m in models:
        if not m.get("hidden"):
            MODEL = m.get("model") or m.get("id")
            break

if scenario != "list":
    pick_model()

if scenario == "hello":
    request("account/read", {"refreshToken": False})
    tid, _ = start_thread()
    run_turn(tid, "Reply with exactly the word: ok")
elif scenario == "list":
    rid = request("thread/list", {"limit": 5})
    pump(response_for(rid))
elif scenario == "two-threads":
    a, _ = start_thread()
    b, _ = start_thread()
    # Interleave: start both turns, then wait for both completions.
    ra = request("turn/start", {"threadId": a, "input": [{"type": "text", "text": "Reply with exactly: alpha"}]})
    rb = request("turn/start", {"threadId": b, "input": [{"type": "text", "text": "Reply with exactly: beta"}]})
    done = set()
    pump(lambda m: (m.get("method") == "turn/completed" and not done.add(m["params"].get("threadId")) and len(done) == 2) or False)
elif scenario == "approval":
    # Read-only sandbox + on-request policy: writing a file must ask first. We decline.
    asked = []
    def answer(msg):
        asked.append(msg["method"])
        decision = "decline"
        send({"id": msg["id"], "result": {"decision": decision}})
    tid, _ = start_thread()
    run_turn(tid, "Create a file named note.txt containing the word hi. Use a shell command or a patch; do not just describe it.",
             on_server_request=answer)
    print("server requests:", asked)
elif scenario == "resume":
    # Persisted thread -> unsubscribe -> resume -> read history -> rename -> archive (cleans up).
    tid, _ = start_thread(ephemeral=False)
    run_turn(tid, "Remember the word tangerine. Reply with just: stored")
    for method, params in [("thread/unsubscribe", {"threadId": tid}),
                           ("thread/resume", {"threadId": tid, "cwd": workspace}),
                           ("thread/read", {"threadId": tid, "includeTurns": True}),
                           ("thread/name/set", {"threadId": tid, "name": "Lumen spike"}),
                           ("thread/list", {"limit": 5, "cwd": workspace}),
                           ("thread/archive", {"threadId": tid})]:
        pump(response_for(request(method, params)))
elif scenario == "interrupt":
    tid, _ = start_thread()
    run_turn(tid, "Count slowly from 1 to 200, one number per line, with a sentence after each.", interrupt_after=3)

proc.stdin.close()
try:
    proc.wait(timeout=5)
except subprocess.TimeoutExpired:
    proc.kill()
print("done", scenario)
