# voxd

An enterprise-style ElevenLabs text-to-speech daemon. It allocates and persists a
distinct **voice + personality per project**, and keeps a single **unifying system
voice** for general / conversational responses. Ships as a daemon (`voxd`) plus a
thin client (`voxd-cli`).

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
```

API key resolution order: `ELEVENLABS_API_KEY` env → `export` line in
`~/.bashrc` → `[elevenlabs].api_key`. The key is never logged.

## Usage

```bash
voxd-cli speak --system "task complete"            # unifying system voice
echo "build done" | voxd-cli speak --project .     # this repo's voice
voxd-cli speak --project . --voice <id> "override once"
voxd-cli voices                                     # list available voices
voxd-cli projects                                   # list bindings
voxd-cli assign . <voice_id> --label "brisk mentor" # pin voice + tone to a project
voxd-cli unassign .                                 # remove a binding
voxd-cli status                                     # uptime, cache, key, counts
voxd-cli logs | voxd-cli stop
```

A "personality" is a project row's `{voice_id, label, stability,
similarity_boost, style, use_speaker_boost}` (text is spoken verbatim — no LLM).
`speed` is stored for forward compatibility and is not sent on the wire.

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
use (and the system voice). The pool is `[pool].voices` if set, else the cached
ElevenLabs voice list, else — for restricted keys without `voices_read` — a
built-in set of canonical premade voices. `assign` overrides at any time.

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
**"Hey Voxd"**, transcribes the command with ElevenLabs Scribe, routes it
through a built-in intent router, and speaks the reply on the unifying voice.

```
mic -> capture (ffmpeg, s16le/16k) -> RMS VAD -> utterance
     -> STT (Scribe) -> wake match -> IntentRouter -> TTS -> ffplay
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

Latency modes (per ElevenLabs): short replies use **streaming TTS**
(`/text-to-speech/.../stream` piped to ffplay) for fast time-to-first-audio;
longer readouts use the **batched STT+TTS** path (synthesize → cache → play).

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
stt_model = "scribe_v1"
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

## Files

- `~/.config/voxd/config.toml` — config + auth token
- `~/.local/share/voxd/state.db` — SQLite (projects, voice cache, utterance log)
- `~/.local/share/voxd/voxd.pid`, `voxd.log` — pidfile + daemon log
- `~/.cache/voxd/*.mp3` — synthesized audio cache

## Codex skill

The `voxd` skill uses `voxd-cli` directly for speech, daemon management, voice
assignment, and listening controls. It calls `voxd-cli speak --system` to read a
short declarative summary aloud at the end of each substantive direct-chat turn.
