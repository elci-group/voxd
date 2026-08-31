#!/usr/bin/env python3
import copy
from curly_expand import expand_or_literal, cartesian
"""Benchmark how well voxd's mimic integration keeps TTS-provider characters
("tokens") off the bill.

Dependency-free (stdlib only), matching the convention of the other tools/
scripts in this repo. It:

  1. Reads the real ~/.config/voxd/config.toml for mimic's URL, pv_bin,
     object_root, and server.auth_token (mimicd expects that exact token when
     mimic.auth_token is empty, so a temp instance must reuse it).
  2. Spawns a throwaway voxd daemon (fresh cache/state dir, random free port)
     pointed at the REAL, already-running mimicd — mimic's object store is
     shared infrastructure and can't be sandboxed from the client side.
  3. By default, redirects ElevenLabs span synthesis to a local mock server
     (via VOXD_ELEVENLABS_BASE_URL) so repeated runs are free and
     deterministic; --real-tts uses the real configured key/endpoint instead.
  4. Drives a built-in corpus of voxd-shaped text (recap narrations, listen
     intent replies, CLI-speak acknowledgements) through /speak in two
     scenarios (end-to-end, and mimic-isolation with no_cache=true), across
     two passes each to show the cold -> warm curve.
  5. Cross-checks the client-observed /speak responses against voxd's
     structured VOXD_TRACE_JSONL trace, and reports token-efficiency,
     admission-denial and visual-only-fallback rates, and latency.

Usage:
    python3 tools/mimic_bench.py
    python3 tools/mimic_bench.py --real-tts --passes 3
    python3 tools/mimic_bench.py --voxd-bin target/release/voxd --out /tmp/report
"""
import argparse
import contextlib
import http.server
import json
import os
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

try:
    import tomllib
except ImportError:  # pragma: no cover - repo targets Python 3.11+
    print("mimic_bench.py requires Python 3.11+ (tomllib)", file=sys.stderr)
    sys.exit(1)

REPO_ROOT = Path(__file__).resolve().parent.parent
REAL_CONFIG_PATH = Path.home() / ".config" / "voxd" / "config.toml"


# --------------------------------------------------------------------------
# Corpus: voxd-shaped text, tagged by category. `{nonce}` slots keep results
# independent of mimic's shared, persistent object store across runs, while
# the surrounding scaffolding repeats verbatim *within* a run so mimic's
# word/phrase-level cache has something realistic to reuse.
# --------------------------------------------------------------------------

RECAP_TEMPLATES = [
    "While you were away, {n} files were modified across the {proj} project "
    "and the test suite passed.",
    "While you were away, {n} files were modified across the {proj} project "
    "and the build failed with a type error.",
    "Finished the refactor in {proj}: {n} functions were renamed and all "
    "call sites were updated.",
    "Task complete. Modified {n} files in {proj} and committed the changes.",
    "Task complete. Modified {n} files in {proj} but left the commit for "
    "you to review.",
    "The {proj} branch is ready to merge; {n} review comments were addressed.",
]

LISTEN_INTENT_TEMPLATES = [
    "The current time is {n} o'clock.",
    "Today's date is the {n}th.",
    "voxd has been running for {n} minutes.",
    "Disk usage is at {n} percent, {n} gigabytes free.",
    "System specs: {n} cores, {n} gigabytes of memory.",
    "voxd status: {n} projects tracked, cache healthy.",
]

CLI_SPEAK_TEMPLATES = [
    "build done",
    "tests passed",
    "{n} files changed, {n} insertions",
    "deploy to {proj} finished",
    "commit {proj}-{n} pushed",
    "lint clean, {n} warnings",
]


def build_corpus(nonce: str, seed: int):
    """Return a list of {category, text} dicts. Deterministic given a seed."""
    import random

    rng = random.Random(seed)
    projects = ["atlas", "bounty", "cortex", "helix"]
    items = []
    for templates, category in (
        (RECAP_TEMPLATES, "recap"),
        (LISTEN_INTENT_TEMPLATES, "listen_intent"),
        (CLI_SPEAK_TEMPLATES, "cli_speak"),
    ):
        for tmpl in templates:
            for _ in range(4):
                text = tmpl.format(
                    n=rng.randint(1, 99),
                    proj=f"{rng.choice(projects)}-{nonce}",
                )
                items.append({"category": category, "text": text})
    rng.shuffle(items)
    return items


# --------------------------------------------------------------------------
# Mock ElevenLabs: serves pcm_16000 span synthesis with silence sized off
# input text length, so mimic's inject/compose loop runs end-to-end without
# spending real API credits or depending on network latency.
# --------------------------------------------------------------------------

class _MockElevenHandler(http.server.BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        pass

    def do_POST(self):
        length = int(self.headers.get("content-length", 0))
        body = self.rfile.read(length)
        try:
            text = json.loads(body).get("text", "") if body else ""
        except json.JSONDecodeError:
            text = ""
        pcm = b"\x00\x00" * max(1, len(text) * 160)
        self.send_response(200)
        self.send_header("content-type", "audio/pcm")
        self.send_header("content-length", str(len(pcm)))
        self.end_headers()
        self.wfile.write(pcm)


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@contextlib.contextmanager
def mock_eleven_server():
    port = free_port()
    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), _MockElevenHandler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{port}"
    finally:
        server.shutdown()
        server.server_close()


# --------------------------------------------------------------------------
# Temp voxd instance
# --------------------------------------------------------------------------

TEMP_CONFIG_TEMPLATE = """\
[server]
bind = "127.0.0.1:{port}"
auth_token = "{auth_token}"
pid_file = "{work_dir}/voxd.pid"

[elevenlabs]
model_id = "{model_id}"
output_format = "{output_format}"

[groq]
tts_model = "canopylabs/orpheus-v1-english"
voice = "troy"
output_format = "wav"
sample_rate = 24000
stt_model = "whisper-large-v3-turbo"

[providers]
tts = "elevenlabs"
stt = "elevenlabs"

[system_voice]
voice_id = "{system_voice_id}"
label = "system"

[defaults]
stability = 0.5
similarity_boost = 0.75
style = 0.0
speed = 1.0
use_speaker_boost = true

[pool]
voices = []

[cache]
dir = "{work_dir}/cache"
enabled = true
max_mb = 64

[mimic]
enabled = true
url = "{mimic_url}"
auth_token = "{mimic_auth_token}"
pv_bin = "{pv_bin}"
object_root = "{object_root}"

[recap]
enabled = false

[routing]
default_voice = "system"
rules = []
"""


def load_real_config() -> dict:
    if not REAL_CONFIG_PATH.exists():
        print(f"error: real config not found at {REAL_CONFIG_PATH}", file=sys.stderr)
        sys.exit(1)
    with open(REAL_CONFIG_PATH, "rb") as f:
        return tomllib.load(f)


def find_voxd_bin(explicit: str | None) -> Path:
    if explicit:
        p = Path(explicit)
        if not p.exists():
            print(f"error: --voxd-bin {p} does not exist", file=sys.stderr)
            sys.exit(1)
        return p
    for candidate in ("target/release/voxd", "target/debug/voxd"):
        p = REPO_ROOT / candidate
        if p.exists():
            return p
    print("building voxd (debug) ...", file=sys.stderr)
    subprocess.run(["cargo", "build", "--bin", "voxd"], cwd=REPO_ROOT, check=True)
    return REPO_ROOT / "target" / "debug" / "voxd"


class VoxdInstance:
    def __init__(self, voxd_bin: Path, real_cfg: dict, work_dir: Path, eleven_base_url: str | None):
        self.work_dir = work_dir
        self.port = free_port()
        self.auth_token = real_cfg["server"]["auth_token"]
        self.trace_path = work_dir / "trace.jsonl"
        self.log_path = work_dir / "voxd.log"

        (work_dir / "cache").mkdir(parents=True, exist_ok=True)
        config_text = TEMP_CONFIG_TEMPLATE.format(
            port=self.port,
            auth_token=self.auth_token,
            work_dir=work_dir,
            model_id=real_cfg["elevenlabs"]["model_id"],
            output_format=real_cfg["elevenlabs"]["output_format"],
            system_voice_id=real_cfg["system_voice"]["voice_id"],
            mimic_url=real_cfg["mimic"]["url"],
            mimic_auth_token=real_cfg["mimic"]["auth_token"],
            pv_bin=real_cfg["mimic"]["pv_bin"],
            object_root=real_cfg["mimic"]["object_root"],
        )
        self.config_path = work_dir / "config.toml"
        self.config_path.write_text(config_text)

        env = dict(os.environ)
        env["VOXD_TRACE_JSONL"] = str(self.trace_path)
        if eleven_base_url:
            env["VOXD_ELEVENLABS_BASE_URL"] = eleven_base_url

        self.log_file = open(self.log_path, "w")
        self.proc = subprocess.Popen(
            [str(voxd_bin), "--config", str(self.config_path)],
            env=env,
            stdout=self.log_file,
            stderr=subprocess.STDOUT,
        )

    def base_url(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    def wait_healthy(self, timeout=10.0):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if self.proc.poll() is not None:
                self.log_file.flush()
                raise RuntimeError(
                    f"voxd exited early (code {self.proc.returncode}); see {self.log_path}"
                )
            try:
                with urllib.request.urlopen(f"{self.base_url()}/health", timeout=1) as r:
                    if r.status == 200:
                        return
            except (urllib.error.URLError, ConnectionError, OSError):
                pass
            time.sleep(0.2)
        raise RuntimeError(f"voxd never became healthy; see {self.log_path}")

    def speak(self, text: str, no_cache: bool) -> dict:
        body = json.dumps(
            {"text": text, "system": True, "no_cache": no_cache, "play": False}
        ).encode()
        req = urllib.request.Request(
            f"{self.base_url()}/speak",
            data=body,
            method="POST",
            headers={
                "Authorization": f"Bearer {self.auth_token}",
                "Content-Type": "application/json",
            },
        )
        started = time.monotonic()
        try:
            with urllib.request.urlopen(req, timeout=30) as r:
                resp = json.loads(r.read())
                resp["_latency_ms"] = (time.monotonic() - started) * 1000.0
                resp["_error"] = None
                return resp
        except urllib.error.HTTPError as e:
            return {
                "_error": e.read().decode(errors="replace"),
                "_latency_ms": (time.monotonic() - started) * 1000.0,
            }

    def shutdown(self):
        self.proc.terminate()
        try:
            self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait(timeout=5)
        self.log_file.close()

    def read_trace(self):
        events = []
        if not self.trace_path.exists():
            return events
        with open(self.trace_path) as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    events.append(json.loads(line))
                except json.JSONDecodeError:
                    pass
        return events


# --------------------------------------------------------------------------
# Run + analyze
# --------------------------------------------------------------------------

def run_scenario(instance: VoxdInstance, name: str, corpus, passes: int, no_cache: bool):
    """Drive `corpus` through /speak for `passes` passes; return per-request records."""
    records = []
    for pass_idx in range(passes):
        for req_idx, item in enumerate(corpus):
            resp = instance.speak(item["text"], no_cache=no_cache)
            records.append(
                {
                    "scenario": name,
                    "pass": pass_idx,
                    "index": req_idx,
                    "category": item["category"],
                    "text_len": len(item["text"]),
                    **resp,
                }
            )
    return records


def path_of(record: dict) -> str:
    if record.get("_error") is not None:
        return "error"
    if record.get("cached"):
        return "outer_cache"
    if record.get("visual_only"):
        return "visual_only"
    return "mimic"


def cross_check(records, trace_events):
    """Compare client-observed responses against the server's own trace.

    Returns a list of human-readable mismatch descriptions (empty if clean).
    """
    speak_events = [e for e in trace_events if e.get("target") == "voxd::speak"]
    attempted = [r for r in records if r.get("_error") is None]
    mismatches = []
    if len(speak_events) != len(attempted):
        mismatches.append(
            f"trace has {len(speak_events)} 'speak' events but {len(attempted)} "
            "successful client responses were recorded"
        )
        return mismatches
    for i, (rec, ev) in enumerate(zip(attempted, speak_events)):
        for field in ("chars", "provider_chars", "cached", "visual_only"):
            if rec.get(field) != ev.get(field):
                mismatches.append(
                    f"request #{i}: client {field}={rec.get(field)!r} "
                    f"!= trace {field}={ev.get(field)!r}"
                )
    return mismatches


def admission_stats(trace_events):
    admits = [e for e in trace_events if e.get("target") == "voxd::mimic" and e.get("event") == "admit"]
    total = len(admits)
    if total == 0:
        return {"admit_events": 0}
    ram_denied = sum(1 for e in admits if not e.get("ram_admitted"))
    storage_denied = sum(1 for e in admits if not e.get("storage_admitted"))
    return {
        "admit_events": total,
        "ram_denied": ram_denied,
        "storage_denied": storage_denied,
        "ram_denied_pct": 100.0 * ram_denied / total,
        "storage_denied_pct": 100.0 * storage_denied / total,
    }


def pctile(values, p):
    if not values:
        return None
    s = sorted(values)
    k = max(0, min(len(s) - 1, int(round(p / 100.0 * (len(s) - 1)))))
    return s[k]


def summarize_scenario(records):
    attempted = [r for r in records if r.get("_error") is None and not r.get("visual_only")]
    errors = [r for r in records if r.get("_error") is not None]
    visual_only = [r for r in records if r.get("visual_only")]

    total_chars = sum(r["chars"] for r in attempted)
    provider_chars = sum(r["provider_chars"] for r in attempted)
    efficiency = 1.0 - (provider_chars / total_chars) if total_chars else None

    by_category = {}
    for r in attempted:
        c = by_category.setdefault(
            r["category"], {"chars": 0, "provider_chars": 0, "n": 0}
        )
        c["chars"] += r["chars"]
        c["provider_chars"] += r["provider_chars"]
        c["n"] += 1
    for c in by_category.values():
        c["efficiency"] = 1.0 - (c["provider_chars"] / c["chars"]) if c["chars"] else None

    by_pass = {}
    for r in attempted:
        c = by_pass.setdefault(r["pass"], {"chars": 0, "provider_chars": 0, "n": 0})
        c["chars"] += r["chars"]
        c["provider_chars"] += r["provider_chars"]
        c["n"] += 1
    for c in by_pass.values():
        c["efficiency"] = 1.0 - (c["provider_chars"] / c["chars"]) if c["chars"] else None

    latencies = [r["_latency_ms"] for r in records if "_latency_ms" in r]

    return {
        "requests": len(records),
        "attempted": len(attempted),
        "errors": len(errors),
        "visual_only": len(visual_only),
        "total_chars": total_chars,
        "provider_chars": provider_chars,
        "efficiency": efficiency,
        "by_category": by_category,
        "by_pass": by_pass,
        "latency_ms": {
            "p50": pctile(latencies, 50),
            "p90": pctile(latencies, 90),
            "p99": pctile(latencies, 99),
        },
    }


def render_report(meta, scenarios, mismatches, admission) -> str:
    lines = []
    lines.append(f"# mimic token-efficiency benchmark — {meta['timestamp']}")
    lines.append("")
    lines.append(f"voxd: {meta['voxd_bin']}  |  mimic: {meta['mimic_url']}  |  "
                 f"tts backend: {'real ElevenLabs' if meta['real_tts'] else 'mock (local)'}")
    lines.append("")

    if mismatches:
        lines.append("## ⚠ cross-check mismatches (client response vs. server trace)")
        for m in mismatches[:20]:
            lines.append(f"- {m}")
        if len(mismatches) > 20:
            lines.append(f"- ... and {len(mismatches) - 20} more")
        lines.append("")
    else:
        lines.append("Cross-check: client-observed `/speak` responses match the "
                      "server-side trace exactly.")
        lines.append("")

    if admission.get("admit_events"):
        lines.append("## mimic admission")
        lines.append(
            f"- {admission['admit_events']} admission decisions; "
            f"RAM denied {admission['ram_denied']} "
            f"({admission['ram_denied_pct']:.1f}%), "
            f"storage denied {admission['storage_denied']} "
            f"({admission['storage_denied_pct']:.1f}%)"
        )
        if admission["storage_denied_pct"] > 0:
            lines.append(
                "  storage denial means newly-synthesized spans are **not persisted** "
                "into mimic's cache — repeat traffic won't benefit until storage "
                "pressure clears."
            )
        lines.append("")

    for name, summary in scenarios.items():
        lines.append(f"## scenario: {name}")
        eff = summary["efficiency"]
        lines.append(
            f"- {summary['requests']} requests ({summary['errors']} errors, "
            f"{summary['visual_only']} visual-only fallback)"
        )
        lines.append(
            f"- {summary['total_chars']} chars attempted, "
            f"{summary['provider_chars']} billed to the TTS provider"
        )
        lines.append(
            f"- **token efficiency: {eff * 100:.1f}%**" if eff is not None else "- no data"
        )
        lines.append(
            f"- latency p50/p90/p99 (ms): "
            f"{summary['latency_ms']['p50']:.1f} / "
            f"{summary['latency_ms']['p90']:.1f} / "
            f"{summary['latency_ms']['p99']:.1f}"
        )
        lines.append("- by pass (cold -> warm):")
        for pass_idx in sorted(summary["by_pass"]):
            p = summary["by_pass"][pass_idx]
            eff_s = f"{p['efficiency'] * 100:.1f}%" if p["efficiency"] is not None else "n/a"
            lines.append(f"    pass {pass_idx}: {p['n']} req, efficiency {eff_s}")
        lines.append("- by category:")
        for cat, c in sorted(summary["by_category"].items()):
            eff_s = f"{c['efficiency'] * 100:.1f}%" if c["efficiency"] is not None else "n/a"
            lines.append(f"    {cat}: {c['n']} req, efficiency {eff_s}")
        lines.append("")

    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--passes", type=int, default=2, help="passes per scenario (default 2: cold, warm)")
    ap.add_argument("--seed", type=int, default=1, help="corpus shuffle/fill seed")
    ap.add_argument("--real-tts", action="store_true", help="use real ElevenLabs instead of the local mock")
    ap.add_argument("--voxd-bin", default=None, help="path to the voxd binary (default: auto-detect/build)")
    ap.add_argument("--out", default=None, help="report output directory (default: tools/mimic_bench_reports)")
    args = ap.parse_args()

    __curly_voxd_bin = expand_or_literal(args.voxd_bin) if args.voxd_bin is not None else [None]
    __curly_out = expand_or_literal(args.out) if args.out is not None else [None]
    for __curly_v_voxd_bin in __curly_voxd_bin:
        for __curly_v_out in __curly_out:
            args = copy.copy(args)
            args.voxd_bin = __curly_v_voxd_bin
            args.out = __curly_v_out
            real_cfg = load_real_config()
            voxd_bin = find_voxd_bin(args.voxd_bin)
            out_dir = Path(args.out) if args.out else REPO_ROOT / "tools" / "mimic_bench_reports"
            out_dir.mkdir(parents=True, exist_ok=True)

            with tempfile.TemporaryDirectory(prefix="mimic_bench_") as tmp:
                work_dir = Path(tmp)
                eleven_ctx = contextlib.nullcontext(None) if args.real_tts else mock_eleven_server()
                with eleven_ctx as eleven_base_url:
                    instance = VoxdInstance(voxd_bin, real_cfg, work_dir, eleven_base_url)
                    try:
                        instance.wait_healthy()

                        nonce_a = uuid.uuid4().hex[:8]
                        nonce_b = uuid.uuid4().hex[:8]
                        corpus_a = build_corpus(nonce_a, args.seed)
                        corpus_b = build_corpus(nonce_b, args.seed)

                        all_records = []
                        all_records += run_scenario(instance, "end_to_end", corpus_a, args.passes, no_cache=False)
                        all_records += run_scenario(instance, "mimic_isolation", corpus_b, args.passes, no_cache=True)

                        trace_events = instance.read_trace()
                    finally:
                        instance.shutdown()

            mismatches = cross_check(all_records, trace_events)
            admission = admission_stats(trace_events)

            scenarios = {}
            for name in ("end_to_end", "mimic_isolation"):
                scenarios[name] = summarize_scenario([r for r in all_records if r["scenario"] == name])

            meta = {
                "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S"),
                "voxd_bin": str(voxd_bin),
                "mimic_url": real_cfg["mimic"]["url"],
                "real_tts": args.real_tts,
                "passes": args.passes,
            }

            report_id = int(time.time())
            json_path = out_dir / f"bench-{report_id}.json"
            md_path = out_dir / f"bench-{report_id}.md"
            json_path.write_text(
                json.dumps(
                    {
                        "meta": meta,
                        "scenarios": scenarios,
                        "mismatches": mismatches,
                        "admission": admission,
                        "records": all_records,
                    },
                    indent=2,
                )
            )
            report_text = render_report(meta, scenarios, mismatches, admission)
            md_path.write_text(report_text)

            print(report_text)
            print(f"\nfull results: {json_path}")
            print(f"report:       {md_path}")

            if mismatches:
                sys.exit(2)


if __name__ == "__main__":
    def __curly_original_entry():
        main()
    import sys
    import subprocess
    from curly_expand import expand_or_literal, cartesian

    _raw_argv = sys.argv[:]
    _positions = []
    _fields = []
    for _i, _a in enumerate(_raw_argv):
        if _a == "--voxd-bin" and _i + 1 < len(_raw_argv):
            _positions.append(_i + 1)
            _fields.append(expand_or_literal(_raw_argv[_i + 1]))
            break
        if _a.startswith("--voxd-bin="):
            _positions.append(_i)
            _fields.append(["--voxd-bin=" + v for v in expand_or_literal(_a.split("=", 1)[1])])
            break
    for _i, _a in enumerate(_raw_argv):
        if _a == "--out" and _i + 1 < len(_raw_argv):
            _positions.append(_i + 1)
            _fields.append(expand_or_literal(_raw_argv[_i + 1]))
            break
        if _a.startswith("--out="):
            _positions.append(_i)
            _fields.append(["--out=" + v for v in expand_or_literal(_a.split("=", 1)[1])])
            break

    if not _fields or all(len(f) <= 1 for f in _fields):
        __curly_original_entry()
    else:
        _combos = cartesian(_fields)
        _failed = False
        for _combo in _combos:
            _new_argv = list(_raw_argv)
            for _pos, _val in zip(_positions, _combo):
                _new_argv[_pos] = _val
            _r = subprocess.run([sys.executable] + _new_argv)
            if _r.returncode != 0:
                _failed = True
        if _failed:
            sys.exit(1)
