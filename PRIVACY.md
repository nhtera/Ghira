# Privacy: what leaves your device

Ghira is built to work offline. This page defines "offline" precisely, so anyone
can check it against the code (all network access lives in `crates/ghi-net`).

## The rules

| Class | What | Rule |
|---|---|---|
| **Your content**: audio, transcripts, notes, voiceprints, meeting titles, people | **Never sent** | Only two exceptions, each an explicit opt-in: **cloud AI** (transcript text only, after you review exactly what will be sent) and **sync with your own paired devices** over your local network (end-to-end encrypted, pinned keys) |
| **Content-free traffic**: model downloads, update check | Only to an **allowlist**. Model downloads: `huggingface.co` and its CDN (`hf.co`). Update check: GitHub Releases (`github.com` and its download host `*.githubusercontent.com`). | **Strict offline** blocks all internet traffic; sync with your paired devices on your private network stays allowed |
| **Telemetry / analytics** | **None** | Crash reports and an event log (no meeting content) are written locally (Settings → About → Diagnostics). You can open and review them, and send them yourself if you choose |
| **Fonts, icons, UI assets** | Bundled with the app | Nothing is loaded from the internet at runtime |

Ghira connects to no other service, and there are no integrations with other
services yet.

## Model downloads

The speech and notes models are not bundled. Ghira downloads them from Hugging
Face, at a pinned revision, and checks each file's SHA-256 before use. Nothing
is sent but the request for the file. Strict offline blocks the download; you
can install the files from a copy instead (see [Speech models](docs/models.md)).

## Update checks

This build has no update feed, so it checks for nothing and sends nothing. Once
signed releases exist, the check will ask GitHub Releases for a public manifest
and nothing else: no ID, no meeting data. You can switch it off (Settings →
About → Check automatically), and Strict offline blocks it.

## What we promise, and how it is checked

1. With **Strict offline** on, recording a 60-minute meeting and processing it
   makes **zero outbound connections**. The release checks measure this with a
   socket audit (`tools/release/net-audit.sh`) and firewall logs (Little Snitch
   on macOS).
2. In the default mode, no network request contains your content. The release
   checks measure this with an intercepting-proxy capture.

## Cloud AI

Cloud AI is off by default and chosen per meeting. You bring your own API key;
Ghira runs no server and never sees your key or data. Before anything is sent, a
preview shows the exact text, with names and other personal details you can
redact. Audio is never sent, and neither are the notes you type yourself. Each
request is recorded on your device (provider, model and token counts, not the
text), and the meeting is marked as having used cloud AI.

A cloud request carries the transcript text and nothing built from your own
settings or habits: it leaves out the moments you marked, your glossary
spellings, and the titles and instructions of your own note templates. If a
meeting uses one of your templates, a cloud rewrite uses the built-in General
template instead and the preview says so.

## Marks, glossaries, templates and saved answers

These features add no network traffic and no new data that leaves your device.

- **Marks** (the moments you mark while recording) and the stars and lists
  Ghira builds from them are worked out on your device each time and stored
  only as part of the meeting, encrypted with it.
- **Glossary packs** are bundled with the app. Which packs are on syncs between
  your own paired devices like other settings; the terms never go anywhere.
- **Your note templates** stay on the computer where you made them (they do not
  sync) and are only used by the model on that computer.
- **Saved Ask answers** are note blocks in the meeting, encrypted like the rest
  of it. Before you save one, the answer waits in memory only. It is cleared
  when the app locks, when its meeting is deleted, when you delete all data, and
  after 30 minutes.

## Voice profiles

Recognizing a speaker by voice uses a voiceprint, which is biometric data. Ghira
only creates one with explicit consent, stores it encrypted on your device, and
deletes it permanently when you ask.

## Calendar

Calendar access is optional and off until you connect it. Ghira reads your local
calendar on your device (EventKit on macOS and iOS, or one ICS file you choose)
only to name a meeting and know who attends. Events are read when needed and
are not stored. Only a meeting you record keeps its event's title and attendees,
encrypted with that meeting. Nothing from your calendar is sent anywhere.

## Sensitive meetings

A meeting marked sensitive keeps no audio: none is written if it is sensitive
from the start, and any audio already written is deleted. It is never sent to
cloud AI and is never used to learn a voice. Only its transcript and notes are
kept, encrypted like every other meeting.
