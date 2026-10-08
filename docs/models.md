# Speech models

Ghira does its work with models that run on your Mac: one that turns speech into
text, one that tells speakers apart, one that writes notes, and a few helpers.
The models are not bundled with the app. Ghira downloads them once, checks them,
and then works offline.

## Presets

Ghira picks a preset from your Mac's memory. You see it in the setup and in
**Settings → Models → Speed and quality**.

| Preset | Memory | What changes |
|---|---|---|
| **Light** | 8 GB | No search-by-meaning model. Ghira does not write notes while a recording runs. |
| **Balanced** | 16 to 24 GB | Adds the search-by-meaning model. |
| **Max** | 32 GB or more | Same models as Balanced, with more memory set aside for them. |

You cannot pick a different preset yet. Every preset uses the same speech,
speaker and notes models.

## What is downloaded

| What it does | Model | Size | Preset |
|---|---|---|---|
| Speech to text | Nemotron 3.5 ASR | 742 MB | All |
| Who said what | Nemotron 3 Diarization | 107 MB | All |
| Meeting notes | Qwen3 4B | 2.5 GB | All |
| Search by meaning | Qwen3 Embedding 0.6B | 639 MB | Balanced, Max |
| Recognising voices | CAM++ | 28 MB | All |

That is about 3.4 GB on Light and about 4 GB on Balanced and Max. The exact
sizes, licenses and download sources are in `crates/ghi-models/registry.toml`.

Until the speech models are installed, Ghira still records. It keeps the audio
and makes the transcript and notes once the models arrive. Without the voice
model you can use Ghira, but it cannot recognise voices.

## Where the models come from

- **Hugging Face only.** Ghira downloads models from `huggingface.co` and its
  download hosts (`hf.co`). It refuses any other host.
- **Pinned and checked.** Each file is fetched at a fixed revision and must match
  its SHA-256 in the registry. A file that fails its check is not used. The
  model list shows “This model failed its check” and a button to download it
  again.
- **Resumable.** If the network drops, Ghira keeps what it has and resumes from
  there.

Only the request for the file goes over the network, never your content. See
[Privacy](../PRIVACY.md).

## Strict offline

Turn on **Strict offline** in **Settings → Privacy** to block all internet
traffic. Nothing is downloaded while it is on. Record as usual, and Ghira
processes the recording once the models are installed.

## Install models without a network

Use this on a Mac that cannot reach the internet.

1. **Fetch the files on a computer with internet.** From a copy of the
   repository, run `./tools/scripts/fetch-models.sh`. Name any optional models
   you want, such as `whisper-large-v3-turbo silero-vad`.
2. **Make a package.** Run `tools/release/offline-models.sh`. It checks every file
   against the registry and writes `ghira-models-<date>.tar` with the files and
   their checksums. Check a package with `--check <file>.tar`.
3. **Move it to the Mac and unpack it.** Copy the tar with AirDrop or a USB drive.
4. **Import each file.** Build the [CLI](cli.md), then run
   `ghi models import --dir <models folder> <file>` for each file in `models/`.
   Ghira's models folder is `models` inside its data folder, which is
   `~/Library/Application Support/com.nhtera.ghira`. The importer refuses any file
   that is not a pinned model.
5. **Check the result.** `ghi models status --dir <models folder> --verify` lists
   what is installed and verified.

The app has no “Install from a file” button yet.

## Better transcripts after the meeting (optional)

By default, Ghira writes the transcript after a meeting with its standard
speech model. The setting **Transcript after the meeting** in
**Settings → Models** also offers **High accuracy (Whisper)**:

- It is about 1.6 times slower, and it is a download of about 575 MB (Whisper
  large-v3-turbo and a voice-activity model).
- It is often more accurate on clear speech. Where it cannot make out the words,
  the standard model fills in.
- Until Whisper is downloaded, Ghira uses **Standard**, which is fast and
  recommended for phone-quality audio and mixed English and Vietnamese.

The setting shows only in builds that include Whisper. The default build from
source does not. Whisper needs `./tools/scripts/build-whisper.sh` and
`--features nemo,whisper` when you build the desktop app.

## Limits

- Download sources are fixed to Hugging Face. There is no mirror.
- You cannot pick a different notes model.
