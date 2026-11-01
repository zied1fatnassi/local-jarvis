# JARVIS Personal Desktop AI Assistant — Daily Usage & Operational Guide

> **Operating System:** Linux Mint 22.3 Zena (Cinnamon Edition, X11)  
> **Hardware Profile:** AMD Ryzen 7 7435HS (8C/16T) · NVIDIA RTX 3050 Laptop (4GB VRAM) · 32GB DDR5  
> **Persona & Voice:** British Butler ("Sir") · Kokoro-82M (`bm_george` @ 24 kHz) · Whisper Small (`ggml-small.bin`) · openWakeWord (`hey_jarvis`)  

---

## 1. Overview & System Architecture

Local Jarvis is a fully autonomous, local-first voice assistant running directly on Linux Mint. It features real-time wake word detection, streaming text-to-speech, local neural speech-to-text, on-device large language model reasoning, and 18 native desktop system skills with built-in safety controls.

```
                  ┌────────────────────────┐
                  │   Microphone (PipeWire) │
                  └───────────┬────────────┘
                              │
                    [openWakeWord Engine]
                      "Hey Jarvis" (0.70 threshold)
                              │
                     [Whisper Small STT]
                    English Speech-to-Text
                              │
               ┌──────────────┴──────────────┐
               │    Local Qwen 2.5-3B LLM     │  ◄── (Optional Cloud Fallback)
               │ (llama.cpp multi-thread CPU)│      Gemini / OpenAI API
               └──────────────┬──────────────┘
                              │ Tool Calls & Safety Checks
             ┌────────────────┼────────────────┐
             ▼                ▼                ▼
     [Desktop Skills]   [SQLite Memory]   [Kokoro-82M TTS]
     18 Native Tools   Hydrated History   British George Voice
     (Volume, Apps...) (memory.sqlite)    (Streaming Audio)
```

---

## 2. Interaction Modalities

### A. Hands-Free Voice ("Hey Jarvis")
Simply speak **"Hey Jarvis"** followed by your query or command.
1. The assistant plays an audio chime (ding) indicating it is listening.
2. Speak your request naturally in English.
3. Silence is automatically detected via Energy VAD.
4. Jarvis processes the request, invokes necessary system tools, and replies aloud in British English.

*Example:*  
> **You:** "Hey Jarvis, what is my battery level and system temperature?"  
> **Jarvis:** "Your battery is currently discharging at eighty-three percent, and the graphics processor is operating at forty-two degrees Celsius, Sir."

### B. Global Desktop Hotkey (`<Super> + J`)
A global Cinnamon desktop keybinding is permanently registered:
- Press **`Super + J`** (Windows key + J) anywhere on your desktop.
- Jarvis immediately enters listening mode without requiring the wake word.
- Speak your command directly.

### C. Command Line Interface (CLI)
Interact with the running daemon or trigger actions from terminal scripts:

| Command | Description |
| :--- | :--- |
| `jarvis ask "<prompt>"` | Send any text query to the running Jarvis daemon and speak the answer |
| `jarvis listen` | Trigger a single voice listening cycle directly |
| `jarvis say "<text>"` | Synthesize and speak text aloud through Kokoro TTS |
| `jarvis cancel` | **Emergency stop**: immediately halts any current audio playback |
| `jarvis skill <name> --args '<json>'` | Directly test or execute any of the 18 native skills |

### D. Live Terminal Dashboard (TUI)
Run the real-time interactive terminal user interface:
```bash
jarvis tui
```
- **Tab 1 (Session):** Live conversation history and interactive prompt box.
- **Tab 2 (Journal):** Real-time daemon log stream and latency metrics.
- **Tab 3 (Tools):** Visual inspector for tool invocations and execution results.
- Press **`Esc`** or **`q`** to exit the TUI without stopping the background assistant daemon.

---

## 3. Available Skills & Voice Commands

Jarvis is equipped with 18 native desktop and system automation tools:

### 1. Date & Time (`datetime`)
* **"What time is it?"**  
  *Returns:* Current local time (e.g., "It is currently twenty-two minutes past one in the afternoon, Sir.")
* **"What day is it today?"**  
  *Returns:* Full date, day of week, and timezone.

### 2. Hardware & System Metrics (`system_stats`)
* **"How is the system doing?"**
* **"Check CPU and RAM usage."**
* **"What is my GPU temperature and battery status?"**  
  *Returns:* Live CPU load averages, RAM used/total, NVIDIA RTX 3050 VRAM utilization and temperature, and battery charge percentage/state.

### 3. Audio & Volume Control (`volume`)
* **"Set volume to 50 percent."**
* **"Turn the volume up by 10 percent."**
* **"Mute the audio."** / **"Unmute volume."**

### 4. Screen Brightness (`brightness`)
* **"Set brightness to 75 percent."**
* **"Increase screen brightness."** / **"Dim the display."**

### 5. Application Launching (`launch_app`)
* **"Open calculator."**
* **"Launch terminal."**
* **"Open Google Chrome."**
* **"Open text editor."**

### 6. Window Management (`list_windows`, `focus_window`)
* **"What windows are currently open?"**  
  *Returns list of active desktop application windows.*
* **"Focus Chrome."** / **"Switch to terminal."**  
  *Brings the specified window to the foreground.*

### 7. Safety-Gated Window Closure (`close_app`)
*Destructive actions require confirmation to prevent accidental loss of unsaved work.*
* **Step 1:** "Close calculator."  
  *Jarvis:* "Are you sure you want to close the calculator window, Sir?"
* **Step 2:** "Yes, please close it."  
  *Jarvis closes the window and reports completion.*

### 8. Media Playback Control (`media_*`)
* **"Pause music."** / **"Play music."**
* **"Next track."** / **"Previous track."**
* **"Stop media."**
* **"What song is currently playing?"**

### 9. Web & File Search (`web_search`, `file_search`)
* **"Search the web for Rust 1.85 release notes."**  
  *Opens results in your default web browser.*
* **"Find PDF files in Documents."**  
  *Safe search restricted to non-sensitive user directories (blocking `.ssh`, `.gnupg`, etc.).*

### 10. Desktop Security & Utilities (`lock_screen`, `screenshot`, `notify`)
* **"Lock my screen."**  
  *Locks the Cinnamon desktop session immediately.*
* **"Take a screenshot."**  
  *Captures screen and saves image to `~/Pictures/Screenshots/`.*
* **"Send a notification 'Meeting in 10 minutes'."**  
  *Triggers a native Cinnamon desktop notification bubble.*

---

## 4. Emergency Voice & CLI Abort

If Jarvis is speaking and you need silence immediately:
1. **Voice Abort:** Say any of the following wake words:
   - **"Stop"**
   - **"Cancel"**
   - **"Quiet"**
   - **"Shut up"**
2. **Terminal / Shortcut Abort:**
   ```bash
   jarvis cancel
   ```
Audio playback is cancelled instantly, and the daemon returns to the idle state.

---

## 5. Cloud Fallback & Hybrid Routing (Optional)

By default, Jarvis runs 100% locally on your machine. If you wish to enable high-speed cloud fallback for deep analytical queries:

1. Obtain a Google Gemini API Key or OpenAI API Key.
2. Add your key to your user environment (`~/.bashrc` or `~/.profile`):
   ```bash
   export GEMINI_API_KEY="your-api-key-here"
   ```
3. Edit `/home/zied/.config/jarvis/config.toml`:
   ```toml
   [llm]
   engine = "hybrid"             # "local", "cloud", or "hybrid"
   cloud_provider = "gemini"     # "gemini" or "openai"
   cloud_model = "gemini-2.5-flash"
   cloud_api_key = "your-api-key-here" # or leave blank to use GEMINI_API_KEY env
   ```
4. Restart the service:
   ```bash
   systemctl --user restart jarvis
   ```
In hybrid mode, Jarvis attempts local inference first; if a timeout or complex query occurs, it seamlessly queries the cloud API while maintaining the British voice output.

---

## 6. Service Management & Maintenance

Jarvis runs as a standard `systemd` user service that automatically starts on login.

### Service Commands:
```bash
# Check service status and memory usage
systemctl --user status jarvis

# Restart the assistant daemon
systemctl --user restart jarvis

# Stop the daemon
systemctl --user stop jarvis

# Start the daemon
systemctl --user start jarvis

# View live system logs
journalctl --user -u jarvis -f

# View recent latency benchmarks
journalctl --user -u jarvis -g "latency" -n 20 --no-pager
```

### File Locations:
- **Configuration:** `~/.config/jarvis/config.toml`
- **Systemd Unit:** `~/.config/systemd/user/jarvis.service`
- **Binary:** `/home/zied/.cargo/bin/jarvis`
- **Conversation Memory Database:** `~/.local/share/jarvis/memory.sqlite`
- **Model Storage:** `~/.local/share/jarvis/models/`
  - `qwen2.5-3b-instruct-q4_k_m.gguf` (2.0 GB)
  - `ggml-small.bin` (466 MB)
  - `hey_jarvis_v0.1.onnx`
  - `kokoro-v1.0.onnx` & `voices/bm_george.bin`

---

## 7. Troubleshooting & Audio Calibration

### Microphone Sensitivity
If Jarvis occasionally misses the wake word or transcribes faint background audio:
```bash
# Calibrate default microphone volume to recommended 85%
wpctl set-volume @DEFAULT_AUDIO_SOURCE@ 0.85
```

### Audio Stutter or PipeWire Issues
If audio sinks become unavailable after sleep/wake:
```bash
systemctl --user restart pipewire wireplumber jarvis
```

### Reset Conversation History
If you wish to clear past conversational memory:
```bash
rm ~/.local/share/jarvis/memory.sqlite
systemctl --user restart jarvis
```
