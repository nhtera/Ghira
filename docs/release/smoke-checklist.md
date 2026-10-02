# Release smoke checklist (macOS)

Scripted manual check of the core flows of the UI brief (`Plans/docs/03-ui-ux-design-brief.md`
section 9, deliverable 5, plus the screens of section 6) on a **clean Mac**, with the signed,
notarized DMG that the release workflow produced. Required because WKWebView cannot be driven by
WebDriver (RT-15). Every row has an expected result; record Pass / Fail / N/A and a note. A Fail
on any row marked **P0** blocks the release.

Run it twice: on macOS 14.2 (the minimum) and on macOS 26, base M1 16 GB if you have one. Use
`tools/release/net-audit.sh` in strict mode during sections 3-6 and 11 (see `network-audit.md`).

| Field | Value |
|---|---|
| Build (tag, DMG SHA-256) | |
| Mac (chip, RAM, macOS) | |
| Tester, date | |
| Language of the UI run | EN first; repeat sections 1, 6, 7 in VI |

## 0. Clean machine and install

A clean Mac = a fresh user account (or a macOS VM image) that has never run Ghira. If you reuse
an account, wipe first:

```sh
osascript -e 'quit app "Ghira"' ; rm -rf ~/Library/Application\ Support/com.nhtera.ghira \
  ~/Library/Caches/com.nhtera.ghira ~/Library/Logs/com.nhtera.ghira
tccutil reset All com.nhtera.ghira
security delete-generic-password -s com.nhtera.ghira 2>/dev/null   # repeat until "not found"
```

| ID | Step | Expected | P0 | Result |
|---|---|---|---|---|
| S-00a | Download the DMG, open it | Opens with no Gatekeeper warning; `spctl -a -vv -t open --context context:primary-signature Ghira.dmg` says `accepted`, source `Notarized Developer ID` | yes | |
| S-00b | Drag Ghira to Applications, launch it | No "unidentified developer" or "damaged" dialog; `spctl -a -vv /Applications/Ghira.app` accepted; `xcrun stapler validate /Applications/Ghira.app` worked | yes | |
| S-00c | `codesign --verify --deep --strict --verbose=2 /Applications/Ghira.app` and `codesign -d --entitlements - /Applications/Ghira.app` | Valid; hardened runtime; the only entitlement is `com.apple.security.device.audio-input`; every nested binary and dylib signed by the same Team ID | yes | |
| S-00d | Quit, launch twice more | A second launch focuses the running window; no duplicate Dock icons or menu-bar items | | |

## 1. First launch and onboarding (D1)

| ID | Step | Expected | P0 | Result |
|---|---|---|---|---|
| S-01 | First launch | Welcome screen with the privacy promise (on this Mac, nothing sent unless you choose cloud, citations); product name "Ghira"; no console errors; works at 960x640 | yes | |
| S-02 | Languages step | EN / Tieng Viet / Both selectable; choice is kept in Settings > Languages | | |
| S-03 | Speech models step | Preset (Light / Balanced / Max) preselected from this Mac's RAM, shows size and RAM; "Continue" works while the download runs; progress visible | yes | |
| S-04 | Cut Wi-Fi mid-download, restore it | Download shows failed/paused and resumes (resumes the `.part`, no restart from 0); a corrupt file is re-downloaded; after finishing, models verify (Settings > Models shows installed) | yes | |
| S-05 | Skip the download, continue offline | Allowed: "record now, process later"; recording is possible, the transcript and notes wait for the models | yes | |
| S-06 | Permissions step: Microphone | System prompt shows the Info.plist text ("Ghira records your microphone ... stays on this Mac"); after Allow the tick turns live | yes | |
| S-07 | Permissions: System audio | System prompt "System Audio Recording" shows the `NSAudioCaptureUsageDescription` text; after Allow the tick turns live; no "Screen Recording" prompt | yes | |
| S-08 | Permissions: Notifications | System prompt; skippable | | |
| S-09 | Deny Microphone (System Settings > Privacy), return | Clear message, deep link opens the right pane, a Room recording is refused with an explanation; granting then returns to a live tick without restarting the app | yes | |
| S-10 | Deny System audio | Call mode unavailable with an explanation; Room mode still works | yes | |
| S-11 | Your voice step | Reads a 20 s passage with a live level meter; consent line must be accepted to save; skippable; skipping stores nothing | | |
| S-12 | Test recording (10 s) | Mic and System levels move; a transcript line appears; Done | yes | |
| S-13 | Recovery key step | A recovery key/phrase is shown once; confirmation requires re-entering words; cancel leaves the app usable and asks again later | yes | |
| S-14 | Keychain | At most one expected Keychain prompt (item `com.nhtera.ghira`); choosing "Always Allow" or Allow does not repeat on relaunch; denying shows a clear "locked" message, not a crash | yes | |
| S-15 | Relaunch | No onboarding again; library empty state offers "Record your first meeting" and Import | | |

## 2. Menu bar, shortcuts, windows (D2)

| ID | Step | Expected | P0 | Result |
|---|---|---|---|---|
| S-20 | Menu-bar icon | Present, readable at 16 px in light and dark menu bars; popover lists Record (Call / Room), recent meetings, privacy state line | | |
| S-21 | Keyboard map | Cmd+Shift+R start/stop; Cmd+M mark; Cmd+K palette; Cmd+, settings; Cmd+F search; Cmd+1 / Cmd+2 Notes / Transcript; Space plays audio; Cmd+E export; Cmd+Q quit | yes | |
| S-22 | Meeting-detected prompt: start a Zoom/Meet/Teams call | Native notification "<App> call detected - Record?" with Start / Not now / Never for <App>; does not steal focus; Start begins recording in one click | | |
| S-23 | "Never for <App>" | No further prompts for that app; reversible in Settings > Recording | | |

## 3. Record a call (D4, flow 1: detect, record, name speaker, stop, notes)

Use a real call with at least two remote voices (or play a two-speaker video/podcast through
the call app as the far end) and your own voice on the mic. Run once with headphones, once with
speakers.

| ID | Step | Expected | P0 | Result |
|---|---|---|---|---|
| S-30 | Start a Call recording | Recording within 2 s; header shows timer, mode Call, language; mini-recorder available and excluded from screen share | yes | |
| S-31 | Speak, let others speak | Live transcript lag about <= 2 s EN / 3 s VN; partial text is visibly provisional; "Me" is your mic; remote speakers appear in arrival order as chips with colour **and** initial | yes | |
| S-32 | Headphones vs speakers | AEC badge reflects the route; with speakers, your voice is not duplicated as a remote speaker | | |
| S-33 | Level meters | Mic and System levels move independently; muting the far end drops System level | | |
| S-34 | No signal for 10 s (mute the call) | "Waiting for audio" hint, no crash | | |
| S-35 | Cmd+M and the Mark button | A marker appears in the transcript and lanes | | |
| S-36 | Notepad | Type lines while recording; they keep an invisible timestamp | | |
| S-37 | Pause / Resume | View dims while paused, Resume is obvious; the paused span has no audio | | |
| S-38 | Discard last N seconds | Confirm dialog; after it, that span is gone from transcript and audio | | |

## 4. Record a room (laptop mic)

| ID | Step | Expected | P0 | Result |
|---|---|---|---|---|
| S-40 | Start a Room recording with 2-4 people in the room, Vietnamese and English mixed | Works without System-audio permission; speakers separate best-effort (VN room is "best effort" per the owner's floors); code-switched phrases transcribe | yes | |
| S-41 | A 10+ minute run | Lag stays within the degrade threshold; no "model lag" badge, or it offers "Switch to Fast mode" | | |

## 5. Name speakers (D7)

| ID | Step | Expected | P0 | Result |
|---|---|---|---|---|
| S-50 | Click a speaker chip | Rename popover with autocomplete from People; a new unknown speaker shows "Identifying..." for <= 2.5 s then "Speaker N"; naming takes <= 2 clicks | yes | |
| S-51 | Rename, then check the transcript, chips, lanes | The name and the same colour + initial everywhere | | |
| S-52 | "Save voice to their profile" | Requires the consent dialog (biometric data, stored encrypted locally, deletable); cannot be skipped by accident; no voiceprint is stored without it | yes | |
| S-53 | Merge two speakers; mark one "Not a person" | Lines reassign; the non-person is dropped from notes owners | | |

## 6. Stop, processing, notes (D5, D6)

| ID | Step | Expected | P0 | Result |
|---|---|---|---|---|
| S-60 | Stop (Cmd+Shift+R) | Immediately returns to the library/detail; no blocking screen; the row shows progress steps: Refining speakers, Matching voices, Improving transcript, Writing notes | yes | |
| S-61 | Leave the meeting while it processes | Library row keeps the progress; a notification says when notes are ready | | |
| S-62 | Time it on a 60 min meeting (or note the ratio) | Notes from the live transcript <= 3 min after stop; refined notes after the final pass <= 10 min per 60 min of audio (RT-7) | yes | |
| S-63 | "Name your speakers" card | Appears only for unnamed voices, 3 s play-sample works, skippable | | |
| S-64 | Notes tab | Summary, Your notes, Decisions, Action items (owner chips with the speaker's colour+initial), Open questions, Key quotes; an AI block you edit becomes "yours" and survives Regenerate | yes | |
| S-65 | Switch EN / VI notes language | Layout stable; regenerates in the other language | | |

## 7. Citation to audio (flow 2)

| ID | Step | Expected | P0 | Result |
|---|---|---|---|---|
| S-70 | Click a citation chip (e.g. 12:04) on any AI sentence | The transcript scrolls to the line and audio plays from there in one click; chips are at least 24x24 px | yes | |
| S-71 | Audio bar | Play, scrub with the speaker-coloured waveform, speed, skip silence; works for a call (both tracks mixed) and across discarded gaps (silence) | | |
| S-72 | Transcript tab | Find-in-transcript, click a line to play, karaoke highlight, inline edit, reassign speaker on a selection | | |
| S-73 | A sentence whose citation is missing from the transcript | Flagged, never silently shown as sourced | | |

## 8. Library search (D3)

| ID | Step | Expected | P0 | Result |
|---|---|---|---|---|
| S-80 | Search `chot` | Finds "chot" lines (with the original diacritics highlighted); also `dong` finds `dong`, `da nang` finds `Da Nang` | yes | |
| S-81 | No results; filters (People, Source, Template, Date) | Empty state; filters combine | | |
| S-82 | 200+ meetings (import a batch) | List stays smooth (virtualized) | | |
| S-83 | Cmd+K palette | Jump to a meeting, Ask, Settings | | |

## 9. Export (D6)

| ID | Step | Expected | P0 | Result |
|---|---|---|---|---|
| S-90 | Copy Markdown; export .docx, .txt, .srt, .vtt | Files open in Word/Pages/QuickTime/VLC; names and timestamps correct; Vietnamese diacritics intact | yes | |
| S-91 | Send to the Obsidian folder | A .md appears in the chosen folder; no other folder is touched | | |
| S-92 | Draft follow-up email | A draft opens/copies in the chosen language and tone; nothing is sent; no network request (check net-audit) | | |

## 10. Import (D10)

| ID | Step | Expected | P0 | Result |
|---|---|---|---|---|
| S-100 | Drop a 30+ min .m4a / .mp3 / .wav / .mp4 on the window and on the Dock icon | Queue shows stages and ETA; a meeting appears with transcript and notes | yes | |
| S-101 | Stereo call recording with "split channels" | Two tracks (you on the first) | | |
| S-102 | Same file again | Detected as a duplicate | | |
| S-103 | A corrupted file and an unsupported file | A plain-language error, no crash, no half-imported meeting left in the library | | |
| S-104 | Quit during an import, relaunch | The half-imported meeting is gone; the file can be imported again | | |

## 11. Cloud send preview (flow 3: improve with cloud)

Use a test API key and a throw-away meeting containing a name and a phone number.

| ID | Step | Expected | P0 | Result |
|---|---|---|---|---|
| S-110 | "Improve with cloud..." before any key | Explains cloud is off and opt-in; nothing is sent | yes | |
| S-111 | Add the provider key (Settings > AI) | Stored in the Keychain, not shown again, not in any file under Application Support (`grep -r` for it) | yes | |
| S-112 | Open the send sheet | Shows provider and model, host, exact payload, SHA-256, word/token count, cost estimate; states "audio never leaves"; redaction toggle with before/after; the payload has the name and phone number replaced when redaction is on | yes | |
| S-113 | Edit the transcript after previewing, then Send | Refused: "the transcript changed since this preview, review it again" | yes | |
| S-114 | Send | Result appears; the engine chip turns amber "Cloud-enhanced - <provider>"; an entry appears in the request log (time, host, size, hash; no content) | yes | |
| S-115 | Settings > Privacy > Strict offline on, then try to send | Refused before any connection; model downloads also refused; LAN stays allowed | yes | |
| S-116 | Mark the meeting as cloud-locked / sensitive | Cloud send is not offered for it | | |

## 12. Settings (D11)

| ID | Step | Expected | P0 | Result |
|---|---|---|---|---|
| S-120 | Walk General, Languages, Recording, AI, Models, Privacy, Sync, Shortcuts, About | Every section opens; copy is plain (model names are secondary); EN and VI | | |
| S-121 | Models | Table shows size, RAM, language, licence, status; offline install from file works (`ghi models import` equivalent) and refuses a file that is not a pinned model | yes | |
| S-122 | About > Licenses | Lists the bundled models and third-party licences; no empty entries | yes | |
| S-123 | Privacy: include in backups | Off by default: `tmutil isexcluded ~/Library/Application\ Support/com.nhtera.ghira` says `[Excluded]`; turning it on shows a warning that backups keep deleted meetings | yes | |
| S-124 | Privacy: delete a meeting | Inline confirm; the meeting, audio, transcript and notes are gone; search no longer finds it | yes | |
| S-125 | Privacy: delete all | Inline confirm; the library is empty; the app still works and creates a new meeting | | |
| S-126 | App lock (if enabled in this build) | Touch ID / password required on launch; cancel leaves the app locked, not crashed | | |
| S-127 | Updates (phase 12b) | With updates off, no update traffic (net-audit shows no socket); manual check works; an update is deferred while recording | | |

## 13. Quit while recording

| ID | Step | Expected | P0 | Result |
|---|---|---|---|---|
| S-130 | Cmd+Q during a recording | A confirm dialog (stop and save / keep recording); "stop and save" finalizes the meeting and quits; it appears in the library with transcript and processing queued | yes | |
| S-131 | Close the main window during a recording | Recording continues in the menu bar / mini-recorder; reopening restores the live view | yes | |
| S-132 | Log out or restart the Mac during a recording | On next launch the meeting is recovered (see 14) | | |

## 14. Crash recovery

| ID | Step | Expected | P0 | Result |
|---|---|---|---|---|
| S-140 | Record 60 s of counting aloud ("one, two, ...") then `pkill -9 Ghira` | Relaunch shows a recovery prompt; the meeting is in the library; the audio ends within 5 s of the kill; the transcript and notes are produced after recovery | yes | |
| S-141 | Repeat during processing (final pass) | After relaunch the job resumes or is retried and completes | yes | |
| S-142 | After a crash (phase 12c) | A banner offers "Reveal report"; the local report holds no transcript, names or keys; a copy of the macOS .ips is kept; nothing was sent anywhere | yes | |
| S-143 | Hard power-off during a recording (hold the power button) | Relaunch recovers the meeting; loss <= 5 s (up to about 3 s expected). Do this 5 times over the release cycle | yes | |
| S-144 | Fill the disk to < 1 GB during a recording | Warning shown; recording stops cleanly with what was saved | | |
| S-145 | Unplug/change the mic or Bluetooth headset mid-recording; sleep and wake the Mac | The recording continues or stops with a clear message; a wake marker is recorded; nothing lost before the event | | |
| S-146 | Run `tools/release/crash-safety.sh` (see its header) on this Mac | `RESULT: PASS`, every loss <= 5 s | yes | |

## 15. Accessibility, themes, windows

| ID | Step | Expected | P0 | Result |
|---|---|---|---|---|
| S-150 | VoiceOver through live view and notes | New speaker turns are announced, not every word; controls have labels | | |
| S-151 | Keyboard only | Every control reachable; focus always visible | | |
| S-152 | Light, dark, Reduce Motion | Both themes legible; waveform/shimmer stop with Reduce Motion | | |
| S-153 | Resize to 960x640 and fullscreen | No clipping, including Vietnamese diacritics | | |

## 16. Network sanity (run beside sections 3-6 and 11)

| ID | Step | Expected | P0 | Result |
|---|---|---|---|---|
| S-160 | Strict offline on, `net-audit.sh --strict --app Ghira` through a whole meeting plus processing | RESULT: PASS (zero sockets), also with Little Snitch (`network-audit.md`) | yes | |
| S-161 | Default mode, same run | Only allowlisted hosts (huggingface.co and its CDN, the model mirror, the update feed), no request carries content | yes | |

## 17. Hardening trial (security verification F4)

| # | Step | Expected |
|---|---|---|
| 17.1 | Build once with `"freezePrototype": true` under `app.security` in `tauri.conf.json`, run sections 3, 5, 8 and 11 | Everything works (no errors in Web Inspector); then keep it on |

## Sign-off

A release candidate passes when every P0 row is Pass on both macOS versions, with the notes and
build recorded above. File each Fail as a bug with the row ID; P0/P1 bugs must be closed before
the tag is published.
