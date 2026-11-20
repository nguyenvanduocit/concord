#!/usr/bin/env python3
"""coord v2 - coordination board for parallel coding agents.

State is one JSON file guarded by flock; every command is a short critical section.
Blocking commands poll (2s) outside the lock.

  init <agent>...                          orchestrator: declare the team, reset the board
  plan <agent> <modifies_csv> <uses_csv>   register intent; '-' for none. Queue order = plan order
  claim <agent> <fn>                       block until this agent's turn on fn; print LATEST change
  done <agent> <fn> <change> <note...>     record the contract note, pass fn to the next in queue
  finish <agent> <change>                  block until nobody else holds/queues fns this agent uses;
                                           exit 0 = clean, exit 10 = new contract changes printed:
                                           rebase onto them, adapt, re-test, then finish again
  log                                      print the event log
"""
import fcntl, json, os, sys, time

BOARD = "/tmp/sw-v2-board"
STATE = os.path.join(BOARD, "state.json")
POLL, TIMEOUT = 2, 900


def locked(mutate):
    os.makedirs(BOARD, exist_ok=True)
    with open(os.path.join(BOARD, "lock"), "w") as lk:
        fcntl.flock(lk, fcntl.LOCK_EX)
        st = json.load(open(STATE)) if os.path.exists(STATE) else None
        out = mutate(st)
        if out is not None and out[0] is not None:
            json.dump(out[0], open(STATE + ".tmp", "w"), indent=1)
            os.replace(STATE + ".tmp", STATE)
        return None if out is None else out[1]


def log(st, msg):
    st["log"].append(f"{time.strftime('%H:%M:%S')} {msg}")


def wait_until(check, what):
    deadline = time.time() + TIMEOUT
    while time.time() < deadline:
        res = locked(lambda st: (None, check(st)))
        if res is not None:
            return res
        time.sleep(POLL)
    sys.exit(f"TIMEOUT waiting for {what}")


def all_planned(st):
    return all(a in st["plans"] for a in st["team"])


def latest(st, fn):
    hist = st["history"].get(fn, [])
    return hist[-1] if hist else None


def cmd_init(agents):
    locked(lambda _: ({"team": agents, "plans": {}, "queue": {}, "history": {}, "seen": {}, "log": [f"INIT team={','.join(agents)}"]}, None))
    print("INIT", " ".join(agents))


def cmd_plan(agent, modifies, uses):
    mods = [] if modifies == "-" else modifies.split(",")
    used = [] if uses == "-" else uses.split(",")

    def m(st):
        # Re-planning replaces the earlier plan: drop queue slots for functions no longer
        # modified, add a slot only where the agent has none yet (keeps its place in line).
        old = st["plans"].get(agent, {"modifies": []})["modifies"]
        for fn in old:
            if fn not in mods and agent in st["queue"].get(fn, []):
                st["queue"][fn].remove(agent)
        st["plans"][agent] = {"modifies": mods, "uses": used}
        for fn in mods:
            q = st["queue"].setdefault(fn, [])
            if agent not in q:
                q.append(agent)
        st["seen"].setdefault(agent, [])
        log(st, f"PLAN {agent} modifies={mods} uses={used}")
        return st, None
    locked(m)
    print(f"PLANNED {agent} modifies={mods} uses={used}")


def cmd_claim(agent, fn):
    def my_turn(st):
        if not all_planned(st):
            return None
        q = st["queue"].get(fn, [])
        if agent not in q:
            return "NOTPLANNED"
        return "OK" if q[0] == agent else None
    res = wait_until(my_turn, f"turn on {fn}")
    if res == "NOTPLANNED":
        sys.exit(f"ERROR {agent} did not plan to modify {fn}; run plan first")

    def m(st):
        log(st, f"CLAIM {agent} {fn}")
        for h in st["history"].get(fn, []):
            if h["id"] not in st["seen"][agent]:
                st["seen"][agent].append(h["id"])
        return st, latest(st, fn)
    last = locked(m)
    print(f"CLAIMED {fn}")
    if last:
        print(f"LATEST {last['change']}  by {last['agent']}: {last['note']}")
        print(f"-> before editing run: jj rebase -b @ -d {last['change']}")


def cmd_done(agent, fn, change, note):
    def m(st):
        q = st["queue"].get(fn, [])
        if not q or q[0] != agent:
            return None, "NOTHOLDER"
        q.pop(0)
        rec = {"id": f"{fn}#{len(st['history'].get(fn, [])) + 1}", "agent": agent, "change": change, "note": note}
        st["history"].setdefault(fn, []).append(rec)
        st["seen"][agent].append(rec["id"])
        users = [a for a, p in st["plans"].items() if fn in p["uses"] and a != agent]
        log(st, f"DONE {agent} {fn} {change} :: {note} (callers to re-check: {users or 'none'})")
        return st, (users, q[0] if q else None)
    res = locked(m)
    if res == "NOTHOLDER":
        sys.exit(f"ERROR {agent} does not hold {fn}")
    users, nxt = res
    print(f"RELEASED {fn}; next holder: {nxt or 'none'}; callers that will be told: {users or 'none'}")


def cmd_finish(agent, change):
    def quiet(st):
        if not all_planned(st):
            return None
        for fn in st["plans"][agent]["uses"]:
            if any(a != agent for a in st["queue"].get(fn, [])):
                return None
        return "OK"
    wait_until(quiet, "callers' dependencies to settle")

    def m(st):
        new = []
        for fn in st["plans"][agent]["uses"]:
            for h in st["history"].get(fn, []):
                if h["id"] not in st["seen"][agent]:
                    new.append(h)
                    st["seen"][agent].append(h["id"])
        st.setdefault("heads", {})[agent] = change
        log(st, f"FINISH {agent} {change} new_contracts={[h['id'] for h in new]}")
        return st, new
    new = locked(m)
    if not new:
        print(f"FINISHED {agent} at {change}: no unseen contract changes")
        return
    print("NEW CONTRACT CHANGES in functions you call:")
    for h in new:
        print(f"  {h['id']} by {h['agent']} at {h['change']}: {h['note']}")
    tip = new[-1]["change"]
    print(f"-> run: jj rebase -b @ -d {tip}; adapt your code; re-run tests; jj describe; then finish again")
    sys.exit(10)


def main(argv):
    if not argv:
        sys.exit(__doc__)
    c, a = argv[0], argv[1:]
    if c == "init":
        cmd_init(a)
    elif c == "plan" and len(a) == 3:
        cmd_plan(*a)
    elif c == "claim" and len(a) == 2:
        cmd_claim(*a)
    elif c == "done" and len(a) >= 4:
        cmd_done(a[0], a[1], a[2], " ".join(a[3:]))
    elif c == "finish" and len(a) == 2:
        cmd_finish(*a)
    elif c == "log":
        print("\n".join(json.load(open(STATE))["log"]) if os.path.exists(STATE) else "(empty)")
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])
