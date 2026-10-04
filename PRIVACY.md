# Privacy: what leaves your device

Ghira is built to work offline. This page defines "offline" precisely, so anyone
can check it against the code (all network access lives in `crates/ghi-net`).

## The rules

| Class | What | Rule |
|---|---|---|
| **Your content**: audio, transcripts, notes, voiceprints, meeting titles, people | **Never sent** | Only three exceptions, each an explicit opt-in: **cloud AI** (transcript text only, after you review exactly what will be sent), **sync with your own paired devices** over your local network (end-to-end encrypted, pinned keys), and integrations you choose to connect |
| **Content-free traffic**: model downloads, update check | Only to an **allowlist**: `huggingface.co` and its CDN, the Ghira model mirror, the Ghira update feed (GitHub Releases: `github.com` and its download host `*.githubusercontent.com`). The update check sends nothing but the request for the public manifest: no ID, no version in the URL | You can switch each one off (Settings → About → Check automatically). **Strict offline** blocks all internet traffic; sync with your paired devices on your private network stays allowed |
| **Telemetry / analytics** | **None** | Crash reports and an event log (no meeting content) are written locally (Settings → About → Diagnostics). You can open, review and send them yourself |
| **Fonts, icons, UI assets** | Bundled with the app | Nothing is loaded from the internet at runtime |

## What we promise, and how it is tested

1. With **Strict offline** on, recording a 60-minute meeting and processing it
   makes **zero outbound connections**. We verify this with firewall logs
   (Little Snitch on macOS, Windows Firewall).
2. In the default mode, no network request contains your content. We verify this
   with a proxy capture test.

## Cloud AI

Cloud AI is off by default and chosen per meeting. You bring your own API key;
Ghira runs no server and never sees your key or data. Before anything is sent, a
preview shows the exact text, with names and other personal details you can
redact. Audio is never sent, and neither are the notes you type yourself. Each
request is recorded on your device (provider, model and token counts, not the
text), and the meeting is marked as having used cloud AI.

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
