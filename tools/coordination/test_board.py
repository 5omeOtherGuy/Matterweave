"""Protocol rehearsals; no models, native builds or network calls."""
import concurrent.futures
import copy
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import unittest

from board import Board, Error, packed, process, tree_state


class BoardTests(unittest.TestCase):
    def setUp(self):
        root = Path(os.environ.get("MATTERWEAVE_COORD_TEST_ROOT", "/mnt/bench/matterweave-dev/v0.3/orchestration/board-bootstrap"))
        root.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=root)
        self.root = Path(self.temp.name)
        self.path = self.root / "board.sqlite"
        self.tree = self.root / "tree"
        self.tree.mkdir()
        subprocess.run(["git", "init", "-q", str(self.tree)], check=True)
        subprocess.run(["git", "-C", str(self.tree), "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "--allow-empty", "-qm", "fixture"], check=True)
        self.head = tree_state(str(self.tree))["head"]
        Board.init(self.path, "fixture", "lead")
        self.holders = []
        self.start_holder()
        self.reconcile()

    def tearDown(self):
        for holder in self.holders:
            if holder.poll() is None:
                holder.terminate()
            holder.communicate(timeout=5)
        self.temp.cleanup()

    def start_holder(self, leader_pid=None, leader_id=None):
        holder = subprocess.Popen([sys.executable, str(Path(__file__).with_name("board.py")), "--db", str(self.path), "hold", "--run", "fixture", "--actor", "lead", *( ["--leader-id", leader_id] if leader_id else ["--leader-pid", str(leader_pid or os.getpid())])], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.holders.append(holder)
        line = holder.stdout.readline()
        self.assertTrue(line, holder.stderr.read() if holder.poll() is not None else "holder did not start")
        self.epoch = json.loads(line)["epoch"]
        return holder

    def call(self, op, actor="lead", **fields):
        req = {"op": op, "actor": actor, "run": "fixture", **fields}
        if actor == "lead":
            req["epoch"] = self.epoch
        board = Board(self.path)
        try:
            return board.perform(req)
        finally:
            board.close()

    def worker(self, op, **fields):
        return self.call(op, actor="worker-1", task="terrain", attempt=1, **fields)

    def reconcile(self, tasks=None):
        report = self.call("recovery")
        return self.call("reconcile", fingerprint=report["fingerprint"], tasks=tasks or {}, evidence="fixture: inspected pending actions, submissions and runtimes")

    def contract(self):
        return {"attempt": 1, "worker": "worker-1", "route": "fixture", "effort": "none", "base": self.head,
                "worktree": str(self.tree), "paths": ["src/terrain"], "resources": [], "dependencies": [],
                "interfaces": {}, "checks": ["fixture"], "runtime": {"kind": "native", "id": "native-1"},
                "evidence": "fixture/log", "state": "ready"}

    def assign(self, interfaces=False):
        current = {"tasks": {"terrain": self.contract()}, "decisions": {}}
        if interfaces:
            current["decisions"]["jobs"] = {"version": 1, "body": "immutable results", "evidence": "docs/interfaces"}
            current["tasks"]["terrain"]["interfaces"]["jobs"] = 1
        self.call("replace", expect=0, current=current)
        return current

    def message(self, index, **fields):
        return self.call("send", task="terrain", attempt=1, to="worker-1", kind="question",
                         key=f"key-{index}", body={"action": f"inspect {index}", "evidence": "fixture/log"}, **fields)["id"]

    def proof(self):
        return {"runtime_id": "native-1", "verified_inactive": True,
                "evidence": "fixture/native supervisor confirms stopped and preserved", "tree": tree_state(str(self.tree))}

    def test_concurrent_cas_one_winner(self):
        def update(index):
            try:
                return self.call("replace", expect=0, current={"tasks": {}, "decisions": {}, "objective": str(index)})
            except Error:
                return "conflict"
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            results = list(pool.map(update, range(2)))
        self.assertEqual(results.count("conflict"), 1)
        self.assertEqual(self.call("snapshot")["version"], 1)

    def test_worker_scope_caps_and_status_replacement(self):
        current = self.assign()
        with self.assertRaises(Error):
            self.worker("snapshot")
        view = self.worker("view")
        self.assertEqual(view["task"]["worker"], "worker-1")
        self.assertNotIn("tasks", view)
        for version in range(20):
            self.worker("status", expect=version, body={"progress": version})
        self.assertEqual(self.worker("status")["body"], {"progress": 19})
        self.assertEqual(self.worker("inbox")["messages"], [])
        with self.assertRaises(Error):
            self.worker("status", expect=20, body="🦀" * 513)
        current["objective"] = "x" * 8192
        with self.assertRaises(Error):
            self.call("replace", expect=1, current=current)
        current.pop("objective")
        current["tasks"]["terrain"]["checks"] = ["x" * 6144]
        with self.assertRaises(Error):
            self.call("replace", expect=1, current=current)
        current["tasks"]["terrain"].update(checks=["fixture"], state="blocked")
        self.call("replace", expect=1, current=current)
        with self.assertRaises(Error):
            self.worker("check")

    def test_inbox_bounds_pagination_no_loss_and_ack_skipping(self):
        self.assign()
        ids = [self.message(i) for i in range(10)]
        batch = self.worker("inbox")
        self.assertEqual(len(batch["messages"]), 4)
        self.assertLessEqual(len(packed(batch).encode()), 8192)
        with self.assertRaises(Error):
            self.worker("ack", through=ids[-1])
        with self.assertRaises(Error):
            self.worker("ack", through=batch["through"])
        seen = []
        while batch["messages"]:
            for message in batch["messages"]:
                seen.append(message["id"])
                self.worker("effect", id=message["id"], disposition="done", evidence="fixture:checked")
            self.worker("ack", through=batch["through"])
            batch = self.worker("inbox")
        self.assertEqual(seen, ids)
        with self.assertRaises(Error):
            self.call("send", task="terrain", attempt=1, to="worker-1", kind="question", key="large", body={"action": "x" * 1024, "evidence": "fixture"})

    def test_retry_and_crash_after_durable_effect_before_ack(self):
        self.assign()
        message = self.message(1)
        self.assertEqual(self.message(1), message)
        self.worker("inbox")
        self.worker("effect", id=message, disposition="done", evidence="fixture/actual-side-effect-verified")
        # New connection/process, same persisted delivery and consequence after crash.
        request = {"actor": "worker-1", "run": "fixture"}
        output = subprocess.check_output([sys.executable, str(Path(__file__).with_name("board.py")), "--db", str(self.path), "inbox"], input=packed(request), text=True)
        self.assertEqual(json.loads(output)["through"], message)
        self.assertEqual(self.worker("message", id=message)["effect"]["disposition"], "done")
        self.assertTrue(self.worker("effect", id=message, disposition="done", evidence="fixture/actual-side-effect-verified")["duplicate"])
        self.worker("ack", through=message)
        self.assertTrue(self.worker("ack", through=message)["duplicate"])
        with self.assertRaises(Error):
            self.call("send", task="terrain", attempt=1, to="worker-1", kind="question", key="key-1", body={"action": "changed", "evidence": "fixture/log"})

    def test_receipt_not_application_pending_survives_ack(self):
        current = self.assign(interfaces=True)
        with self.assertRaises(Error):
            self.worker("check")
        message = self.call("send", task="terrain", attempt=1, to="worker-1", kind="decision", key="jobs-1", decision="jobs", decision_version=1, body={"action": "apply jobs v1", "evidence": "docs/interfaces"})["id"]
        self.worker("inbox")
        with self.assertRaises(Error):
            self.worker("effect", id=message, disposition="done", evidence="fixture")
        self.worker("effect", id=message, disposition="deferred", evidence="fixture/todo")
        self.worker("ack", through=message)
        self.assertEqual(self.worker("pending")["messages"][0]["id"], message)
        self.worker("apply", decision="jobs", decision_version=1, evidence="fixture/implementation")
        applied = json.loads(self.call("inspect", task="terrain", attempt=1)["status"]["applied"])
        self.assertEqual(applied["jobs"]["evidence"], "fixture/implementation")
        with self.assertRaises(Error):
            self.worker("check")
        self.worker("effect", id=message, disposition="done", evidence="fixture/implementation")
        self.assertTrue(self.worker("check")["valid"])
        current["decisions"]["jobs"] = {"version": 2, "supersedes": 1, "body": "revised results", "evidence": "docs/v2"}
        current["tasks"]["terrain"]["interfaces"]["jobs"] = 2
        self.call("replace", expect=1, current=current)
        with self.assertRaises(Error):
            self.worker("check")
        with self.assertRaises(Error):
            self.worker("apply", decision="jobs", decision_version=1, evidence="old")

    def test_supersession_is_explicit_and_new_action_stays_pending(self):
        self.assign()
        first = self.message(1)
        with self.assertRaises(Error):
            self.worker("effect", id=first, disposition="superseded", evidence="just old")
        second = self.message(2, supersedes=first)
        self.worker("effect", id=first, disposition="superseded", evidence=f"message:{second}")
        self.assertEqual([m["id"] for m in self.worker("pending")["messages"]], [second])

    def test_replacement_requires_quiescence_and_rejects_late_attempt(self):
        current = self.assign()
        old_message = self.message(1)
        updated = copy.deepcopy(current)
        updated["tasks"]["terrain"].update(attempt=2, worker="worker-2", runtime={"kind": "native", "id": "native-2"})
        with self.assertRaises(Error):
            self.call("replace", expect=1, current=updated)
        proof = self.proof()
        with self.assertRaises(Error):
            self.call("quiescence", task="terrain", attempt=1, proof={**proof, "verified_inactive": False})
        self.call("quiescence", task="terrain", attempt=1, proof=proof)
        with self.assertRaises(Error):
            self.worker("submit", revision=self.head, evidence="late")
        self.call("replace", expect=1, current=updated)
        with self.assertRaises(Error):
            self.worker("submit", revision=self.head, evidence="late")
        with self.assertRaises(Error):
            self.worker("status", expect=0, body="late")
        self.assertTrue(self.call("check", actor="worker-2", task="terrain", attempt=2)["valid"])
        self.assertEqual(self.call("pending", for_actor="worker-1")["messages"][0]["id"], old_message)
        self.call("effect", for_actor="worker-1", id=old_message, disposition="superseded", evidence="replacement attempt 2")
        self.assertEqual(self.call("pending", for_actor="worker-1")["messages"], [])

    def test_quiescence_inspection_invalidated_by_tree_change(self):
        current = self.assign()
        self.call("quiescence", task="terrain", attempt=1, proof=self.proof())
        (self.tree / "unpreserved.txt").write_text("change")
        current["tasks"]["terrain"].update(attempt=2, worker="worker-2", runtime={"kind": "native", "id": "native-2"})
        with self.assertRaises(Error):
            self.call("replace", expect=1, current=current)

    def test_single_lead_lock_restart_gate_and_recovery_fingerprint(self):
        self.assign()
        second = subprocess.run([sys.executable, str(Path(__file__).with_name("board.py")), "--db", str(self.path), "hold", "--run", "fixture", "--actor", "lead", "--leader-pid", str(os.getpid())], capture_output=True, text=True)
        self.assertEqual(second.returncode, 2)
        old_epoch = self.epoch
        self.holders[-1].terminate(); self.holders[-1].communicate(timeout=5)
        with self.assertRaises(Error):
            self.worker("status", expect=0, body="no supervisor")
        self.start_holder()
        with self.assertRaises(Error):
            self.worker("check")
        with self.assertRaises(Error):
            board = Board(self.path)
            try:
                board.perform({"op": "inbox", "actor": "lead", "run": "fixture", "epoch": old_epoch})
            finally:
                board.close()
        report = self.call("recovery")
        with self.assertRaises(Error):
            self.call("reconcile", fingerprint="wrong", tasks={"terrain": self.proof()}, evidence="fixture")
        self.call("reconcile", fingerprint=report["fingerprint"], tasks={"terrain": self.proof()}, evidence="fixture/recovery-inspection")
        self.assertTrue(self.call("snapshot")["reconciled"])
        with self.assertRaises(Error):
            self.worker("check")  # Recovery does not resurrect an attested stopped worker.

    def test_process_children_must_stop_and_no_signals_sent_by_board(self):
        child_code = "import subprocess,sys,time; p=subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)']); print(p.pid,flush=True); time.sleep(30)"
        owned = subprocess.Popen([sys.executable, "-c", child_code], start_new_session=True, stdout=subprocess.PIPE, text=True)
        try:
            child_pid = int(owned.stdout.readline())
            current = self.assign()
            # Register a fresh process attempt after proving native predecessor quiet.
            self.call("quiescence", task="terrain", attempt=1, proof=self.proof())
            runtime = process(owned.pid)
            current["tasks"]["terrain"].update(attempt=2, worker="worker-2", runtime=runtime)
            self.call("replace", expect=1, current=current)
            proof = {"tree": tree_state(str(self.tree)), "evidence": "fixture/inspection"}
            with self.assertRaises(Error):
                self.call("quiescence", task="terrain", attempt=2, proof=proof)
            self.assertIsNone(owned.poll())
            owned.terminate(); owned.wait(timeout=5)
            with self.assertRaises(Error):
                self.call("quiescence", task="terrain", attempt=2, proof=proof)
            self.assertIsNotNone(process(child_pid))
            os.killpg(runtime["pgid"], signal.SIGKILL)
            # Wait for OS to reap/mark the child zombie, with a strict short timeout.
            import time
            until = time.monotonic() + 3
            while process(child_pid) and time.monotonic() < until:
                time.sleep(0.01)
            self.assertTrue(self.call("quiescence", task="terrain", attempt=2, proof=proof)["quiescent"])
        finally:
            try:
                os.killpg(owned.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            owned.wait(timeout=5)
            owned.stdout.close()

    def test_submission_and_acceptance_stable_revision(self):
        current = self.assign()
        current["tasks"]["terrain"]["state"] = "running"
        self.call("replace", expect=1, current=current)
        first = self.worker("submit", revision=self.head, evidence="fixture/checks")
        self.assertEqual(self.worker("submit", revision=self.head, evidence="fixture/checks"), first)
        current["tasks"]["terrain"]["state"] = "submitted"
        self.call("replace", expect=2, current=current)
        self.call("quiescence", task="terrain", attempt=1, proof=self.proof())
        current["tasks"]["terrain"].update(state="accepted", acceptance={"revision": self.head, "evidence": "fixture/lead-review"})
        self.call("replace", expect=3, current=current)
        self.assertEqual(self.call("snapshot")["current"]["tasks"]["terrain"]["state"], "accepted")

    def test_native_opaque_ids_and_explicit_same_lead_resume(self):
        # Native task names are opaque canonical paths, not invented safe IDs.
        self.path = self.root / "native.sqlite"
        Board.init(self.path, "fixture", "lead")
        self.start_holder(leader_id="/root")
        self.reconcile()
        current = {"tasks": {"terrain": self.contract()}, "decisions": {}}
        current["tasks"]["terrain"]["runtime"]["id"] = "/root/terrain"
        self.call("replace", expect=0, current=current)
        self.assertEqual(self.worker("view")["task"]["runtime"]["id"], "/root/terrain")
        self.holders[-1].terminate(); self.holders[-1].communicate(timeout=5)
        self.start_holder(leader_id="/root")
        report = self.call("recovery")
        proof = {**self.proof(), "runtime_id": "/root/terrain"}
        with self.assertRaises(Error):
            self.call("reconcile", fingerprint=report["fingerprint"], tasks={"terrain": proof}, evidence="fixture")
        self.call("reconcile", fingerprint=report["fingerprint"], tasks={"terrain": proof},
                  previous_leader={"runtime_id": "/root", "same_lead_resume": True, "evidence": "native supervisor confirms sole root"}, evidence="fixture/recovered")

    def test_recovery_rejects_live_previous_leader(self):
        decoy = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])
        try:
            self.holders[-1].terminate(); self.holders[-1].communicate(timeout=5)
            self.start_holder(leader_pid=decoy.pid)
            with self.assertRaisesRegex(Error, "Previous lead runtime"):
                self.reconcile()
        finally:
            decoy.terminate(); decoy.wait(timeout=5)

    def test_pending_serialized_byte_bound_and_pagination(self):
        self.assign()
        ids = []
        for i in range(5):
            ids.append(self.call("send", task="terrain", attempt=1, to="worker-1", kind="control", key=f"large-{i}", body={"action": "x" * 950, "evidence": "fixture"})["id"])
            self.worker("effect", id=ids[-1], disposition="deferred", evidence="y" * 950)
        first = self.worker("pending")
        self.assertLess(len(first["messages"]), 4)
        self.assertLessEqual(len(packed(first).encode()), 8192)
        second = self.worker("pending", after=first["through"])
        self.assertEqual([m["id"] for m in first["messages"] + second["messages"]], ids)

    def test_submission_rejects_dirty_tree_and_dependency_cycles(self):
        current = self.assign()
        (self.tree / "uncommitted.txt").write_text("not submitted")
        with self.assertRaises(Error):
            self.worker("submit", revision=self.head, evidence="fixture")
        current["tasks"]["terrain"]["dependencies"] = ["terrain"]
        with self.assertRaises(Error):
            self.call("replace", expect=1, current=current)

    def test_cannot_release_live_worker_by_revoking_or_renaming_task(self):
        current = self.assign()
        current["tasks"]["terrain"]["state"] = "revoked"
        with self.assertRaises(Error):
            self.call("replace", expect=1, current=current)
        current["tasks"] = {"replacement": self.contract()}
        with self.assertRaises(Error):
            self.call("replace", expect=1, current=current)


if __name__ == "__main__":
    unittest.main()
