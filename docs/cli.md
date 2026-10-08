# Command line

The `ghi` command runs Ghira's engines without the app. Use it on files you
already have, to script a job, or to check a setup. It is a developer tool: you
build it yourself, and it has no installer.

## Build it

```sh
cargo build --release -p ghi-cli --features nemo   # target/release/ghi
```

The `nemo` feature adds the speech engines. Without it, `ghi transcribe`,
`diarize` and `bench` stop with “ghi was built without speech engines”. Build the
engines first with `./tools/scripts/build-nemo.sh`, and fetch the models with
`./tools/scripts/fetch-models.sh`. See [Install from source](install.md).

Local notes and answers also need the notes model
(`./tools/scripts/fetch-models.sh qwen3-4b`) and the worker process
(`cargo build -p ghi-llm-worker`).

Commands print JSON on standard output (the streaming ones print one JSON line
per event). The `schema` field names the format, for example
`ghi.transcript/1`. The formats are described in
[Data, CLI and report formats](../tools/eval/docs/formats.md). On failure, a
command prints a `ghi.error/1` document with a `code` and a `message`.

## Commands

```text
$ ghi --help
Headless Ghira CLI for the eval harness and tests

Usage: ghi <COMMAND>

Commands:
  version     Print the versions of ghi, the core and the speech engines
  transcribe  Transcribe an audio file
  diarize     Find who spoke when
  notes       Write meeting notes from a transcript file
  ask         Answer a question about a transcript file, with citations
  keys        Cloud AI provider API keys in the OS keystore
  bench       Run the pipeline on an audio file and report timings
  record      Record the mic (and system audio) to files
  recover     Decode the Ogg Opus tracks in a recording directory, including a
              torn last page after a crash, to `*.recovered.wav`
  store       Inspect and manage an encrypted Ghira store (a data directory)
  sync        LAN sync between two data directories: serve (hub), pair, run (spoke), status
  detect      Show processes using audio and what meeting auto-detect would do
  models      Model manager: hardware tier, installed models, verify, fetch, import
  decode      Probe or decode an audio file with the import decoders
  session     Record a meeting through the core pipeline into the store
  jobs        Run the queued jobs (notes, final pass, import) after crash recovery
  import      Import an audio/video file as a meeting
  help        Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

Run `ghi <command> --help` for every option. The sections below show the main
ones.

### transcribe

```text
Usage: ghi transcribe [OPTIONS] <AUDIO>

Options:
      --lang <LANG>        auto, vi or en [default: auto]
      --pass <PASS>        live or final [default: final]
      --stream             Print caption events as NDJSON while transcribing
      --realtime           Feed audio at 1x speed, as the live app does
      --asr <ASR>          nemo (default) or whisper (needs a build with Whisper)
      --cpu                Run on the CPU instead of the GPU (Metal on macOS)
      --asr-model <FILE>   ASR model GGUF (default: the registry's model)
      --diar-model <FILE>  Diarization model GGUF (default: the registry's model)
```

```sh
ghi transcribe standup.wav --lang vi
```

`ghi diarize <AUDIO>` finds who spoke when (`--max-speakers`, `--pass`), and
`ghi bench <AUDIO>` reports timings. They share the engine options above.

### record

```text
Usage: ghi record [OPTIONS] --out <OUT>

Options:
      --mode <MODE>      call (mic + system audio) or room (mic only) [default: call]
      --out <OUT>        Output directory
      --duration <SECS>  Stop after this many seconds (default: Ctrl-C)
      --format <FORMAT>  wav (default), opus (crash-safe Ogg Opus), or store
                         (a meeting in the encrypted store at --out)
      --title <TITLE>    Meeting title for --format store
      --replay <WAV>...  Play WAV files through the pipeline instead of capturing
      --pid <PIDS>       Capture system audio from these processes only (repeatable)
      --no-aec           Turn echo cancellation off
```

On macOS, a debug build needs to be signed once before it can ask for the
microphone: `codesign -s - -f target/debug/ghi`.

### recover

```text
Usage: ghi recover <DIR>
```

Decodes the Ogg Opus tracks in a recording folder, including a last page torn by
a crash, to `*.recovered.wav`.

### detect

```text
Usage: ghi detect [OPTIONS]

Options:
      --watch <SECS>  Poll every second for this many seconds; print one line per prompt
```

Shows which processes use audio and what meeting detection would do.

### import

```text
Usage: ghi import [OPTIONS] --dir <DIR> [FILE]

Arguments:
  [FILE]  An audio or video file (WAV, MP3, M4A/AAC, FLAC, Ogg, Opus, MP4, ...)

Options:
      --dir <DIR>             The data directory (the encrypted store)
      --tracks <FILE_OR_DIR>...  One recording from several participants' own
                              tracks (Zoom "record a separate audio file for
                              each participant"): the files, or a Zoom meeting
                              folder. Speakers are named from the file names.
      --split-channels        Keep the first two channels as separate tracks
      --lang <LANG>           auto, vi or en [default: auto]
      --title <TITLE>
      --process               Run the final pass and notes right away
```

### notes

```text
Usage: ghi notes [OPTIONS] <TRANSCRIPT>

Options:
      --lang <LANG>               Language of the notes: auto, vi or en [default: auto]
      --template <TEMPLATE>       general, one_on_one, standup, sales, interview,
                                  client or lecture [default: general]
      --template-file <FILE>      A custom template (TOML)
      --user-notes <FILE>         Notes you typed, one per line ("[mm:ss] " prefix
                                  = when typed), to expand
      --provider <PROVIDER>       local (default, offline), openai, anthropic or gemini
      --cloud-model <MODEL>       The cloud model (required with a cloud provider)
      --preview                   Print the exact request and send nothing
      --confirm-send <SHA256>     Send, if the payload's SHA-256 matches --preview
      --redact-names <NAMES>      More names to hide, comma-separated
      --strict-offline            Block all internet traffic
```

With a cloud provider, run `--preview` first. It prints the exact request and
its SHA-256. Pass that value to `--confirm-send` to send it. Nothing is sent
without it.

### ask

```text
Usage: ghi ask [OPTIONS] <TRANSCRIPT> <QUESTION>
```

Answers a question about a transcript file, with citations. It takes the same
`--lang`, `--provider`, `--cloud-model`, `--preview`, `--confirm-send`,
`--redact-names` and `--strict-offline` options as `notes`.

### keys

```text
Usage: ghi keys <COMMAND>

Commands:
  set     Store a provider's API key, read from the first line of stdin
  delete  Remove a provider's API key
  status  Which providers have a key stored (never the key itself)
```

```sh
ghi keys set anthropic < key.txt
```

### store

```text
Usage: ghi store --dir <DIR> <COMMAND>

Commands:
  list            List meetings and their audio tracks
  search          Accent-insensitive search over transcripts and notes
  audio           Decrypt one audio track of a meeting to a WAV file
  add-transcript  Store a transcript file as a meeting's transcript
  notes           Write (or regenerate) a meeting's notes from its transcript;
                  pinned, user-written, edited and done items are kept
  delete          Delete a meeting: its key is destroyed first, so nothing stays readable
  export          Export everything into one archive encrypted with a password read from stdin
  import          Restore an export into the (empty) --dir; password from stdin
```

`--dir` is the data directory. It is created on first use. A debug build keeps
the store's key in a file next to it. A release build uses the Keychain.

### models

```text
Usage: ghi models <COMMAND>

Commands:
  status  Hardware tier and which pinned models are installed (--verify checks SHA-256)
  verify  Check an installed model's SHA-256 (`all` for every one)
  fetch   Download a pinned model (--strict-offline refuses before any connection)
  import  Install a model from a local file; its SHA-256 must be a pinned model's
```

Without `--dir`, the models folder is `$GHI_MODELS_DIR`, else `./models`. See
[Speech models](models.md).

### session

```text
Usage: ghi session [OPTIONS] --dir <DIR>

Options:
      --dir <DIR>        The data directory (the encrypted store)
      --mode <MODE>      call or room [default: room]
      --replay <WAV>...  Replay WAV files instead of capturing
      --lang <LANG>      auto, vi or en [default: auto]
      --speed <SPEED>    Replay speed: 1 = real time, 0 = as fast as possible [default: 1]
      --duration <SECS>  Live capture: stop after this many seconds (default: Ctrl-C)
      --process          Run the notes and final-pass jobs right after stop
      --record-only      Record without speech engines, as the app does while
                         the models are missing
```

Records a meeting through the same pipeline the app uses and stores it. It
prints one line per event and a summary.

### jobs

```text
Usage: ghi jobs [OPTIONS] --dir <DIR>
```

Runs the queued jobs (notes, final pass, import) after crash recovery. It takes
the engine options of `transcribe` and `--model` for the notes model.

### sync

```text
Usage: ghi sync <COMMAND>

Commands:
  serve   Hub: listen on the private LAN addresses, advertise over mDNS and open a
          pairing window; serves sessions until Ctrl-C
  pair    Spoke: pair with a hub from its pairing code
  run     Spoke: run one session with the paired hub
  status  Paired devices and sync cursors; no secrets
  export  Write a passphrase-sealed file with meetings, their audio and keys
  import  Merge a sealed export file into this store; no pairing needed
```

Each command takes `--dir`, the data directory. `serve` takes `--bind`, `--port`,
`--name`, `--print-qr` and `--no-pair`. `pair` takes `--qr` or `--qr-file`.
Sync stays on private network addresses.

### Other commands

- `ghi version` prints the versions of `ghi`, the core and the speech engines.
- `ghi decode <AUDIO>` probes a file (`--probe`) or writes it as 16 kHz mono WAV
  (`--wav <OUT.WAV>`, `--channel <N>`).

## Limits

- The CLI is for developers and tests. Its options can change between
  pre-release builds.
- Speech commands need a build with `--features nemo` and the models.
- Whisper (`--asr whisper`) needs a build with the `whisper` feature. See
  [Speech models](models.md).
