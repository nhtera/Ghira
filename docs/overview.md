# Overview

Ghira is a meeting note taker that works offline. It records a call or a
meeting in the room, shows a live transcript in English, Vietnamese or both, and
tells the speakers apart as they talk. When you stop, it writes notes on your
Mac. Audio, transcripts and notes stay on your device.

## What it does

- **Record.** Capture a call (your microphone plus the meeting app's audio) or a
  room (your microphone). The transcript appears as people talk, and each voice
  gets a color and an initial.
- **Write notes.** A model on your Mac writes a summary, decisions, action items
  with owners and open questions. Each sentence links to the moment it was said.
  What you typed during the meeting is kept and expanded.
- **Review.** Search every meeting, with or without accents. Play any cited
  moment, correct the transcript, rename speakers, and export to Markdown, Word,
  text, subtitles or Obsidian. Ghira can also draft a follow-up email.
- **Import.** Bring in recordings from Voice Memos, Zoom, Plaud, or any audio or
  video file.
- **Keep it private.** Meetings live in an encrypted local database with a key
  per meeting. Deleting a meeting destroys its key.
- **Use cloud AI, if you choose.** It is off by default and chosen per meeting.
  You see the exact text before anything is sent, names can be hidden, and audio
  never leaves your device.

## What is shipped, and what is not

Ghira is **pre-release**.

- The Mac app (Apple Silicon, macOS 14.2 or later) is the main target. There is
  no signed download yet. You [build it from source](install.md).
- The iPhone app is built and tested on the Simulator. Runs on a physical
  iPhone are still being tested.
- Windows is not shipped, and there is no Android app.
- The app does not update itself yet. See [Privacy](../PRIVACY.md) for what it
  does and does not send over the network.

## Where to go next

1. [Install from source](install.md) builds the app.
2. [Record your first meeting](getting-started.md) walks you through setup and
   a first recording.
3. [Speech models](models.md) explains what Ghira downloads and why.
4. [Privacy](../PRIVACY.md) defines exactly what may leave your device.
5. [Command line](cli.md) covers the `ghi` command for files you already have.
