#!/usr/bin/env python3
"""Phase 1 spike: drive the native `claude` CLI over stream-json (no Node adapter).

Usage: claude_spike.py <scenario> <workspace> <transcript.jsonl>
Scenarios: hello | approval | interrupt | resume
`--permission-prompt-tool stdio` makes Claude send `can_use_tool` control requests
on the same stream instead of needing an MCP endpoint. Research tool, not product code.
"""
import json, os, queue, re, subprocess, sys, threading, time

scenario, workspace, out_path = sys.argv[1:4]
HOME = os.path.expanduser("~")
EMAIL = re.compile(r"[\w.+-]+@[\w-]+\.[\w.-]+")
SENSITIVE = {"email", "emailAddress", "organizationName", "accountUuid", "organizationUuid", "apiKeySource"}
out = open(out_path, "w")
t0 = time.time()

def scrub(v):
    if isinstance(v, dict):
        return {k: ("<redacted>" if k in SENSITIVE and x else scrub(x)) for k, x in v.items()}
    if isinstance(v, list):
        return [scrub(x) for x in v]
    return v

def log(direction, msg):
    text = EMAIL.sub("<email>", json.dumps(scrub(msg))).replace(HOME, "~")
    out.write(json.dumps({"dir": direction, "t": round(time.time() - t0, 3), "msg": json.loads(text)}) + "\n")
    out.flush()

def launch(extra=()):
    cmd = ["claude", "-p", "--input-format", "stream-json", "--output-format", "stream-json",
           "--verbose", "--include-partial-messages", "--permission-prompt-tool", "stdio",
           "--permission-mode", "default", *extra]
    return subprocess.Popen(cmd, cwd=workspace, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, text=True, bufsize=1)

class Session:
    def __init__(self, extra=()):
        self.proc = launch(extra)
        self.inbox = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()
        threading.Thread(target=self._err, daemon=True).start()
        self.session_id = None

    def _read(self):
        for line in self.proc.stdout:
            line = line.strip()
            if line:
                msg = json.loads(line)
                log("recv", msg)
                self.inbox.put(msg)
        self.inbox.put(None)

    def _err(self):
        for line in self.proc.stderr:
            out.write(json.dumps({"dir": "stderr", "t": round(time.time() - t0, 3), "msg": line.strip()[:300]}) + "\n")

    def send(self, msg):
        log("send", msg)
        self.proc.stdin.write(json.dumps(msg) + "\n")
        self.proc.stdin.flush()

    def user(self, text):
        self.send({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": text}]}})

    def pump(self, until, timeout=120, on_control=None):
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                msg = self.inbox.get(timeout=1)
            except queue.Empty:
                continue
            if msg is None:
                raise SystemExit("claude exited")
            if msg.get("type") == "system" and msg.get("subtype") == "init":
                self.session_id = msg.get("session_id")
            if msg.get("type") == "control_request" and on_control:
                on_control(self, msg)
            if until(msg):
                return msg
        raise SystemExit("timeout")

    def close(self):
        try:
            self.proc.stdin.close()
            self.proc.wait(timeout=8)
        except Exception:
            self.proc.kill()

is_result = lambda m: m.get("type") == "result"

if scenario == "hello":
    s = Session()
    s.user("Reply with exactly the word: ok")
    s.pump(is_result)
    s.close()
elif scenario == "approval":
    asked = []
    def control(session, msg):
        req = msg["request"]
        asked.append(req.get("subtype"))
        if req.get("subtype") == "can_use_tool":
            session.send({"type": "control_response", "response": {
                "subtype": "success", "request_id": msg["request_id"],
                "response": {"behavior": "deny", "message": "Denied by the spike client."}}})
    s = Session()
    s.user("Create a file named note.txt containing the word hi using the Write tool.")
    s.pump(is_result, on_control=control)
    print("control requests:", asked)
    s.close()
elif scenario == "interrupt":
    s = Session()
    s.user("Count slowly from 1 to 200, one number per line, with a sentence after each.")
    s.pump(lambda m: m.get("type") == "stream_event", timeout=60)
    time.sleep(1.0)
    s.send({"type": "control_request", "request_id": "int-1", "request": {"subtype": "interrupt"}})
    s.pump(is_result)
    s.close()
elif scenario == "resume":
    s = Session()
    s.user("Remember the word: tangerine. Reply with just: stored")
    s.pump(is_result)
    sid = s.session_id
    s.close()
    s2 = Session(("--resume", sid))
    s2.user("What word did I ask you to remember? Reply with just the word.")
    s2.pump(is_result)
    s2.close()
print("done", scenario)
