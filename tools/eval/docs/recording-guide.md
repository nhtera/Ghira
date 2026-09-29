# Recording guide

This guide is for the person who plans and runs the recordings. You need about **10 hours of real meetings**, recorded with consent, in the mix below. Take your time with the first two recordings; they are the practice ones.

Nobody outside your company hears these recordings. The Ghira team only receives a report of aggregate numbers (see `runbook.md`).

## 1. What to record

**Total: at least 10 hours.** Suggested plan (hours of audio):

| | Room (laptop mic) | Call (online) | Total | Share |
|---|---|---|---|---|
| Vietnamese (`vi`) | 2.0 | 2.0 | 4.0 | ~40% |
| English (`en`) | 1.5 | 1.5 | 3.0 | ~30% |
| Mixed, Vietnamese with English words or switching mid-sentence (`mixed`) | 1.5 | 1.5 | 3.0 | ~30% |

Also cover:

- **Room:** one laptop on the table, its built-in microphone, 2 to 6 people in the room.
- **Call:** 2 to 8 people. Half of the call hours with the local participant on **headphones**, half on **speakers**.
- **Big meetings:** 3 to 5 recordings with **6 to 8 different speakers** (they can be rooms or calls).
- **Real meetings**, not read scripts. Normal pace, people interrupting each other, some overlap. Do not ask people to speak more clearly than usual.
- **Length:** 20 to 60 minutes per recording. About 15 to 30 recordings in total.
- **Different people and topics.** Avoid ten recordings of the same five people.

"Mixed" means the meeting itself switches between Vietnamese and English (for example Vietnamese with terms like "deadline", "scope", "release"). Label it `mixed` when English is more than a few loanwords.

## 2. Consent comes first

- Every person who speaks must have signed the consent form (`consent-form-vi.md` or `consent-form-en.md`) **before** the recording starts.
- At the start of each meeting say aloud: "This meeting is being recorded for our internal speech-software test. Tell me if you do not want to be recorded."
- If someone has not signed or says no, do not record them. Pause the recording while they speak, or do not record that meeting.
- Someone who joins late must sign before you continue. Someone who withdraws later: see `runbook.md`, "Deletion".
- Keep the signed forms with the data contact, not with the recordings.
- Pick a code for each person, such as `P01`, `P02`. Keep the list "code to name" in a separate file on the encrypted volume. You will need the codes in `manifest.yaml`.

## 3. Equipment and settings

Save the recording as **WAV, PCM 16-bit**, **16 kHz or 48 kHz**. Mono is fine. Do not use MP3 or M4A.

Record **on the encrypted volume** (see `runbook.md`, step 2), or move the files there right after and delete the originals.

### Room recording (one microphone)

1. Install **Audacity** (free, open source, https://www.audacityteam.org).
2. Set the device to the laptop's built-in microphone, **Recording Channels: 1 (Mono)**, and **Project Rate: 16000 Hz** (or 48000).
3. Put the laptop in the middle of the table, lid open, microphone not covered. Do not put it on a soft surface.
4. Press record, check the level bar moves when people speak (peaks around the middle, not touching the right edge).
5. When the meeting ends: File > Export > Export as WAV, encoding **Signed 16-bit PCM**.

On macOS the first recording asks for microphone permission. Allow it.

### Call recording (online meeting: Zoom, Teams, Meet, ...)

Best result: **two tracks**, one with your microphone and one with everything you hear from the call (`system`), plus a mix of both. If you cannot do two tracks, record one mixed track; that is accepted.

Free tool that works on macOS and Windows: **OBS Studio** (free, open source, GPL-2.0; https://obsproject.com). Use OBS 30 or newer.

1. Add sources in one scene:
   - Your microphone: **Audio Input Capture**.
   - The call audio: on **Windows** **Audio Output Capture**; on **macOS 13 or newer** **macOS Audio Capture** (OBS asks for the Screen & System Audio Recording permission).
2. Settings > Output > Output Mode **Advanced** > Recording. Recording format **mkv** (safe if it crashes). Audio Track: tick **1, 2 and 3**.
3. Edit > Advanced Audio Properties. Under **Tracks**:
   - microphone: tracks **1 and 2** only,
   - call audio: tracks **1 and 3** only.
   - Track 1 is the mix. Track 2 is only you. Track 3 is only the others.
4. Settings > Audio: Sample Rate **48 kHz**, Channels **Mono** (or Stereo, both work).
5. Start Recording before the call starts. Say something, and check that both meters move.
6. After the meeting, split the tracks with **FFmpeg**. FFmpeg is a separate free program that you install yourself (macOS `brew install ffmpeg`; Windows `winget install Gyan.FFmpeg`; its licence is LGPL/GPL, and we do not ship it). Run:

   ```
   ffmpeg -i rec.mkv -map 0:a:0 -c:a pcm_s16le -ac 1 m001.wav ^
                     -map 0:a:1 -c:a pcm_s16le -ac 1 m001.mic.wav ^
                     -map 0:a:2 -c:a pcm_s16le -ac 1 m001.system.wav
   ```

   (On macOS/Linux, end the lines with `\` instead of `^`.) Put the three files in `audio/` under your dataset.

**Headphones or speakers matters.** With speakers, the call audio leaks into your microphone. Do not fix that yourself (no echo cancellation tricks in OBS). The test needs the real situation. Write the truth in `playback` in the manifest.

If you cannot use OBS, alternatives: BlackHole (macOS, free, GPL-3.0, needs an audio routing setup) or Loopback (macOS, paid, from Rogue Amoeba). Zoom's local option "Record a separate audio file for each participant" also gives clean tracks, but they do not match the `mic` / `system` layout, so use it only if you also keep a mixed recording.

### After every recording

- Play 30 seconds from the middle. Can you hear everyone? Is there a loud hum or clipping?
- If a recording is broken, record another one. Do not fix audio with filters.

## 4. File names and folder

Use a neutral id. **No names, no company names, no dates with meaning** in the file name. Use `m001`, `m002`, ...

```
customer-2026q4/
  manifest.yaml
  audio/m001.wav
  audio/m002.wav
  audio/m002.mic.wav        # only for calls with two tracks
  audio/m002.system.wav
  labels/                   # filled in by the labelling step
  refs/
  notes/                    # reference notes for 5-10 meetings
```

`audio/<id>.wav` is always the mix. The two extra tracks are optional.

## 5. Fill in `manifest.yaml`

Create it in the dataset folder. One entry per recording. Example:

```yaml
version: 1
name: customer-2026q4
# People, companies and projects that may be spoken in the meetings.
# The report checker uses this list to make sure no name appears in a report.
# The list itself never leaves the machine.
names: [Linh, Minh, Hùng, "Công ty ABC", Orion]
files:
  - id: m001
    audio: audio/m001.wav
    rttm: labels/m001.rttm          # created by the labelling step
    ref: refs/m001.txt              # created by the labelling step
    lang: vi
    setting: room
    playback: na
    speakers: 4
    persons: {spk1: P01, spk2: P02, spk3: P03, spk4: P04}

  - id: m002
    audio: audio/m002.wav
    tracks: {mic: audio/m002.mic.wav, system: audio/m002.system.wav}
    rttm: labels/m002.rttm
    ref: refs/m002.txt
    notes_ref: notes/m002.yaml      # only for the 5-10 meetings with reference notes
    lang: mixed
    setting: call
    playback: headphones
    speakers: 6
    persons: {spk1: P01, spk2: P05, spk3: P06, spk4: P07, spk5: P08, spk6: P09}
```

| Field | Values and meaning |
|---|---|
| `id` | Letters, digits, `_` and `-`. Unique. Same as the file name. |
| `lang` | `vi`, `en` or `mixed` (see section 1). |
| `setting` | `room` (people in one room, one microphone) or `call` (online meeting). |
| `playback` | `headphones` or `speakers` for calls. `na` for rooms. |
| `speakers` | How many different people spoke. Count everyone who said more than a word. |
| `persons` | Optional but useful. Maps the speaker names you use when labelling (`spk1`, `spk2`, ...) to person codes (`P01`, ...). Use the **same code for the same person in every meeting**. This lets us test recognising a person across meetings. Codes only, never real names. |
| `names` | Real names that might be spoken: people, companies, projects. Add them as you learn them. |
| `duration_s` | Optional. Read from the audio if left out. |

The `rttm` and `ref` files do not exist yet. Add those lines now or after labelling; `validate` in the next step tells you what is missing.

## 6. Check coverage

Run from the `tools/eval` folder (see `runbook.md` for the full path to your dataset):

```
uv run ghi-eval validate --dataset /path/to/customer-2026q4
```

It prints hours per language and per setting and compares them with the targets. Read the `warning:` lines: they say what is still missing, for example "lang mixed: 12% of hours, target ~30%" or "calls: need both headphones and speakers playback". Record more until the warnings are gone or you agree they are acceptable. An `error:` line (a missing file or an unreadable WAV) must be fixed.

## Coverage checklist

- [ ] At least 10 hours in total
- [ ] About 40% `vi`, 30% `en`, 30% `mixed` (each within 10 points)
- [ ] Room recordings and call recordings
- [ ] Calls on headphones and calls on speakers
- [ ] 3 to 5 recordings with 6 to 8 speakers
- [ ] Every speaker signed the consent form, forms filed with the data contact
- [ ] No names in file names
- [ ] Two-track calls where you could (`tracks` in the manifest)
- [ ] `names` list filled in, `persons` filled in
- [ ] `ghi-eval validate` shows no errors
- [ ] Recordings are on the encrypted volume only
