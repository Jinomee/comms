"""Run the comms demo for real and record each command's actual output.

Usage, from the repo root after `cargo build --release`:
    python3 scripts/demo/record.py   # the ask scene needs a logged-in `claude`
    python3 scripts/demo/render.py   # needs Pillow and ffmpeg; writes docs/media/demo.{gif,mp4}

Every command below is executed against the real comms binary in a fresh
project; the renderer only replays what was recorded here.
"""
import json
import os
import subprocess
import tempfile
import textwrap

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
BIN = os.environ.get("COMMS_DEMO_BIN", os.path.join(REPO, "target", "release"))
SCRATCH = tempfile.mkdtemp(prefix="comms-demo-")

root = tempfile.mkdtemp(prefix="proj", dir=SCRATCH)
home = tempfile.mkdtemp(prefix="home", dir=SCRATCH)
project = os.path.join(root, "shop-api")
os.makedirs(os.path.join(project, "src", "auth"))

with open(os.path.join(project, "queue.py"), "w") as f:
    f.write(textwrap.dedent('''\
        import threading

        class Queue:
            def __init__(self):
                self.items = []
                self.lock = threading.Lock()

            def push(self, item):
                with self.lock:
                    self.items.append(item)

            def flush(self):
                batch = self.items
                self.items = []
                return batch
    '''))
with open(os.path.join(project, "src", "auth", "session.py"), "w") as f:
    f.write("def refresh(token):\n    ...\n")
subprocess.run(["git", "init", "-q"], cwd=project, check=True)

base_env = {
    "HOME": home,
    "PATH": f"{BIN}:/usr/bin:/bin",
    "TERM": "xterm",
    "LANG": "C.UTF-8",
}
# `comms ask claude` needs this container's Claude credentials.
ask_env = dict(os.environ)
ask_env.update({"PATH": f"{BIN}:" + os.environ["PATH"], "HOME": os.environ["HOME"]})
for key in [k for k in ask_env if k.startswith("COMMS_")]:
    del ask_env[key]


def run(cmd, who="you", env_extra=None, use_ask_env=False, display=None):
    env = dict(ask_env if use_ask_env else base_env)
    if who != "you":
        env["COMMS_PROCESS_ID"] = f"proc-{who}"
    env.update(env_extra or {})
    proc = subprocess.run(
        cmd, shell=True, cwd=project, env=env, capture_output=True, text=True
    )
    out = (proc.stdout + proc.stderr).rstrip("\n")
    print(f"[{who}] $ {display or cmd}\n{out}\n(exit {proc.returncode})\n")
    return {"who": who, "cmd": display or cmd, "out": out, "code": proc.returncode}


scenes = []

def scene(title, subtitle, steps):
    scenes.append({"title": title, "subtitle": subtitle, "steps": steps})


scene("Set up a project", "one shared space for every agent working here", [
    run("comms init", display="comms init"),
])

question = (
    "Is there a race condition in Queue.flush()? Answer in at most two short "
    "sentences, plain text, no markdown."
)
scene("Get a second opinion", "a fresh, read-only Claude answers and exits", [
    run(
        f'comms ask claude "{question}" --file queue.py 2>/dev/null',
        use_ask_env=True,
        display='comms ask claude "Is there a race condition in Queue.flush()?" --file queue.py',
    ),
])

# Two agent shells join. (In real use `comms claude` / `comms codex` launch the
# agents; here plain shells run the same commands the agents run.)
join = [
    run("comms start --as luna >/dev/null && echo 'luna joined'", who="luna",
        display="comms start --as luna"),
    run("comms start --as nova >/dev/null && echo 'nova joined'", who="nova",
        display="comms start --as nova"),
]
scene("Agents join", "each agent shell gets a name", join)

scene("Claim files before editing", "other agents' edits to them get blocked", [
    run('comms claim "src/auth/**" -n "refactoring session refresh"', who="luna"),
    run("comms claims --check src/auth/session.py", who="nova"),
])

scene("Talk in a room", "plain messages reach the room, not everyone", [
    run("comms room join auth | head -1", who="luna", display="comms room join auth"),
    run("comms room join auth | head -1", who="nova", display="comms room join auth"),
    run('comms send -- "session refresh is done, can you review src/auth?"', who="luna"),
    run("comms listen --timeout 5", who="nova"),
])

scene("No runaway loops", "agents pause after 20 messages without you", [
    run("comms budget"),
])

with open(os.path.join(HERE, "cast.json"), "w") as f:
    json.dump(scenes, f, indent=2)
print("project:", project)
