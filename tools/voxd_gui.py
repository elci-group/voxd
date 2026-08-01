#!/usr/bin/env python3
"""Tkinter settings manager for voxd.

The GUI deliberately uses voxd-cli for config generation and updates. It reads
configuration through `voxd-cli --json config show` and saves changes through
`voxd-cli config set KEY VALUE`, so validation stays in the Rust CLI.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tkinter as tk
from pathlib import Path
from tkinter import messagebox, ttk


CONFIG_FIELDS = [
    ("Server bind", "server.bind"),
    ("Pid file", "server.pid_file"),
    ("ElevenLabs model", "elevenlabs.model_id"),
    ("Output format", "elevenlabs.output_format"),
    ("ElevenLabs API key", "elevenlabs.api_key"),
    ("TTS provider", "providers.tts"),
    ("STT provider", "providers.stt"),
    ("Groq TTS model", "groq.tts_model"),
    ("Groq voice", "groq.voice"),
    ("Groq output format", "groq.output_format"),
    ("Groq sample rate", "groq.sample_rate"),
    ("Groq STT model", "groq.stt_model"),
    ("Groq API key", "groq.api_key"),
    ("System voice id", "system_voice.voice_id"),
    ("System label", "system_voice.label"),
    ("Default stability", "defaults.stability"),
    ("Default similarity", "defaults.similarity_boost"),
    ("Default style", "defaults.style"),
    ("Default speed", "defaults.speed"),
    ("Speaker boost", "defaults.use_speaker_boost"),
    ("Cache dir", "cache.dir"),
    ("Cache enabled", "cache.enabled"),
    ("Cache max MB", "cache.max_mb"),
    ("Wake word", "listen.wake_word"),
    ("Capture device", "listen.device"),
    ("Sample rate", "listen.sample_rate"),
    ("VAD threshold", "listen.vad_threshold"),
    ("VAD noise margin", "listen.vad_noise_margin"),
    ("Min utterance ms", "listen.min_utterance_ms"),
    ("Silence ms", "listen.silence_ms"),
    ("Max utterance secs", "listen.max_utterance_secs"),
    ("Low latency", "listen.low_latency"),
    ("STT model", "listen.stt_model"),
    ("Reply voice", "listen.reply_voice"),
]


class VoxdGui(tk.Tk):
    def __init__(self) -> None:
        super().__init__()
        self.title("voxd settings")
        self.geometry("980x700")
        self.minsize(820, 560)

        self.cli_var = tk.StringVar(value=find_cli())
        self.status_var = tk.StringVar(value="Ready")
        self.fields: dict[str, tk.Variable] = {}
        self.voices: list[dict[str, object]] = []
        self.projects: list[dict[str, object]] = []

        self._build()
        self.after(100, self.load_config)

    def _build(self) -> None:
        root = ttk.Frame(self, padding=10)
        root.pack(fill=tk.BOTH, expand=True)

        top = ttk.Frame(root)
        top.pack(fill=tk.X, pady=(0, 8))
        ttk.Label(top, text="voxd-cli").pack(side=tk.LEFT)
        ttk.Entry(top, textvariable=self.cli_var).pack(side=tk.LEFT, fill=tk.X, expand=True, padx=8)
        ttk.Button(top, text="Init", command=self.init_config).pack(side=tk.LEFT, padx=(0, 4))
        ttk.Button(top, text="Reload", command=self.load_all).pack(side=tk.LEFT, padx=(0, 4))
        ttk.Button(top, text="Save", command=self.save_config).pack(side=tk.LEFT)

        notebook = ttk.Notebook(root)
        notebook.pack(fill=tk.BOTH, expand=True)

        self.settings_frame = ttk.Frame(notebook, padding=10)
        self.projects_frame = ttk.Frame(notebook, padding=10)
        self.daemon_frame = ttk.Frame(notebook, padding=10)
        notebook.add(self.settings_frame, text="Settings")
        notebook.add(self.projects_frame, text="Projects")
        notebook.add(self.daemon_frame, text="Daemon")

        self._build_settings()
        self._build_projects()
        self._build_daemon()

        status = ttk.Label(root, textvariable=self.status_var, anchor=tk.W)
        status.pack(fill=tk.X, pady=(8, 0))

    def _build_settings(self) -> None:
        canvas = tk.Canvas(self.settings_frame, highlightthickness=0)
        scroll = ttk.Scrollbar(self.settings_frame, orient=tk.VERTICAL, command=canvas.yview)
        inner = ttk.Frame(canvas)
        inner.bind("<Configure>", lambda _: canvas.configure(scrollregion=canvas.bbox("all")))
        canvas.create_window((0, 0), window=inner, anchor="nw")
        canvas.configure(yscrollcommand=scroll.set)
        canvas.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
        scroll.pack(side=tk.RIGHT, fill=tk.Y)

        for row, (label, key) in enumerate(CONFIG_FIELDS):
            ttk.Label(inner, text=label).grid(row=row, column=0, sticky=tk.W, padx=(0, 12), pady=4)
            if key.endswith("enabled") or key.endswith("low_latency") or key.endswith("use_speaker_boost"):
                var = tk.BooleanVar()
                widget = ttk.Checkbutton(inner, variable=var)
            else:
                var = tk.StringVar()
                widget = ttk.Entry(
                    inner,
                    textvariable=var,
                    width=58,
                    show="*" if key.endswith("api_key") else "",
                )
            widget.grid(row=row, column=1, sticky=tk.EW, pady=4)
            self.fields[key] = var
        inner.columnconfigure(1, weight=1)

    def _build_projects(self) -> None:
        split = ttk.PanedWindow(self.projects_frame, orient=tk.HORIZONTAL)
        split.pack(fill=tk.BOTH, expand=True)

        left = ttk.Frame(split)
        right = ttk.Frame(split)
        split.add(left, weight=2)
        split.add(right, weight=1)

        self.project_tree = ttk.Treeview(
            left,
            columns=("name", "label", "voice", "path"),
            show="headings",
            selectmode="browse",
        )
        for col, width in [("name", 140), ("label", 130), ("voice", 220), ("path", 260)]:
            self.project_tree.heading(col, text=col.title())
            self.project_tree.column(col, width=width)
        self.project_tree.pack(fill=tk.BOTH, expand=True)

        ttk.Button(left, text="Refresh projects", command=self.load_projects).pack(anchor=tk.E, pady=(8, 0))

        ttk.Label(right, text="Project path or id").pack(anchor=tk.W)
        self.project_var = tk.StringVar(value=".")
        ttk.Entry(right, textvariable=self.project_var).pack(fill=tk.X, pady=(0, 8))

        ttk.Label(right, text="Voice").pack(anchor=tk.W)
        self.voice_var = tk.StringVar()
        self.voice_combo = ttk.Combobox(right, textvariable=self.voice_var)
        self.voice_combo.pack(fill=tk.X, pady=(0, 8))

        ttk.Label(right, text="Label").pack(anchor=tk.W)
        self.label_var = tk.StringVar()
        ttk.Entry(right, textvariable=self.label_var).pack(fill=tk.X, pady=(0, 8))

        ttk.Button(right, text="Assign voice", command=self.assign_voice).pack(fill=tk.X, pady=(4, 4))
        ttk.Button(right, text="Unassign project", command=self.unassign_project).pack(fill=tk.X)
        ttk.Button(right, text="Refresh voices", command=self.load_voices).pack(fill=tk.X, pady=(12, 0))

    def _build_daemon(self) -> None:
        actions = ttk.Frame(self.daemon_frame)
        actions.pack(anchor=tk.NW)
        for text, command in [
            ("Status", self.show_status),
            ("Health", self.show_health),
            ("Start listener", lambda: self.run_text(["listen", "start"])),
            ("Stop listener", lambda: self.run_text(["listen", "stop"])),
            ("Listener status", lambda: self.run_text(["listen", "status"])),
            ("Stop daemon", lambda: self.run_text(["stop"])),
            ("Logs", lambda: self.run_text(["logs"])),
        ]:
            ttk.Button(actions, text=text, command=command).pack(side=tk.LEFT, padx=(0, 6))

        self.output = tk.Text(self.daemon_frame, height=22, wrap=tk.WORD)
        self.output.pack(fill=tk.BOTH, expand=True, pady=(12, 0))

    def cli(self, args: list[str], *, json_out: bool = False) -> str:
        cmd = [self.cli_var.get().strip() or "voxd-cli"]
        if json_out:
            cmd.append("--json")
        cmd.extend(args)
        try:
            proc = subprocess.run(
                cmd,
                check=False,
                text=True,
                capture_output=True,
                env=os.environ.copy(),
            )
        except FileNotFoundError as exc:
            raise RuntimeError(f"Cannot find {cmd[0]}") from exc
        if proc.returncode != 0:
            detail = proc.stderr.strip() or proc.stdout.strip()
            raise RuntimeError(detail or f"{' '.join(cmd)} failed")
        return proc.stdout.strip()

    def load_all(self) -> None:
        self.load_config()
        self.load_voices()
        self.load_projects()

    def init_config(self) -> None:
        try:
            self.cli(["config", "init"], json_out=True)
            self.status_var.set("Config initialized")
            self.load_config()
        except Exception as exc:
            messagebox.showerror("voxd", str(exc))

    def load_config(self) -> None:
        try:
            raw = self.cli(["config", "show"], json_out=True)
            cfg = json.loads(raw)
            for key, var in self.fields.items():
                value = nested_get(cfg, key)
                if isinstance(var, tk.BooleanVar):
                    var.set(bool(value))
                else:
                    var.set("" if value is None else str(value))
            self.status_var.set("Config loaded through voxd-cli")
        except Exception as exc:
            messagebox.showerror("voxd", str(exc))

    def save_config(self) -> None:
        try:
            for key, var in self.fields.items():
                value = var.get()
                if isinstance(var, tk.BooleanVar):
                    value = "true" if value else "false"
                self.cli(["config", "set", key, str(value)], json_out=True)
            self.status_var.set("Config saved through voxd-cli")
        except Exception as exc:
            messagebox.showerror("voxd", str(exc))

    def load_voices(self) -> None:
        try:
            raw = self.cli(["voices"], json_out=True)
            self.voices = json.loads(raw)
            values = [
                f"{v.get('name', '')} | {v.get('voice_id', '')}"
                for v in self.voices
            ]
            self.voice_combo["values"] = values
            if values and not self.voice_var.get():
                self.voice_var.set(values[0])
            self.status_var.set(f"Loaded {len(values)} voices")
        except Exception as exc:
            messagebox.showerror("voxd", str(exc))

    def load_projects(self) -> None:
        try:
            raw = self.cli(["projects"], json_out=True)
            self.projects = json.loads(raw)
            for item in self.project_tree.get_children():
                self.project_tree.delete(item)
            for project in self.projects:
                self.project_tree.insert(
                    "",
                    tk.END,
                    values=(
                        project.get("name", ""),
                        project.get("label", ""),
                        project.get("voice_id", ""),
                        project.get("root_path", ""),
                    ),
                )
            self.status_var.set(f"Loaded {len(self.projects)} projects")
        except Exception as exc:
            messagebox.showerror("voxd", str(exc))

    def assign_voice(self) -> None:
        voice = self.voice_var.get().split("|")[-1].strip()
        if not voice:
            messagebox.showwarning("voxd", "Choose or enter a voice id")
            return
        args = ["assign", self.project_var.get().strip() or ".", voice]
        if self.label_var.get().strip():
            args.extend(["--label", self.label_var.get().strip()])
        self.run_text(args)
        self.load_projects()

    def unassign_project(self) -> None:
        self.run_text(["unassign", self.project_var.get().strip() or "."])
        self.load_projects()

    def show_status(self) -> None:
        self.run_text(["status"])

    def show_health(self) -> None:
        self.run_text(["health"])

    def run_text(self, args: list[str]) -> None:
        try:
            out = self.cli(args)
            self.output.delete("1.0", tk.END)
            self.output.insert(tk.END, out + "\n")
            self.status_var.set("Command completed")
        except Exception as exc:
            messagebox.showerror("voxd", str(exc))


def nested_get(data: dict[str, object], key: str) -> object:
    cur: object = data
    for part in key.split("."):
        if not isinstance(cur, dict):
            return None
        cur = cur.get(part)
    return cur


def find_cli() -> str:
    found = shutil.which("voxd-cli")
    if found:
        return found
    script = Path(__file__).resolve()
    candidates = [
        script.parents[1] / "target" / "debug" / "voxd-cli",
        script.parents[1] / "target" / "release" / "voxd-cli",
    ]
    for candidate in candidates:
        if candidate.exists():
            return str(candidate)
    return "voxd-cli"


if __name__ == "__main__":
    try:
        VoxdGui().mainloop()
    except KeyboardInterrupt:
        sys.exit(130)
