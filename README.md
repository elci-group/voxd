# voxd

A multi-provider text-to-speech and speech-to-speech daemon with ElevenLabs and
Groq support. It allocates and persists a distinct **voice + personality per
project**, and keeps a single **unifying system voice** for general /
conversational responses. Ships as a daemon (`voxd`) plus a thin client
(`voxd-cli`).

- Per-project voice is auto-allocated once and persisted (SQLite), so each repo
  keeps its own sound across restarts.
- The system voice is one fixed voice used with `--system` (or no project) — the
  consistent sound for end-of-task summaries and general replies.
- Synthesized audio is cached by content hash, so identical utterances are free
  on repeat.
- Loopback-only HTTP API protected by a generated bearer token.

## Build & install

```bash
cargo build --release
install -m755 target/release/voxd target/release/voxd-cli ~/.local/bin/
voxd --daemon                 # first run writes ~/.config/voxd/config.toml + token
voxd-cli speak --system "voxd is online."
```

Requires Rust 1.96+ and `ffplay` (ffmpeg) for playback. No systemd needed:
`voxd --daemon` detaches via `setsid`, writes a pidfile, and `voxd-cli stop`
shuts it down. The client also auto-starts the daemon on demand.

## Configuration — `~/.config/voxd/config.toml`

```toml
[server]
bind = "127.0.0.1:17843"
auth_token = "<generated>"
pid_file = "/home/sal/.local/share/voxd/voxd.pid"

[elevenlabs]
model_id = "eleven_multilingual_v2"
output_format = "mp3_44100_128"
# api_key = "..."        # optional; env ELEVENLABS_API_KEY / ~/.bashrc win

[groq]
tts_model = "canopylabs/orpheus-v1-english"
voice = "troy"
output_format = "wav"    # Orpheus currently supports WAV
sample_rate = 48000
stt_model = "whisper-large-v3-turbo"
# api_key = "..."        # optional; env GROQ_API_KEY / ~/.bashrc win

[providers]
tts = "elevenlabs"       # "elevenlabs" or "groq"
stt = "elevenlabs"       # selected independently for the STS loop

[system_voice]
voice_id = "21m00Tcm4TlvDq8ikWAM"   # Rachel — the single unifying voice
label = "system"

[defaults]              # base personality settings
stability = 0.5
similarity_boost = 0.75
style = 0.0
speed = 1.0
use_speaker_boost = true

[pool]
voices = []             # empty -> live list, else built-in premade fallback

[cache]
dir = "/home/sal/.cache/voxd"
enabled = true
max_mb = 512

[recap]
enabled = true
poll_interval_secs = 2
claude_projects_dir = "~/.claude/projects"
```

API key resolution order is provider environment variable → matching `export`
line in `~/.bashrc` → provider config: `ELEVENLABS_API_KEY` /
`[elevenlabs].api_key` and `GROQ_API_KEY` / `[groq].api_key`. Keys are never
logged.

Enable Groq for both ordinary TTS and the complete speech-to-speech loop:

```bash
voxd-cli config set providers.tts groq
voxd-cli config set providers.stt groq
voxd-cli config set groq.voice hannah       # optional
```

Restart the daemon after changing providers. TTS and STT are independent, so a
mixed configuration such as Groq Whisper STT with ElevenLabs TTS is supported.

## Usage

```bash
voxd-cli speak --system "task complete"            # unifying system voice
voxd-cli speak --project . "build done"           # this repo's voice
voxd-cli speak --project . --voice <id> "override once"
voxd-cli voices                                     # list available voices
voxd-cli projects                                   # list bindings
voxd-cli assign . <voice_id> --label "brisk mentor" # pin voice + tone to a project
voxd-cli unassign .                                 # remove a binding
voxd-cli status                                     # uptime, cache, key, counts
voxd-cli logs | voxd-cli stop
```

### Piped dictation

For batch text, pipe to stdin (reads all input before speaking):

```bash
echo "build done" | voxd-cli speak --project .
cat message.txt | voxd-cli speak --system
```

For real-time dictation where each line is spoken as it arrives, use `--stream`:

```bash
echo -e "Line one\nLine two\nLine three" | voxd-cli speak --system --stream
tail -f log.txt | voxd-cli speak --project . --stream --no-play
cat document.txt | voxd-cli speak --system --stream
```

This is useful for live log monitoring, real-time text processing, or streaming text sources where you want immediate audio feedback. The stream mode processes each line independently, respecting all the usual speak options (voice selection, project binding, caching, etc.).

A "personality" is a project row's `{voice_id, label, stability,
similarity_boost, style, use_speaker_boost}` (text is spoken verbatim — no LLM).
`speed` is sent to Groq Orpheus and retained for ElevenLabs compatibility.

## Settings GUI

A dependency-free Tkinter GUI is available for managing config and common daemon
actions without editing TOML directly:

```bash
python3 tools/voxd_gui.py
```

The GUI uses `voxd-cli` under the hood. Config generation and edits go through:

```bash
voxd-cli config init
voxd-cli --json config show
voxd-cli config set defaults.stability 0.55
voxd-cli config set listen.wake_word "hey voxd"
```

It also calls existing CLI commands for status, voices, projects, assignment,
listener controls, logs, and daemon shutdown.

## System tray status widget

`tools/voxd-tray` registers a freedesktop StatusNotifierItem for COSMIC-style
docks and panels. It uses the voxd logo with a traffic-light status dot:

- green — `/health` responds and the daemon is running
- yellow — tray is starting/checking
- red — daemon is not responding

Install the tray helper, icons, desktop launcher, and autostart entry:

```bash
tools/install-tray
```

The widget reads the configured bind address with `voxd-cli --json config show`
and checks `/health` directly, so showing the tray does not auto-start `voxd`.
Run it manually with:

```bash
voxd-tray
```

## Siri-style listening indicator

`tools/voxd-overlay` is a small Python/Tkinter visual helper for the
always-listening loop. Install it as `voxd-overlay` next to the `voxd` binary or
anywhere on `PATH`:

```bash
install -D -m755 tools/voxd-overlay ~/.local/bin/voxd-overlay
```

When `voxd-cli listen start` runs, the daemon looks for `voxd-overlay`, spawns
it if available, and writes simple newline commands to stdin:

```text
listening
triggered
speaking
idle
```

Manual smoke test:

```bash
printf "listening\ntriggered\nspeaking\nidle\n" | voxd-overlay
```

## Project identity

A project is the git repo root of the given path (auto-detected); outside a repo
it falls back to the canonical path. The id is `sha256(root)[:16]`, so voices are
stable per repo regardless of which subdirectory you call from.

## Voice allocation

On first speak for an unseen project, a voice is chosen deterministically:
`index = sha256(project_id) mod pool_len`, linear-probing past voices already in
use (and the system voice). For ElevenLabs, the pool is `[pool].voices` if set,
else the cached/live voice list, else a built-in premade fallback. For Groq, the
pool is the model's published Orpheus voices (or valid entries from
`[pool].voices`). `assign` overrides at any time. Existing incompatible voice
ids fall back to `[groq].voice` when Groq TTS is active.

## HTTP API (127.0.0.1, `Authorization: Bearer <token>`)

| Method | Path | Body | Notes |
|--------|------|------|-------|
| GET | `/health` | – | public, liveness |
| POST | `/speak` | `{text, project_path?, project_id?, system?, voice_id?, label?, play?, no_cache?, settings?}` | returns `{voice_id, label, project_id, cached, chars, audio_path}` |
| GET | `/voices` | – | live list, or built-in fallback |
| GET | `/projects` | – | bindings |
| POST | `/projects/assign` | `{project_id, voice_id, label?, settings?}` | pin a voice |
| POST | `/projects/unassign` | `{project_id}` | remove a binding |
| GET | `/status` | – | uptime, counts, cache, key_present |

> Routes use ids in the request body rather than path parameters (e.g.
> `/projects/assign`) for maximum portability across router versions.

## Always-listening speech-to-speech

`voxd` can run a closed voice loop: it captures the mic continuously, wakes on
**"Hey Voxd"**, transcribes the command with the configured STT provider, routes
it through a built-in intent router, and speaks the reply with the configured
TTS provider.

```
mic -> capture (ffmpeg, s16le/16k) -> RMS VAD -> utterance
     -> STT (Scribe or Groq Whisper) -> wake match -> IntentRouter
     -> TTS (ElevenLabs or Groq Orpheus) -> ffplay
```

```bash
voxd-cli listen start            # start the always-listening loop
voxd-cli listen status           # is it running?
voxd-cli listen stop             # stop it
voxd-cli listen test --secs 3    # record 3s, transcribe, print (mic check)
```

Built-in intents (no LLM, fully local): current **time**, **date**, **uptime**,
**disk** usage, **system specs**, voxd **status**, and **"stop listening"** to
end the loop. Anything else gets a "You said: …" fallback. The router sits
behind a `Responder` trait so an LLM/kimi backend can be added later.

With ElevenLabs TTS, short replies use **streaming TTS** for fast
time-to-first-audio; longer readouts use the batched path. Groq Orpheus replies
use WAV synthesis; inputs over its 200-character limit are split at natural
boundaries and joined into one lossless WAV before playback or caching.

### Config — `[listen]` in `config.toml`

```toml
[listen]
wake_word = "hey voxd"
device = "default"          # Pulse/PipeWire source name or "default"
sample_rate = 16000
vad_threshold = 0.02        # normalized RMS (0..1); raise if noisy
vad_noise_margin = 3.0      # speech must exceed noise_floor * this; floor adapts
min_utterance_ms = 400      # shorter utterances dropped locally, no STT call
silence_ms = 700            # trailing silence that ends an utterance
max_utterance_secs = 12
low_latency = true          # streaming TTS for short replies
stt_model = "scribe_v1"    # used when providers.stt = "elevenlabs"
reply_voice = "system"      # "system" or a voice id
```

The loop runs on its own single-threaded runtime (one OS thread, regardless of
the server's thread pool) and gates everything locally before spending an STT
call: an **adaptive noise floor** learns sustained ambient noise (traffic, fans)
and stops re-triggering on it, and utterances shorter than `min_utterance_ms`
are dropped as clicks/rustles. Only plausible speech reaches Scribe.

### HTTP

| Method | Path | Notes |
|--------|------|-------|
| POST | `/listen/start` | start the loop (409 if already running) |
| POST | `/listen/stop` | stop it |
| GET | `/listen/status` | `{ "listening": bool }` |
| POST | `/listen/transcribe` | raw audio body (`Content-Type: audio/wav` or `audio/mpeg`) → `{ "text": ... }` |

### Live test (needs you at the mic)

```bash
voxd-cli listen start
# say:  "Hey Voxd, what time is it?"   -> hears the time spoken
# say:  "Hey Voxd, system specs"       -> short spec readout
# say:  "Hey Voxd, stop listening"     -> loop ends
```

### Limitations

- Wake detection is **fuzzy text matching** on the transcript (e.g. "Voxd",
  "Vox T", "Voxie" all match), not a dedicated wake engine — tune
  `vad_threshold` / `wake_word` if you see false triggers or misses.
- "Low-latency" speeds up the **output** (streaming TTS); STT is still batched
  per utterance. True end-to-end realtime needs Scribe's streaming websocket
  (out of scope here).
- Capture is suppressed while a reply plays to avoid self-trigger; headphones
  are still recommended.

## Automatic recap narration

The daemon runs a background watcher that speaks harness-generated "recap"
text on its own — no `voxd-cli speak` call, hook, or LLM effort required. A
recap is content the harness already produces to summarize what happened
while you weren't watching; this is a passive delivery channel for it, not a
new kind of summary.

**Claude Code** is the only harness that currently emits one: its `away_summary`
system line (the `/recap` feature, `CLAUDE_CODE_ENABLE_AWAY_SUMMARY`),
appended to `~/.claude/projects/<project>/<session>.jsonl` whenever a session
was unfocused. voxd tails every session file under `[recap].claude_projects_dir`
(default `~/.claude/projects`), and on each new `away_summary` line speaks its
`content` with the project voice for the session's `cwd` (falling back to the
system voice outside a known project). Byte offsets are persisted to
`~/.local/share/voxd/recap_state.json` so restarts never replay old recaps,
and a session file seen for the first time starts from its current end, so
turning the watcher on doesn't dump history.

Other installed harnesses (Codex, Devin, ...) were checked and don't expose an
equivalent passive idle/away artifact today — their session logs record every
turn, not a distinct "you were away" summary — so they still rely on the
manual `voxd-cli speak` skill described below. Adding a harness here means
adding a `poll_*` function in `src/recap.rs`; disable the whole watcher with
`voxd-cli config set recap.enabled false`.

## Mimic (optional TTS caching layer)

When `[mimic].enabled = true` and `providers.tts = "elevenlabs"`, ElevenLabs
synthesis is routed through mimic (a separate, sibling project running as a
`mimicd` daemon): mimic splits the request text into cached/missing spans,
voxd only pays ElevenLabs for the missing spans, and mimic composes the final
audio (`synthesize_with_mimic` in `src/server.rs`). This sits *behind* voxd's
own whole-text content-hash cache (`src/cache.rs`), which short-circuits exact
repeats before mimic is ever consulted. `mimicd` runs a mandatory RAM/storage
admission check (via `pv admit`) before any provider call; a RAM denial
degrades that reply to a text-only desktop notification instead of audio.

Structured tracing for this path (`voxd::mimic` / `voxd::speak` targets) is
always emitted through the normal `tracing` subscriber; set
`VOXD_TRACE_JSONL=<path>` to also append one flat JSON object per event to a
file — useful for tailing plan/admission/compose behavior in production, and
for `tools/mimic_bench.py` below.

### Mimic efficiency benchmark

```bash
python3 tools/mimic_bench.py                 # mock TTS backend, free & repeatable
python3 tools/mimic_bench.py --real-tts       # spend real ElevenLabs credits
python3 tools/mimic_bench.py --passes 3 --out /tmp/report
```

Spins up a throwaway voxd instance against the real, already-running `mimicd`,
drives a built-in corpus shaped like voxd's actual traffic (recap narrations,
`listen` intent replies, CLI-speak acknowledgements) through `/speak`, and
reports the fraction of characters mimic keeps off the ElevenLabs bill — split
into an *end-to-end* view (voxd's own cache included, as in production) and a
*mimic-isolation* view (`no_cache: true`, isolating mimic's own span cache),
each across a cold and a warm pass. It cross-checks its own client-observed
results against the `VOXD_TRACE_JSONL` trace and reports mimic's RAM/storage
admission-denial rate, since a denial silently disables the caching benefit
for that request. Reports land in `tools/mimic_bench_reports/` as JSON +
Markdown. `VOXD_ELEVENLABS_BASE_URL` (the mock-backend seam) and
`VOXD_TRACE_JSONL` are test/benchmark-only env vars — never set them for
normal operation.

## Files

- `~/.config/voxd/config.toml` — config + auth token
- `~/.local/share/voxd/state.db` — SQLite (projects, voice cache, utterance log)
- `~/.local/share/voxd/voxd.pid`, `voxd.log` — pidfile + daemon log
- `~/.local/share/voxd/recap_state.json` — recap-watcher file offsets
- `~/.cache/voxd/*.mp3` — synthesized audio cache

## Codex skill

The `voxd` skill uses `voxd-cli` directly for speech, daemon management, voice
assignment, and listening controls. It calls `voxd-cli speak --system` to read a
short declarative summary aloud at the end of each substantive direct-chat turn.
