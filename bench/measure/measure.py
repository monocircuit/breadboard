#!/usr/bin/env python3
"""Where does the wall-clock time of breadboard's headless task agents go?

Read-only (#451, README.md next to it). Inputs:
  - Claude Code transcripts ~/.claude/projects/<main>/<session>.jsonl of every
    session in <main>/.claude/runs/status.json (the run engine's agents)
  - the orchestrator transcript (runs tell commands) for engine queue latency
    (--orchestrator <session>.jsonl, else none)
  - <out>/data/events_*.json (GitLab push events), <out>/data/mrs_*.json (MRs):
    fetch_gitlab.sh <out>/data
  - git (committer dates of pushed commits)
  - the windows: the periods compared and the holds (--windows <file>.json,
    default windows-2026-10-19.json)
Outputs (in --out, default $TMPDIR/breadboard-measure): categories.csv, runs.csv,
checks.csv, issues.csv, queue_latency.csv, summary.txt

  python3 measure.py [--main <checkout>] [--orchestrator <file>] [--windows <file>] [--out <dir>]

Definitions
  run      : starts at a prompt (user record with text content) that arrives while
             the agent is not in a turn; ends at the assistant record with
             stop_reason=end_turn after which no tool is pending. Prompts within a
             run (task notifications, queued lines) continue it.
  cut run  : a run that ends without end_turn, or holds a tool result "[Tool call
             interrupted" (crash, OOM kill, herdr closed, reboot). Reported apart.
  in-run attribution (sweep over the run's records in time order): each interval
             between two consecutive records goes to
               - the highest-priority category among tools pending at its start, or
               - "model" (coding: the model reading tool output and generating
                 thinking + text + the next tool call) when nothing is pending,
               - "api retry" when an api_error record opened the interval.
             A gap inside a run after end_turn while a background task is pending
             (the process waits for its notification) goes to that task's category.
  between runs: per session, from its first run start in the window to its last run
             end (or now): split into held (engine paused, known crash/reboot
             downtime) and idle; "unpushed while idle" is the part of idle time
             where the branch had commits not yet on GitLab.
"""
import argparse
import csv
import glob
import json
import os
import re
import statistics
import subprocess
import sys
from collections import defaultdict
from datetime import datetime, timezone

HERE = os.path.dirname(os.path.abspath(__file__))


def ts(s):
    return datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()


def iso(t):
    return datetime.fromtimestamp(t, timezone.utc).strftime("%Y-%m-%d %H:%MZ")


def main_checkout():
    """The main checkout of the clone this script is in: the parent of git's common dir."""
    common = subprocess.run(["git", "-C", HERE, "rev-parse", "--path-format=absolute", "--git-common-dir"],
                            capture_output=True, text=True).stdout.strip()
    return os.path.dirname(common)


ARGS = argparse.ArgumentParser(description="Where the wall-clock time of the headless task agents goes.")
ARGS.add_argument("--main", default=None, help="the main checkout (default: this clone's)")
ARGS.add_argument("--orchestrator", default=None, help="the orchestrator's transcript, for queue latency")
ARGS.add_argument("--windows", default=os.path.join(HERE, "windows-2026-10-19.json"),
                  help="periods and holds (JSON)")
ARGS.add_argument("--out", default=os.path.join(os.environ.get("TMPDIR", "/tmp"), "breadboard-measure"),
                  help="where the tables go; its data/ holds fetch_gitlab.sh's files")
OPTS = ARGS.parse_args()
MAIN = os.path.abspath(OPTS.main or main_checkout())
# Claude Code's project directory of a path: every / and . a -.
PROJ = os.path.expanduser("~/.claude/projects/" + re.sub(r"[/.]", "-", MAIN))
ORCH = OPTS.orchestrator or ""
OUT = os.path.abspath(OPTS.out)
os.makedirs(OUT, exist_ok=True)

# The periods compared and the known holds (UTC), from the windows file: a period
# without an end runs until now.
WINDOWS = json.load(open(OPTS.windows))
NOW = datetime.now(timezone.utc).timestamp()
PERIODS = {name: (ts(p["from"]), ts(p["to"]) if p.get("to") else NOW) for name, p in WINDOWS["periods"].items()}
# A run still in its turn at the end of the last period counts as live, not cut.
NOW = min(NOW, max(hi for _, hi in PERIODS.values()))
HELD = [(h["what"], ts(h["from"]), ts(h["to"])) for h in WINDOWS.get("held", [])]
# GitLab as it was when the table's data/ was fetched: later pushes and merges left out.
FETCHED = ts(WINDOWS["gitlab_fetched"]) if WINDOWS.get("gitlab_fetched") else NOW

CATS = ["model", "checks & builds", "waiting on others", "git", "gitlab", "file tools",
        "subagents", "coordination", "other shell", "api retry"]
PRIO = {c: i for i, c in enumerate(["waiting on others", "subagents", "checks & builds", "gitlab", "git",
                                     "other shell", "coordination", "file tools", "api retry", "model"])}

RE_WAIT = re.compile(r"wt:wait|\bsleep\s+\d|\buntil\b|while\s.*;\s*do|--live|inotifywait|wait_for")
RE_CHECK = re.compile(r"wt:check|\bcargo\b|gauntlet|\bpnpm\b[^|;&]*\b(test|lint|typecheck|build|check)\b|eslint|\btsc\b|vitest|turbo\s+run|build:rs|node --test|dev:ready|\bbb\s+submit\b")
RE_GITLAB = re.compile(r"\bglab\b|curl[^|]*gitlab|wt:mr\b|wt:tick\b")
RE_GIT = re.compile(r"\bgit\s+(-C\s+\S+\s+)?(push|commit|merge|pull|fetch|rebase|cherry-pick|stash|checkout|switch|add|reset|restore|rm|mv)\b|wt:merge-base|wt:snapshot")
RE_FILE = re.compile(r"^\s*(cd [^;&]+[;&]+\s*)?(python3?\s+-\s*<<|sed\b|cat\b|grep\b|rg\b|head\b|tail\b|ls\b|find\b|wc\b|awk\b|diff\b|echo\b|nl\b|tree\b|jq\b|git\s+(-C\s+\S+\s+)?(log|show|diff|status|blame|grep|rev-parse|branch|ls-files)\b|perl\b|tee\b|printf\b|python3?\s+<<)")


RE_LOOP = re.compile(r"\bsleep\s+\d|\buntil\b|\bwhile\b[^\n]*\bdo\b|\bseq\b|--live|inotifywait|tail -f /dev/null")
RE_CHECKLOG = re.compile(r"check[\w.-]*\.log|wt-check|pgrep[^|]*check")


def classify_bash(cmd):
    """Dominant time of a shell call. Polling loops are classed by what they poll."""
    if "wt:wait" in cmd:
        return "waiting on others"
    loop = RE_LOOP.search(cmd)
    if loop and re.search(r"glab|pipeline|merge_requests|mr view|curl", cmd) and not RE_CHECKLOG.search(cmd):
        return "waiting on others"
    if RE_CHECK.search(cmd) or RE_CHECKLOG.search(cmd):
        return "checks & builds"
    if loop:
        return "waiting on others"
    if RE_GITLAB.search(cmd):
        return "gitlab"
    if RE_GIT.search(cmd):
        return "git"
    if RE_FILE.search(cmd):
        return "file tools"
    return "other shell"


def classify_tool(name, inp):
    if name == "Bash":
        return classify_bash(inp.get("command", ""))
    if name in ("Read", "Edit", "Write", "Grep", "Glob", "NotebookEdit", "MultiEdit"):
        return "file tools"
    if name in ("Agent", "Task"):
        # test-runner runs wt:check or the gauntlet for the agent (#373): a check.
        return "checks & builds" if inp.get("subagent_type") == "test-runner" else "subagents"
    if name in ("Monitor", "TaskOutput"):
        return "waiting on others"
    if name in ("SendMessage", "ListAgents", "ToolSearch", "Skill", "TaskStop", "TodoWrite", "AskUserQuestion"):
        return "coordination"
    return "other shell"


def text_of(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(x.get("text", "") for x in content if isinstance(x, dict))
    return ""


def load_session(path):
    recs = []
    for line in open(path):
        try:
            d = json.loads(line)
        except Exception:
            continue
        if d.get("isSidechain"):
            continue
        t = d.get("timestamp")
        if not t:
            continue
        typ = d.get("type")
        if typ == "user":
            c = d["message"]["content"]
            if isinstance(c, list) and c and c[0].get("type") == "tool_result":
                for b in c:
                    if b.get("type") == "tool_result":
                        recs.append(("result", ts(t), b["tool_use_id"], text_of(b.get("content"))))
            else:
                recs.append(("prompt", ts(t), d.get("promptSource") or "", text_of(c)))
        elif typ == "assistant":
            m = d["message"]
            if m.get("model") == "<synthetic>":
                recs.append(("assistant", ts(t), None, None))
                continue
            uses = [b for b in m.get("content", []) if b.get("type") == "tool_use"]
            recs.append(("assistant", ts(t), m.get("stop_reason"), uses))
        elif typ == "system" and d.get("subtype") == "api_error":
            recs.append(("api_error", ts(t), None, None))
    recs.sort(key=lambda r: r[1])
    return recs


def clip(a, b, lo, hi):
    return max(0.0, min(b, hi) - max(a, lo))


def period_of(t):
    for p, (lo, hi) in PERIODS.items():
        if lo <= t < hi:
            return p
    return None


def add_interval(acc, cat, a, b):
    """Add [a,b] to acc[period][cat], clipped to each period."""
    for p, (lo, hi) in PERIODS.items():
        s = clip(a, b, lo, hi)
        if s > 0:
            acc[p][cat] += s


def held_overlap(a, b):
    return sum(clip(a, b, lo, hi) for _, lo, hi in HELD)


STALL = 15 * 60  # no tool pending and no output for this long: the process is gone, not generating


def analyse_session(sid, name, recs):
    """Split a session's records into runs; attribute every interval inside a run."""
    runs = []
    cur = None
    pending = {}  # tool_use_id -> (cat, start, cmd, name, is_bg)
    bg = {}  # background task id -> cat
    calls = []
    last_t = None
    in_turn = False
    api_err = False

    def new_run(t, prompt, source):
        nonlocal cur, in_turn, last_t, api_err
        cur = {"session": sid, "name": name, "start": t, "iv": [], "prompt": prompt[:200],
               "source": source, "end_turn_at": None, "cut": False}
        pending.clear()
        in_turn = True
        last_t = t
        api_err = False

    def close_run():
        nonlocal cur
        if cur is None:
            return
        if cur["end_turn_at"] is not None and not in_turn:
            cur["end"] = cur["end_turn_at"]
        else:
            cur["end"] = last_t
            cur["cut"] = True
        # drop intervals after the end
        cur["iv"] = [(c, a, min(b, cur["end"])) for c, a, b in cur["iv"] if a < cur["end"]]
        runs.append(cur)
        cur = None

    for kind, t, a, b in recs:
        if kind == "prompt":
            notif = b.startswith("<task-notification>")
            stale_notif = notif and "previous session ended" in b
            if cur is not None and notif and not stale_notif and not in_turn and cur["end_turn_at"] is not None \
                    and t - cur["end_turn_at"] < 6 * 3600:
                # same process: it waited for a background task after its turn ended
                m = re.search(r"<task-id>([^<]+)</task-id>", b)
                cat = bg.get(m.group(1), "waiting on others") if m else "waiting on others"
                cur["iv"].append((cat, last_t, t))
                in_turn = True
                cur["end_turn_at"] = None
                last_t = t
                continue
            new_process = a in ("sdk", "typed", "") or stale_notif or not in_turn
            if cur is not None and in_turn and a == "queued":
                new_process = False
            if new_process:
                # a prompt within 10 s of the previous run start belongs to the same start
                if cur is not None and in_turn and t - cur["start"] < 10 and not cur["iv"]:
                    cur["prompt"] += " | " + b[:100]
                    continue
                close_run()
                new_run(t, b, a)
            else:
                cur["iv"].append(("model" if not pending else max_cat(pending), last_t, t))
                last_t = t
            continue
        if cur is None:
            continue
        if kind == "assistant" and b is None:
            continue
        if pending:
            cat = max_cat(pending)
        elif api_err:
            cat = "api retry"
        else:
            cat = "model"
        if kind == "result" and b.startswith("[Tool call interrupted"):
            # synthetic, written when the next process resumed: the run was cut at last_t
            pending.pop(a, None)
            cur["cut"] = True
            continue
        if cat == "model" and t - last_t > STALL:
            # nothing pending, nothing written for > 15 min: the process died (pause --now, kill)
            in_turn = True
            cur["end_turn_at"] = None
            close_run()
            if kind != "assistant":
                continue
            new_run(t, "(resumed without prompt)", "")
        if kind == "assistant":
            cur["iv"].append((cat, last_t, t))
            api_err = False
            uses, stop = b, a
            for u in uses:
                c = classify_tool(u["name"], u.get("input", {}))
                pending[u["id"]] = (c, t, u.get("input", {}).get("command", "") if u["name"] == "Bash" else u["name"],
                                    u["name"], bool(u.get("input", {}).get("run_in_background")))
            if stop == "end_turn" and not pending:
                in_turn = False
                cur["end_turn_at"] = t
            elif uses:
                cur["end_turn_at"] = None
        elif kind == "result":
            uid, txt = a, b
            cur["iv"].append((cat, last_t, t))
            if uid in pending:
                c, st, cmd, nm, isbg = pending.pop(uid)
                calls.append({"cat": c, "name": nm, "start": st, "end": t, "cmd": cmd, "out": txt, "bg": isbg,
                              "session": sid, "agent": name, "cut": False})
                m = re.search(r"with ID: (\w+)", txt)
                if isbg and m:
                    bg[m.group(1)] = c
            in_turn = True
        elif kind == "api_error":
            cur["iv"].append((cat, last_t, t))
            api_err = True
        last_t = t
    if cur is not None:
        if in_turn and NOW - last_t < 900:
            cur["live"] = True
            cur["end_turn_at"] = last_t
            in_turn = False
        close_run()
    return runs, calls


def max_cat(pending):
    return min((v[0] for v in pending.values()), key=lambda c: PRIO[c])


def pct(xs, q):
    if not xs:
        return float("nan")
    xs = sorted(xs)
    k = (len(xs) - 1) * q
    f = int(k)
    c = min(f + 1, len(xs) - 1)
    return xs[f] + (xs[c] - xs[f]) * (k - f)


def main():
    status = json.load(open(os.path.join(MAIN, ".claude/runs/status.json")))["runs"]
    sessions = {}
    for sid, v in status.items():
        p = os.path.join(PROJ, sid + ".jsonl")
        if not os.path.exists(p):
            hits = glob.glob(os.path.expanduser(f"~/.claude/projects/*/{sid}.jsonl"))
            if not hits:
                continue
            p = hits[0]
        sessions[sid] = (v["name"], v["dir"], p)

    lo_all = min(lo for lo, _ in PERIODS.values())
    all_runs, all_calls = [], []
    for sid, (name, d, p) in sessions.items():
        # The transcripts as they were at the end of the last period: a re-run reproduces its table.
        recs = [r for r in load_session(p) if lo_all - 3600 <= r[1] <= NOW]
        if not recs:
            continue
        runs, calls = analyse_session(sid, name, recs)
        runs = [r for r in runs if r["end"] >= lo_all]
        all_runs += runs
        all_calls += [c for c in calls if c["end"] >= lo_all]

    # ---------- in-run attribution ----------
    acc = {p: defaultdict(float) for p in PERIODS}
    acc_cut = {p: defaultdict(float) for p in PERIODS}
    nruns = {p: [0, 0] for p in PERIODS}
    run_durs = {p: [] for p in PERIODS}
    for r in all_runs:
        tgt = acc_cut if r["cut"] else acc
        for cat, a, b in r["iv"]:
            add_interval(tgt, cat, a, b)
        p = period_of(r["start"]) or period_of(r["end"])
        if p:
            nruns[p][1 if r["cut"] else 0] += 1
            if not r["cut"]:
                run_durs[p].append(r["end"] - r["start"])

    # ---------- between runs: idle / held / unpushed ----------
    pushes = defaultdict(list)  # branch -> [(time, from, to)]
    for f in sorted(glob.glob(os.path.join(OUT, "data/events_*.json"))):
        for e in json.load(open(f)):
            pd = e.get("push_data") or {}
            if ts(e["created_at"]) > FETCHED:
                continue  # not yet pushed when the table's data was fetched
            if pd.get("ref_type") == "branch" and pd.get("action") in ("pushed", "created") and pd.get("commit_to"):
                pushes[pd["ref"]].append((ts(e["created_at"]), pd.get("commit_from"), pd["commit_to"]))
    unpushed = defaultdict(list)  # session name -> [(commit_time, push_time)]

    def git(*args):
        return subprocess.run(["git", "-C", MAIN, *args], capture_output=True, text=True).stdout

    for ref, ps in pushes.items():
        m = re.match(r"(?:feature|fix)/(.+)", ref)
        if not m:
            continue
        for (t, frm, to) in ps:
            rng = f"{frm}..{to}" if frm and not frm.startswith("0000") else f"origin/develop..{to}"
            out = git("log", "--format=%ct", rng)
            for line in out.split():
                ct = float(line)
                if t - ct > 5:
                    unpushed[m.group(1)].append((ct, t))

    def union(ivs):
        ivs = sorted(ivs)
        out = []
        for a, b in ivs:
            if out and a <= out[-1][1]:
                out[-1][1] = max(out[-1][1], b)
            else:
                out.append([a, b])
        return out

    def inter(xs, ys):
        out = []
        for a1, b1 in xs:
            for a2, b2 in ys:
                lo, hi = max(a1, a2), min(b1, b2)
                if hi > lo:
                    out.append((lo, hi))
        return out

    def minus(xs, ys):
        out = list(xs)
        for a2, b2 in ys:
            nxt = []
            for a1, b1 in out:
                if b2 <= a1 or a2 >= b1:
                    nxt.append((a1, b1))
                    continue
                if a1 < a2:
                    nxt.append((a1, a2))
                if b2 < b1:
                    nxt.append((b2, b1))
            out = nxt
        return out

    def total(xs):
        return sum(b - a for a, b in xs)

    held_iv = [(lo, hi) for _, lo, hi in HELD]
    between = {p: defaultdict(float) for p in PERIODS}
    by_session = defaultdict(list)
    for r in all_runs:
        by_session[r["name"]].append(r)
    for name, rs in by_session.items():
        rs.sort(key=lambda r: r["start"])
        up = union(unpushed.get(name, []))
        gaps = [(rs[i]["end"], rs[i + 1]["start"]) for i in range(len(rs) - 1) if rs[i + 1]["start"] > rs[i]["end"]]
        for p, per in PERIODS.items():
            g = inter(gaps, [per])
            held = inter(g, held_iv)
            idle = minus(g, held_iv)
            between[p]["held (pause/crash/reboot)"] += total(held)
            between[p]["idle"] += total(idle)
            between[p]["idle with unpushed commits"] += total(inter(idle, [tuple(x) for x in up]))

    # ---------- engine queue latency (orchestrator runs tell -> run start) ----------
    tells = []
    if ORCH and os.path.exists(ORCH):
        for line in open(ORCH):
            d = json.loads(line)
            if d.get("type") != "assistant":
                continue
            for u in d["message"]["content"]:
                if u.get("type") == "tool_use" and u["name"] == "Bash" and "runs tell" in u["input"].get("command", ""):
                    tells.append((ts(d["timestamp"]), u["input"]["command"]))
    qlat = []
    for r in all_runs:
        pr = r["prompt"]
        m = re.search(r"\[orchestrator\]\s*(.{30})", pr)
        if not m:
            continue
        key = m.group(1)
        cands = [t for t, c in tells if key in c and t <= r["start"] + 1 and r["start"] - t < 86400]
        if cands:
            t = max(cands)
            busy = any(o["name"] == r["name"] and o["start"] <= t < o["end"] for o in all_runs)
            held = held_overlap(t, r["start"]) > 0
            qlat.append((r["name"], t, r["start"] - t, period_of(r["start"]), busy, held))

    # ---------- tool calls: checks ----------
    checks = []
    lockwait = {p: [0, 0] for p in PERIODS}
    lockdur = {p: [] for p in PERIODS}
    for c in all_calls:
        p = period_of(c["start"])
        if not p or c["cut"]:
            continue
        if c["name"] == "Bash" and ("another checkout builds" in c["out"]):
            lockwait[p][0] += 1
            lockdur[p].append(c["end"] - c["start"])
        if c["name"] == "Bash" and re.search(r"wt:check|\bcargo\b", c["cmd"]) and not c["bg"]:
            m = re.search(r"checked in (\d+) s", c["out"])
            fail = bool(re.search(r"✗|failed|exit [1-9]", c["out"][-400:]))
            checks.append({"period": p, "agent": c["agent"], "start": iso(c["start"]),
                           "kind": "wt:check" if "wt:check" in c["cmd"] else "cargo",
                           "call_s": round(c["end"] - c["start"], 1),
                           "checked_in_s": int(m.group(1)) if m else "",
                           "lock_wait_line": "another checkout builds" in c["out"],
                           "sccache_or_from_scratch": "Compiling" in c["out"], "failed": fail})
    for c in all_calls:
        p = period_of(c["start"])
        if p:
            lockwait[p][1] += 1

    # ---------- issues: first commit -> merged ----------
    mrs = []
    for f in sorted(glob.glob(os.path.join(OUT, "data/mrs_*.json"))):
        mrs += json.load(open(f))
    issues = []
    for mr in mrs:
        if not mr.get("merged_at"):
            continue
        mt = ts(mr["merged_at"])
        if mt < lo_all or mt > FETCHED:
            continue
        m = re.match(r"(?:feature|fix)/(.+)", mr["source_branch"])
        if not m or m.group(1) not in by_session:
            continue
        name = m.group(1)
        head = mr["sha"]
        mc = mr.get("merge_commit_sha") or mr.get("squash_commit_sha")
        out = git("log", "--format=%ct", f"{mc}^1..{head}") if mc else ""
        cts = [float(x) for x in out.split()]
        if not cts:
            continue
        first = min(cts)
        rs = [r for r in by_session[name] if r["end"] > first and r["start"] < mt]
        active = sum(clip(r["start"], r["end"], first, mt) for r in rs)
        last_end = max((min(r["end"], mt) for r in rs), default=first)
        held = held_overlap(first, mt)
        issues.append({"mr": mr["iid"], "branch": name, "first_commit": iso(first), "mr_created": iso(ts(mr["created_at"])),
                       "merged": iso(mt), "period_merged": period_of(mt) or "",
                       "lead_h": round((mt - first) / 3600, 2), "agent_active_h": round(active / 3600, 2),
                       "waiting_between_runs_h": round(max(0, last_end - first - active) / 3600, 2),
                       "after_last_run_to_merge_h": round((mt - last_end) / 3600, 2),
                       "held_overlap_h": round(held / 3600, 2), "runs": len(rs)})

    # tool call stats per category
    callstats = {p: defaultdict(list) for p in PERIODS}
    for c in all_calls:
        p = period_of(c["start"])
        if p and not c["cut"]:
            callstats[p][c["cat"]].append(c["end"] - c["start"])

    # ---------- write outputs ----------
    rows = []
    for p in PERIODS:
        tot = sum(acc[p].values())
        for cat in CATS:
            v = acc[p].get(cat, 0.0)
            cs = callstats[p].get(cat, [])
            rows.append({"period": p, "scope": "in clean runs", "category": cat, "hours": round(v / 3600, 3),
                         "share_of_agent_wall": round(v / tot, 4) if tot else 0, "tool_calls": len(cs),
                         "call_median_s": round(statistics.median(cs), 1) if cs else "",
                         "call_p90_s": round(pct(cs, .9), 1) if cs else ""})
        agents = {r["name"] for r in all_runs if period_of(r["start"]) == p}
        for k, v in [("clean runs", nruns[p][0]), ("cut runs", nruns[p][1]), ("agents", len(agents)),
                     ("agent-hours in clean runs", round(tot / 3600, 2)),
                     ("tool calls", sum(len(x) for x in callstats[p].values())),
                     ("run minutes median", round(statistics.median(run_durs[p]) / 60, 1) if run_durs[p] else ""),
                     ("run minutes p90", round(pct(run_durs[p], .9) / 60, 1) if run_durs[p] else "")]:
            rows.append({"period": p, "scope": "sample", "category": k, "hours": v})
        for cat, v in sorted(acc_cut[p].items()):
            rows.append({"period": p, "scope": "in cut runs", "category": cat, "hours": round(v / 3600, 3), "share_of_agent_wall": ""})
        for cat, v in between[p].items():
            rows.append({"period": p, "scope": "between runs (agent-hours)", "category": cat, "hours": round(v / 3600, 3), "share_of_agent_wall": ""})
    with open(os.path.join(OUT, "categories.csv"), "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=list(rows[0]), restval="")
        w.writeheader()
        w.writerows(rows)
    with open(os.path.join(OUT, "runs.csv"), "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["agent", "session", "start", "end", "minutes", "cut", "period", "prompt"] + CATS)
        for r in sorted(all_runs, key=lambda r: r["start"]):
            per = defaultdict(float)
            for cat, a, b in r["iv"]:
                per[cat] += b - a
            w.writerow([r["name"], r["session"], iso(r["start"]), iso(r["end"]), round((r["end"] - r["start"]) / 60, 1),
                        r["cut"], period_of(r["start"]) or "", r["prompt"].replace("\n", " ")[:100]] +
                       [round(per[c] / 60, 2) for c in CATS])
    if checks:
        with open(os.path.join(OUT, "checks.csv"), "w", newline="") as f:
            w = csv.DictWriter(f, fieldnames=list(checks[0]))
            w.writeheader()
            w.writerows(checks)
    if issues:
        with open(os.path.join(OUT, "issues.csv"), "w", newline="") as f:
            w = csv.DictWriter(f, fieldnames=list(issues[0]))
            w.writeheader()
            w.writerows(sorted(issues, key=lambda x: x["merged"]))
    with open(os.path.join(OUT, "queue_latency.csv"), "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["agent", "told", "latency_s", "period", "agent_was_in_a_run", "held_window"])
        for n, t, l, p, busy, held in qlat:
            w.writerow([n, iso(t), round(l, 1), p, busy, held])

    out = []
    for p in PERIODS:
        tot = sum(acc[p].values())
        agents = {r["name"] for r in all_runs if period_of(r["start"]) == p}
        out.append(f"== {p} {iso(PERIODS[p][0])} → {iso(PERIODS[p][1])}")
        out.append(f"clean runs {nruns[p][0]}, cut runs {nruns[p][1]}, agents {len(agents)}, "
                   f"agent wall in clean runs {tot/3600:.2f} h, tool calls {sum(len(v) for v in callstats[p].values())}")
        if run_durs[p]:
            out.append(f"run minutes: median {statistics.median(run_durs[p])/60:.1f}, p90 {pct(run_durs[p],.9)/60:.1f}")
        for cat in CATS:
            v = acc[p].get(cat, 0)
            cs = callstats[p].get(cat, [])
            extra = f"  calls {len(cs)}, median {statistics.median(cs):.1f}s, p90 {pct(cs,.9):.1f}s" if cs else ""
            out.append(f"  {cat:20s} {v/3600:7.2f} h  {100*v/tot if tot else 0:5.1f}%{extra}")
        out.append(f"  cut runs: " + ", ".join(f"{k} {v/3600:.2f} h" for k, v in acc_cut[p].items()))
        out.append(f"  between runs: " + ", ".join(f"{k} {v/3600:.2f} h" for k, v in between[p].items()))
        ck = [c for c in checks if c["period"] == p and c["kind"] == "wt:check"]
        cc = [c["call_s"] for c in ck]
        ci = [c["checked_in_s"] for c in ck if c["checked_in_s"] != ""]
        if cc:
            out.append(f"  wt:check foreground calls {len(cc)}: median {statistics.median(cc):.0f}s, p90 {pct(cc,.9):.0f}s, "
                       f"total {sum(cc)/3600:.2f} h; 'checked in' n={len(ci)} median {statistics.median(ci) if ci else float('nan')}s; "
                       f"with lock-wait line {sum(c['lock_wait_line'] for c in ck)}; compiled {sum(c['sccache_or_from_scratch'] for c in ck)}")
        cg = [c["call_s"] for c in checks if c["period"] == p and c["kind"] == "cargo"]
        if cg:
            out.append(f"  cargo calls {len(cg)}: median {statistics.median(cg):.0f}s, p90 {pct(cg,.9):.0f}s, total {sum(cg)/3600:.2f} h")
        ld = lockdur[p]
        out.append(f"  tool results with 'another checkout builds' (lock wait): {lockwait[p][0]} of {lockwait[p][1]} calls"
                   + (f", those calls took median {statistics.median(ld):.0f}s, p90 {pct(ld,.9):.0f}s, total {sum(ld)/3600:.2f} h" if ld else ""))
        if cc:
            out.append(f"  wt:check calls >= 580 s (hit the agent's timeout 590/600): {sum(1 for x in cc if x >= 580)}; mean {statistics.mean(cc):.0f}s")
        ql = [l for n, t, l, pp, busy, held in qlat if pp == p]
        if ql:
            out.append(f"  engine queue latency (orchestrator runs tell -> run start) n={len(ql)}: median {statistics.median(ql):.1f}s, p90 {pct(ql,.9):.0f}s; "
                       f"> 60 s: {[(round(l), 'agent in a run' if busy else 'hold' if held else '?') for n, t, l, pp, busy, held in qlat if pp == p and l > 60]}")
        # concurrency: how many clean or cut runs were in flight, time-weighted over this period's in-run time
        evs = sorted([(r["start"], 1) for r in all_runs] + [(r["end"], -1) for r in all_runs])
        lo, hi = PERIODS[p]
        cur_n, last, wsum, tsum, peak = 0, None, 0.0, 0.0, 0
        for t, dlt in evs:
            if last is not None and cur_n > 0:
                seg = clip(last, t, lo, hi)
                wsum += seg * cur_n
                tsum += seg
            cur_n += dlt
            if lo <= t < hi:
                peak = max(peak, cur_n)
            last = t
        def conc_at(t):
            return sum(1 for r in all_runs if r["start"] <= t < r["end"])
        out.append(f"  concurrent runs while any runs: mean {wsum/tsum if tsum else 0:.1f}, peak {peak}")
        for lab, f_ in [("<=3", lambda n: n <= 3), ("4-8", lambda n: 4 <= n <= 8), (">8", lambda n: n > 8)]:
            xs = [c["call_s"] for c in ck if f_(conc_at(ts(c["start"].replace(" ", "T").replace("Z", ":00Z"))))]
            if xs:
                out.append(f"    wt:check with {lab} runs in flight: n={len(xs)}, median {statistics.median(xs):.0f}s, p90 {pct(xs,.9):.0f}s, mean {statistics.mean(xs):.0f}s")
        iss = [i for i in issues if i["period_merged"] == p]
        if iss:
            lead = [i["lead_h"] for i in iss]
            act = [i["agent_active_h"] for i in iss]
            aft = [i["after_last_run_to_merge_h"] for i in iss]
            out.append(f"  issues merged {len(iss)}: lead median {statistics.median(lead):.1f} h (p90 {pct(lead,.9):.1f}); "
                       f"agent-active median {statistics.median(act):.2f} h; last run→merge median {statistics.median(aft):.2f} h; "
                       f"active share of total lead {sum(act)/sum(lead)*100:.0f}%")
    txt = "\n".join(out)
    open(os.path.join(OUT, "summary.txt"), "w").write(txt + "\n")
    print(txt)


if __name__ == "__main__":
    main()
