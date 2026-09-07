#!/usr/bin/env python3
"""Trusted-local bounded coordination board. Python 3.10+, Linux, local SQLite."""
import argparse
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import sqlite3
import subprocess
import sys
import time
import uuid


class Error(ValueError):
    pass


def packed(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True)


def bounded(value, limit):
    if len(packed(value).encode()) > limit:
        raise Error(f"Serialized body exceeds {limit} UTF-8 bytes; split it or use an evidence pointer")
    return value


def ident(value):
    if not isinstance(value, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.:@-]{0,95}", value):
        raise Error("Expected an identifier of at most 96 ASCII characters")
    return value


def native_id(value):
    require(isinstance(value, str) and value.strip() and len(value.encode()) <= 256
            and not any(ord(char) < 32 for char in value), "Native runtime ID must be a bounded nonempty opaque identifier")
    return value


def require(condition, message):
    if not condition:
        raise Error(message)


def process(pid):
    """Identity includes boot/start ticks, so a reused PID is never the old process."""
    try:
        raw = Path(f"/proc/{int(pid)}/stat").read_text()
        fields = raw[raw.rfind(")") + 2:].split()
        if fields[0] == "Z":
            return None
        return {"kind": "process", "pid": int(pid), "start": fields[19],
                "pgid": int(fields[2]), "sid": int(fields[3]),
                "boot": Path("/proc/sys/kernel/random/boot_id").read_text().strip()}
    except (FileNotFoundError, ProcessLookupError):
        return None


def same_process(runtime):
    return process(runtime["pid"]) == runtime


def inactive(runtime, proof):
    kind = runtime["kind"]
    if kind == "unlaunched":
        return
    if kind == "native":
        require(proof.get("runtime_id") == runtime["id"] and proof.get("verified_inactive") is True
                and isinstance(proof.get("evidence"), str) and proof["evidence"],
                "Native runtime needs exact ID and explicit supervision evidence of inactivity")
        return
    require(not same_process(runtime), "Owned worker process is still active")
    # Children can outlive their original parent. A session must be isolated at launch.
    if runtime["boot"] == Path("/proc/sys/kernel/random/boot_id").read_text().strip():
        for entry in Path("/proc").iterdir():
            if entry.name.isdigit():
                live = process(entry.name)
                require(not live or live["sid"] != runtime["sid"], "Owned worker session still has live processes")


def tree_state(path):
    def git(*args):
        return subprocess.check_output(["git", "-C", path, *args], stderr=subprocess.PIPE)
    status = git("status", "--porcelain=v1", "--untracked-files=all")
    digest = hashlib.sha256(status)
    digest.update(git("diff", "--binary", "HEAD", "--"))
    for name in git("ls-files", "--others", "--exclude-standard", "-z").split(b"\0"):
        if not name:
            continue
        file = Path(path) / os.fsdecode(name)
        digest.update(name)
        if file.is_symlink():
            digest.update(os.fsencode(os.readlink(file)))
        elif file.is_file():
            with file.open("rb") as stream:
                for chunk in iter(lambda: stream.read(65536), b""):
                    digest.update(chunk)
    return {"head": git("rev-parse", "HEAD").decode().strip(),
            "changes_sha256": digest.hexdigest(), "clean": not status}


class Board:
    def __init__(self, path):
        self.path = Path(path).absolute()
        require(not self.path.is_symlink(), "Use the canonical database path")
        self.db = sqlite3.connect(self.path.as_uri() + "?mode=rw", uri=True, timeout=10)
        self.db.row_factory = sqlite3.Row

    def close(self):
        self.db.close()

    @staticmethod
    def init(path, run, lead):
        ident(run); ident(lead)
        fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        os.close(fd)
        with sqlite3.connect(path) as db:
            db.executescript('''
                CREATE TABLE state (id INTEGER PRIMARY KEY CHECK(id=1), body TEXT NOT NULL);
                CREATE TABLE statuses (task TEXT, attempt INTEGER, actor TEXT, version INTEGER,
                    body TEXT, submission TEXT, applied TEXT NOT NULL DEFAULT '{}',
                    PRIMARY KEY(task,attempt));
                CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, sender TEXT,
                    recipient TEXT, task TEXT, attempt INTEGER, kind TEXT, body TEXT,
                    retry_key TEXT, decision TEXT, decision_version INTEGER,
                    supersedes INTEGER, UNIQUE(sender,retry_key));
                CREATE INDEX inbox ON messages(recipient,id);
                CREATE TABLE receipts (actor TEXT PRIMARY KEY, cursor INTEGER NOT NULL DEFAULT 0,
                    delivery TEXT);
                CREATE TABLE effects (actor TEXT, message INTEGER, disposition TEXT,
                    evidence TEXT, PRIMARY KEY(actor,message));
                CREATE TABLE quiet (task TEXT, attempt INTEGER, proof TEXT,
                    PRIMARY KEY(task,attempt));
            ''')
            state = {"schema": 1, "run": run, "lead": lead, "version": 0,
                     "current": {"tasks": {}, "decisions": {}}, "holder": None,
                     "leader_runtime": None, "epoch": None, "reconciled": False}
            db.execute("INSERT INTO state VALUES (1,?)", (packed(state),))
        return {"run": run, "lead": lead}

    def state(self):
        state = json.loads(self.db.execute("SELECT body FROM state WHERE id=1").fetchone()[0])
        require(state["schema"] == 1, "Unsupported schema")
        return state

    def save(self, state):
        self.db.execute("UPDATE state SET body=? WHERE id=1", (packed(state),))

    def lock_active(self, state):
        require(state["holder"] and same_process(state["holder"]), "No live supervisor; recover before writes")
        with open(str(self.path) + ".lock", "a") as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                return
            raise Error("Supervisor lock is not held")

    def lead(self, req, state):
        require(req["actor"] == state["lead"] and req.get("epoch") == state["epoch"],
                "Lead operation requires active supervisor epoch and lead identity")

    def task(self, req, state, worker=True):
        task = state["current"]["tasks"].get(req.get("task"))
        require(task and task["attempt"] == req.get("attempt"), "Stale or unknown task attempt")
        if worker:
            require(task["worker"] == req["actor"], "This worker does not own the active attempt")
            require(not self.db.execute("SELECT 1 FROM quiet WHERE task=? AND attempt=?", (req["task"], req["attempt"])).fetchone(), "Attempt is quiescent; replace it before further worker activity")
        return task

    def status_row(self, req):
        row = self.db.execute("SELECT * FROM statuses WHERE task=? AND attempt=?",
                              (req["task"], req["attempt"])).fetchone()
        return dict(row) if row else None

    def ready(self, req, state):
        task = self.task(req, state)
        require(task["state"] in ("ready", "running"), "Attempt is not open for work")
        require(task["runtime"]["kind"] != "unlaunched", "Register the runtime before work")
        status = self.status_row(req)
        applied = json.loads(status["applied"]) if status else {}
        require(all(applied.get(k, {}).get("version") == v for k, v in task["interfaces"].items()),
                "Apply every current interface explicitly before dependent work")
        pending = self.db.execute('''SELECT m.id FROM messages m LEFT JOIN effects e
            ON e.actor=m.recipient AND e.message=m.id WHERE m.recipient=? AND m.task=?
            AND m.attempt=? AND m.kind IN ('decision','control','blocker')
            AND (e.disposition IS NULL OR e.disposition='deferred') LIMIT 1''',
            (req["actor"], req["task"], req["attempt"])).fetchone()
        require(not pending, "Resolve pending decision/control/blocker before dependent work")
        for dep in task["dependencies"]:
            require(state["current"]["tasks"].get(dep, {}).get("state") == "accepted",
                    "Dependency is not accepted")
        return task

    def fingerprint(self, state):
        digest = hashlib.sha256(packed(state).encode())
        for table in ("statuses", "messages", "receipts", "effects", "quiet"):
            for row in self.db.execute(f"SELECT * FROM {table} ORDER BY rowid"):
                digest.update(packed(list(row)).encode())
        return digest.hexdigest()

    def perform(self, req):
        require(isinstance(req, dict), "Request must be an object")
        bounded(req, 16384)
        ident(req.get("actor")); ident(req.get("run"))
        with self.db:
            self.db.execute("BEGIN IMMEDIATE")
            state = self.state()
            require(req["run"] == state["run"], "Wrong run")
            self.lock_active(state)
            op = req["op"]
            if req["actor"] == state["lead"] or op in ("snapshot", "replace", "inspect", "quiescence", "recovery", "reconcile"):
                self.lead(req, state)
            require(state["reconciled"] or op in ("recovery", "reconcile", "snapshot"),
                    "Startup gate closed: reconcile recovery before dispatch or writes")
            result = self.dispatch(op, req, state)
            return result

    def dispatch(self, op, req, state):
        if op == "snapshot":
            return {"version": state["version"], "current": state["current"], "reconciled": state["reconciled"]}
        if op == "recovery":
            return {"fingerprint": self.fingerprint(state), "run": state["run"],
                    "version": state["version"], "previous_leader": state.get("previous_leader"),
                    "tasks": {k: {"attempt": t["attempt"], "worker": t["worker"],
                        "runtime": t["runtime"], "tree": tree_state(t["worktree"]),
                        "submission": (self.status_row({"task": k, "attempt": t["attempt"]}) or {}).get("submission")}
                        for k, t in state["current"]["tasks"].items()},
                    "pending_count": self.db.execute('''SELECT COUNT(*) FROM messages m LEFT JOIN effects e
                        ON e.actor=m.recipient AND e.message=m.id
                        WHERE e.disposition IS NULL OR e.disposition='deferred' ''').fetchone()[0]}
        if op == "reconcile":
            require(req["fingerprint"] == self.fingerprint(state), "Recovery state changed; inspect again")
            old = state.get("previous_leader")
            if old:
                if old["kind"] == "native" and old == state["leader_runtime"]:
                    proof = req.get("previous_leader", {})
                    require(proof.get("runtime_id") == old["id"] and proof.get("same_lead_resume") is True and proof.get("evidence"), "Native lead resume needs explicit sole-owner supervision evidence")
                elif old != state["leader_runtime"]:
                    if old["kind"] == "process":
                        require(not same_process(old), "Previous lead runtime is still alive")
                    else:
                        inactive(old, req.get("previous_leader", {}))
            for key, task in state["current"]["tasks"].items():
                proof = req.get("tasks", {}).get(key, {})
                require(proof.get("tree") == tree_state(task["worktree"]), "Inspect current worktree head and changes")
                inactive(task["runtime"], proof)
                require(proof.get("evidence"), "Recovery requires inspection/preservation evidence")
                self.db.execute("INSERT OR REPLACE INTO quiet VALUES (?,?,?)",
                                (key, task["attempt"], packed(proof)))
            require(req.get("evidence"), "Recovery requires pending-action/submission reconciliation evidence")
            state["reconciled"] = True
            state["recovery_evidence"] = req["evidence"]
            self.save(state)
            return {"reconciled": True, "version": state["version"]}
        if op == "replace":
            require(req["expect"] == state["version"], "Snapshot version conflict; reread and reconcile")
            current = bounded(req["current"], 8192)
            self.validate_current(current, state)
            state["current"] = current
            state["version"] += 1
            self.save(state)
            return {"version": state["version"]}
        if op == "inspect":
            self.task(req, state, worker=False)
            row = self.status_row(req)
            return {"status": row}
        if op == "view":
            task = self.task(req, state)
            view = {"run": state["run"], "snapshot_version": state["version"], "task_id": req["task"],
                    "task": task, "decisions": {k: state["current"]["decisions"][k] for k in task["interfaces"]}}
            return bounded(view, 6144)
        if op == "check":
            self.ready(req, state)
            return {"valid": True, "version": state["version"]}
        if op in ("status", "apply", "submit"):
            task = self.task(req, state)
            require(task["state"] not in ("accepted", "revoked"), "Attempt is closed")
            row = self.status_row(req)
            if not row:
                self.db.execute("INSERT INTO statuses(task,attempt,actor,version) VALUES (?,?,?,0)",
                                (req["task"], req["attempt"], req["actor"]))
                row = self.status_row(req)
            if op == "status":
                if "body" not in req:
                    return {"version": row["version"], "body": json.loads(row["body"]) if row["body"] else None}
                require(req["expect"] == row["version"], "Status version conflict")
                bounded(req["body"], 2048)
                self.db.execute("UPDATE statuses SET version=version+1,body=? WHERE task=? AND attempt=?",
                                (packed(req["body"]), req["task"], req["attempt"]))
                return {"version": row["version"] + 1}
            if op == "apply":
                key, version = req["decision"], req["decision_version"]
                require(task["interfaces"].get(key) == version, "Decision is not the current applicable interface")
                require(isinstance(req.get("evidence"), str) and req["evidence"], "Record the concrete interface application evidence")
                bounded(req["evidence"], 512)
                applied = {k: v for k, v in json.loads(row["applied"]).items() if k in task["interfaces"]}
                applied[key] = {"version": version, "evidence": req["evidence"]}
                bounded(applied, 2048)
                self.db.execute("UPDATE statuses SET applied=? WHERE task=? AND attempt=?",
                                (packed(applied), req["task"], req["attempt"]))
                return {"applied": key, "version": version}
            self.ready(req, state)
            submission = bounded({"revision": req["revision"], "evidence": req["evidence"]}, 2048)
            tree = tree_state(task["worktree"])
            require(tree["head"] == req["revision"] and tree["clean"], "Submit current clean, stable worktree HEAD")
            if row["submission"]:
                require(json.loads(row["submission"]) == submission, "Submission is immutable; lead must start a new attempt")
            self.db.execute("UPDATE statuses SET submission=? WHERE task=? AND attempt=?",
                            (packed(submission), req["task"], req["attempt"]))
            return {"submitted": submission, "accepted": False}
        if op == "quiescence":
            task = self.task(req, state, worker=False)
            proof = bounded(req["proof"], 2048)
            require(proof.get("tree") == tree_state(task["worktree"]), "Inspect/preserve exact current worktree state")
            require(proof.get("evidence"), "Quiescence requires preservation evidence")
            inactive(task["runtime"], proof)
            self.db.execute("INSERT OR REPLACE INTO quiet VALUES (?,?,?)", (req["task"], req["attempt"], packed(proof)))
            return {"quiescent": True, "verification": task["runtime"]["kind"]}
        if op == "send":
            return self.send(req, state)
        if op in ("inbox", "pending", "message", "effect", "ack"):
            require(req["actor"] == state["lead"] or any(t["worker"] == req["actor"] for t in state["current"]["tasks"].values()), "Unknown active inbox owner")
            if "for_actor" in req:
                self.lead(req, state)
                ident(req["for_actor"])
                require(op in ("pending", "message") or (op == "effect" and req.get("disposition") == "superseded"), "Lead audit allows targeted reads or explicit supersession only")
                require(self.db.execute("SELECT 1 FROM statuses WHERE actor=?", (req["for_actor"],)).fetchone(), "Unknown historical worker")
            return self.mail(op, req, state)
        raise Error("Unknown operation")

    def validate_current(self, current, state):
        require(set(current) <= {"tasks", "decisions", "objective"} and isinstance(current["tasks"], dict)
                and isinstance(current["decisions"], dict), "Current contains tasks, decisions, optional objective")
        old_tasks = state["current"]["tasks"]
        for key, decision in current["decisions"].items():
            ident(key)
            require(type(decision["version"]) is int and decision["version"] > 0 and decision.get("body") and decision.get("evidence"), "Decision requires version/body/evidence")
            old = state["current"]["decisions"].get(key)
            if old and decision != old:
                require(decision["version"] == old["version"] + 1 and decision.get("supersedes") == old["version"], "Decision update must name immediately preceding version")
        workers, paths, resources, worktrees = set(), [], set(), set()
        for key, task in current["tasks"].items():
            ident(key); ident(task["worker"])
            require(task["worker"] != state["lead"], "Worker instance must differ from lead")
            required = {"attempt", "worker", "route", "effort", "base", "worktree", "paths", "resources", "dependencies", "interfaces", "checks", "runtime", "evidence", "state"}
            require(required <= set(task), "Assignment lacks required contract fields")
            require(type(task["attempt"]) is int and task["attempt"] > 0, "Attempt must be a positive integer")
            require(task["state"] in ("ready", "running", "submitted", "accepted", "blocked", "revoked"), "Invalid task state")
            require(Path(task["worktree"]).is_absolute() and Path(task["worktree"]).is_dir() and str(Path(task["worktree"]).resolve()) == task["worktree"], "Exact canonical absolute worktree required")
            require(all(task[k] for k in ("route", "effort", "base", "checks", "evidence")), "Contract fields must be nonempty")
            require(isinstance(task["paths"], list) and isinstance(task["resources"], list)
                    and isinstance(task["dependencies"], list) and isinstance(task["interfaces"], dict), "Invalid contract field types")
            for path in task["paths"]:
                require(path and not Path(path).is_absolute() and ".." not in Path(path).parts
                        and not any(c in path for c in "*?[]"), "Owned paths must be literal repository-relative paths")
            runtime = task["runtime"]
            require(runtime["kind"] in ("process", "native", "unlaunched"), "Unsupported runtime")
            if runtime["kind"] == "process":
                require(set(runtime) == {"kind", "pid", "start", "pgid", "sid", "boot"}, "Exact process identity required")
                require(runtime["pid"] == runtime["sid"] == runtime["pgid"], "Workers must launch in isolated sessions")
            elif runtime["kind"] == "native":
                native_id(runtime["id"])
            old = old_tasks.get(key)
            if old:
                if task["attempt"] != old["attempt"]:
                    require(task["attempt"] == old["attempt"] + 1 and task["worker"] != old["worker"], "Replacement increments attempt and uses a fresh worker instance")
                    self.require_quiet(key, old)
                    require(task["state"] == "ready", "Replacement starts ready")
                else:
                    require(all(task[k] == old[k] for k in ("worker", "worktree", "base", "paths", "resources")), "Ownership contract is immutable within an attempt")
                    require(task["runtime"] == old["runtime"] or old["runtime"]["kind"] == "unlaunched", "Runtime identity is immutable after registration")
                    transitions = {"ready": {"ready", "running", "blocked", "revoked"}, "running": {"running", "blocked", "submitted", "revoked"}, "blocked": {"blocked", "ready", "running", "submitted", "revoked"}, "submitted": {"submitted", "accepted", "revoked"}, "accepted": {"accepted"}, "revoked": {"revoked"}}
                    require(task["state"] in transitions[old["state"]], "Invalid lifecycle transition")
                if task["state"] in ("submitted", "accepted"):
                    row = self.status_row({"task": key, "attempt": task["attempt"]})
                    require(row and row["submission"], "Worker must submit before lead advances lifecycle")
                    applied = json.loads(row["applied"])
                    require(all(applied.get(k, {}).get("version") == v for k, v in task["interfaces"].items()), "Submission does not match current applied interfaces")
                    revision = json.loads(row["submission"])["revision"]
                    tree = tree_state(task["worktree"])
                    require(tree["head"] == revision and tree["clean"], "Submitted revision or worktree changed; review is invalid")
                    if task["state"] == "accepted":
                        require(task.get("acceptance", {}).get("revision") == revision and task["acceptance"].get("evidence"), "Acceptance requires checked revision and evidence")
            else:
                require(task["attempt"] == 1 and task["state"] == "ready", "New tasks start at attempt 1, ready")
            if task["state"] in ("accepted", "revoked"):
                self.require_quiet(key, task)
            if runtime["kind"] == "process" and (not old or task["attempt"] != old["attempt"] or old["runtime"]["kind"] == "unlaunched"):
                require(same_process(runtime), "Registered process identity must match a live worker")
            existing = self.db.execute("SELECT task,attempt FROM statuses WHERE actor=?", (task["worker"],)).fetchone()
            require(not existing or (existing["task"] == key and existing["attempt"] == task["attempt"]), "Worker instance was already used")
            self.db.execute("INSERT OR IGNORE INTO statuses(task,attempt,actor,version) VALUES (?,?,?,0)", (key, task["attempt"], task["worker"]))
            for decision, version in task["interfaces"].items():
                require(current["decisions"].get(decision, {}).get("version") == version, "Task must reference current decision version")
            bounded({"run": state["run"], "snapshot_version": state["version"] + 1, "task_id": key,
                     "task": task, "decisions": {k: current["decisions"][k] for k in task["interfaces"]}}, 6144)
            if task["state"] not in ("accepted", "revoked"):
                require(task["worker"] not in workers and task["worktree"] not in worktrees, "One live writer per instance/worktree")
                workers.add(task["worker"]); worktrees.add(task["worktree"])
                for path in task["paths"]:
                    normalized = str(Path(path))
                    require(not any(normalized == p or normalized.startswith(p + "/") or p.startswith(normalized + "/") or normalized == "." or p == "." for p in paths), "Owned paths overlap")
                    paths.append(normalized)
                require(not resources.intersection(task["resources"]), "Owned exclusive resources overlap")
                resources.update(task["resources"])
        def visit(key, visiting, visited):
            require(key in current["tasks"], "Dependency must name a current task")
            require(key not in visiting, "Task dependency cycle")
            if key in visited:
                return
            for dep in current["tasks"][key]["dependencies"]:
                visit(dep, visiting | {key}, visited)
            visited.add(key)
        visited = set()
        for key in current["tasks"]:
            visit(key, set(), visited)
        for key, old in old_tasks.items():
            if key not in current["tasks"]:
                require(old["state"] in ("accepted", "revoked"), "Archive only closed tasks")
                self.require_quiet(key, old)
        # A removed task ID cannot be silently reused as a new attempt.
        for key, task in current["tasks"].items():
            if key not in old_tasks:
                require(not self.db.execute("SELECT 1 FROM quiet WHERE task=?", (key,)).fetchone(), "Archived task ID cannot be reused")

    def require_quiet(self, key, task):
        row = self.db.execute("SELECT proof FROM quiet WHERE task=? AND attempt=?", (key, task["attempt"])).fetchone()
        require(row, "Verify previous worker and children quiescence before releasing ownership")
        proof = json.loads(row[0])
        require(proof["tree"] == tree_state(task["worktree"]), "Worktree changed after quiescence inspection")
        inactive(task["runtime"], proof)

    def send(self, req, state):
        if req["actor"] == state["lead"]:
            self.lead(req, state)
        ident(req["key"]); ident(req["to"])
        task = self.task(req, state, worker=req["actor"] != state["lead"])
        require(req["to"] in (state["lead"], task["worker"]), "Target the owning worker or lead")
        require(req["kind"] in ("decision", "blocker", "question", "update", "handoff", "control"), "Invalid message kind")
        content = bounded(req["body"], 1024)
        require(isinstance(content, dict) and content.get("action") and content.get("evidence"), "Message requires action and evidence pointer")
        decision = req.get("decision")
        version = req.get("decision_version")
        if req["kind"] == "decision":
            self.lead(req, state)
            require(task["interfaces"].get(decision) == version, "Decision message must name current applicable version")
        predecessor = req.get("supersedes")
        if predecessor:
            self.lead(req, state)
            old = self.db.execute("SELECT * FROM messages WHERE id=?", (predecessor,)).fetchone()
            require(old and old["recipient"] == req["to"] and old["task"] == req["task"] and old["attempt"] == req["attempt"], "Supersession must target same recipient/task/attempt")
        payload = (req["actor"], req["to"], req["task"], req["attempt"], req["kind"], packed(content), req["key"], decision, version, predecessor)
        old = self.db.execute("SELECT * FROM messages WHERE sender=? AND retry_key=?", (req["actor"], req["key"])).fetchone()
        if old:
            require(tuple(old)[1:] == payload, "Retry key already has different content")
            return {"id": old["id"], "duplicate": True}
        cursor = self.db.execute("INSERT INTO messages(sender,recipient,task,attempt,kind,body,retry_key,decision,decision_version,supersedes) VALUES (?,?,?,?,?,?,?,?,?,?)", payload)
        return {"id": cursor.lastrowid, "duplicate": False}

    def mail(self, op, req, state):
        actor = req.get("for_actor", req["actor"])
        self.db.execute("INSERT OR IGNORE INTO receipts(actor) VALUES (?)", (actor,))
        receipt = self.db.execute("SELECT * FROM receipts WHERE actor=?", (actor,)).fetchone()
        if op in ("inbox", "pending"):
            if op == "inbox" and receipt["delivery"]:
                return json.loads(receipt["delivery"])
            after = receipt["cursor"] if op == "inbox" else req.get("after", 0)
            rows = self.db.execute('''SELECT m.*,e.disposition,e.evidence AS effect_evidence
                FROM messages m LEFT JOIN effects e ON e.actor=m.recipient AND e.message=m.id
                WHERE m.recipient=? AND m.id>? ''' + ("AND (e.disposition IS NULL OR e.disposition='deferred') " if op == "pending" else "") + "ORDER BY m.id LIMIT 5", (actor, after)).fetchall()
            result = {"messages": [], "through": after, "has_more": False, "run": state["run"]}
            for row in rows:
                message = dict(row); message["body"] = json.loads(message["body"])
                candidate = {**result, "messages": result["messages"] + [message], "through": row["id"], "has_more": True}
                if len(result["messages"]) == 4 or len(packed(candidate).encode()) > 8192:
                    break
                result = candidate
            result["has_more"] = len(rows) > len(result["messages"])
            require(not rows or result["messages"], "One message cannot fit output bound")
            if op == "inbox" and result["messages"]:
                self.db.execute("UPDATE receipts SET delivery=? WHERE actor=?", (packed(result), actor))
            return bounded(result, 8192)
        if op == "ack":
            if req["through"] == receipt["cursor"]:
                return {"through": receipt["cursor"], "duplicate": True}
            require(receipt["delivery"], "No delivered batch to acknowledge")
            delivery = json.loads(receipt["delivery"])
            require(req["through"] == delivery["through"], "Acknowledge only the exact delivered batch boundary")
            for message in delivery["messages"]:
                require(self.db.execute("SELECT 1 FROM effects WHERE actor=? AND message=?", (actor, message["id"])).fetchone(), "Durably record every consequence or deferred action before ack")
            self.db.execute("UPDATE receipts SET cursor=?,delivery=NULL WHERE actor=?", (req["through"], actor))
            return {"through": req["through"]}
        row = self.db.execute("SELECT * FROM messages WHERE id=? AND recipient=?", (req["id"], actor)).fetchone()
        require(row, "Message is not in this actor's inbox")
        if op == "message":
            result = dict(row); result["body"] = json.loads(result["body"])
            effect = self.db.execute("SELECT disposition,evidence FROM effects WHERE actor=? AND message=?", (actor, req["id"])).fetchone()
            result["effect"] = dict(effect) if effect else None
            return bounded(result, 8192)
        disposition = req["disposition"]
        require(disposition in ("done", "deferred", "superseded"), "Invalid effect disposition")
        bounded(req["evidence"], 1024); require(isinstance(req["evidence"], str) and req["evidence"], "Effect/deferred action requires a durable evidence pointer string")
        if disposition == "done" and row["kind"] == "decision":
            task_req = {"actor": actor, "task": row["task"], "attempt": row["attempt"]}
            task = self.task(task_req, state)
            status = self.status_row(task_req)
            require(task["interfaces"].get(row["decision"]) == row["decision_version"] and status
                    and json.loads(status["applied"]).get(row["decision"], {}).get("version") == row["decision_version"], "Receipt is not application; apply current interface first")
        if disposition == "superseded":
            replacement = self.db.execute("SELECT id FROM messages WHERE supersedes=?", (row["id"],)).fetchone()
            task = state["current"]["tasks"].get(row["task"])
            require(replacement or not task or task["attempt"] != row["attempt"], "Explicit superseding message or obsolete attempt required")
        old = self.db.execute("SELECT * FROM effects WHERE actor=? AND message=?", (actor, req["id"])).fetchone()
        if old and old["disposition"] != "deferred":
            require(old["disposition"] == disposition and old["evidence"] == req["evidence"], "Resolved effect is immutable; inspect before replay")
            return {"recorded": req["id"], "duplicate": True}
        self.db.execute("INSERT OR REPLACE INTO effects VALUES (?,?,?,?)", (actor, req["id"], disposition, req["evidence"]))
        return {"recorded": req["id"]}


def hold(path, actor, run, leader_pid=None, leader_id=None):
    """Supervisor owns a real kernel lock until exit; it never launches/stops workers."""
    with open(str(Path(path).absolute()) + ".lock", "a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        board = Board(path)
        try:
            with board.db:
                board.db.execute("BEGIN IMMEDIATE")
                state = board.state()
                require(state["lead"] == actor and state["run"] == run, "Wrong run/lead")
                leader = {"kind": "native", "id": native_id(leader_id)} if leader_id else process(leader_pid)
                require(leader, "Lead runtime PID must be alive")
                state["previous_leader"] = state["leader_runtime"]
                state.update(holder=process(os.getpid()), leader_runtime=leader,
                             epoch=uuid.uuid4().hex, reconciled=False)
                board.save(state)
            print(packed({"epoch": state["epoch"], "holder": state["holder"], "recovery_required": True}), flush=True)
            while leader["kind"] == "native" or same_process(leader):
                time.sleep(0.5)
        finally:
            board.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", required=True)
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("init", "hold"):
        item = sub.add_parser(name)
        item.add_argument("--run", required=True); item.add_argument("--actor", required=True)
        if name == "hold":
            leader = item.add_mutually_exclusive_group(required=True)
            leader.add_argument("--leader-pid", type=int)
            leader.add_argument("--leader-id", help="Native lead task ID; uses explicit supervision attestations")
    proc = sub.add_parser("process"); proc.add_argument("--pid", type=int, required=True)
    for name in ("snapshot", "replace", "inspect", "view", "check", "status", "apply", "submit", "quiescence", "send", "inbox", "pending", "message", "effect", "ack", "recovery", "reconcile"):
        item = sub.add_parser(name)
        item.add_argument("--file", default="-", help="JSON request fields (actor/run plus operation fields); - reads stdin")
    args = parser.parse_args()
    try:
        if args.command == "init":
            result = Board.init(args.db, args.run, args.actor)
        elif args.command == "hold":
            hold(args.db, args.actor, args.run, args.leader_pid, args.leader_id)
            return 0
        elif args.command == "process":
            result = process(args.pid)
            require(result, "Process is not alive")
        else:
            with contextlib.ExitStack() as stack:
                stream = sys.stdin.buffer if args.file == "-" else stack.enter_context(open(args.file, "rb"))
                raw = stream.read(16385)
                require(len(raw) <= 16384, "Request exceeds 16 KiB")
                request = json.loads(raw)
            request["op"] = args.command
            board = Board(args.db)
            try:
                result = board.perform(request)
            finally:
                board.close()
        print(packed(result))
        return 0
    except (Error, OSError, sqlite3.Error, KeyError, TypeError, ValueError, subprocess.CalledProcessError) as exc:
        print(packed({"error": str(exc)}), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
