# Settings

Open Settings with ⌘, or from the sidebar. It has nine sections. Most changes
apply at once. A few apply from your next recording, and the screen says so.

## General

- **Appearance.** Choose **System**, **Light** or **Dark**.
- **App language.** Choose English or Tiếng Việt.
- **Open Ghira at login.** Ghira needs this to notice when a call starts.
- **Show Ghira in the menu bar.** The icon always shows while you record, even if
  this is off, because it is the recording indicator.
- **Export.** Choose the **Export folder** and the **Obsidian vault folder** that
  exports save to.
- **Folders and tags.** Rename or delete the folders and tags you use to organize
  meetings. Deleting one never deletes a meeting.

## Languages

- **Languages spoken in meetings.** English, Tiếng Việt, or both.
- **Write notes in.** Pick a language, or **Same as the meeting**.
- **Search without accents.** Always on. “chot” finds “chốt”.
- **Custom vocabulary.** Add names, products and jargon. The final transcript is
  corrected towards these words. Ghira also learns terms from the names you give
  speakers. You can remove any of them.

## Recording

- **Microphone.** Shows whether macOS lets Ghira use it, with a button to open
  Privacy Settings.
- **Ask to record when these apps start a call.** Choose from Zoom, Microsoft
  Teams, Google Meet, Slack huddles, Zalo, Webex and FaceTime. Ghira always asks
  first and never starts recording on its own.
- **Echo cancellation on speakers.** Keeps other people's voices out of your
  microphone, so nobody is counted twice.
- **Record only the meeting app's sound.** Music and notifications from other
  apps are left out.
- **Consent message.** The text you paste in the meeting chat when you start
  recording. Edit it in English and in Tiếng Việt, or choose **Use the built-in
  message**.
- **Live transcript.** **Auto** chooses for your hardware. **Fast** is lighter on
  your Mac and shows captions a little later. **Accurate** shows captions sooner
  and uses more processing while you record. It applies from the next recording.
  A Mac with 8 GB of memory uses Fast.

## AI

- **On this device.** The default. Notes, summaries and answers are written by a
  model on your Mac. Nothing leaves it.
- **Cloud, only when you ask.** Offers cloud AI for one meeting at a time. It
  sends transcript text, never audio.
- **Your API keys.** Add a key for each provider. Keys are stored in the Keychain
  and a saved key is never shown again.
- **Default for cloud requests.** The provider and model the send sheet starts
  with.
- **Rules for cloud requests.** **Hide names and personal data by default**
  replaces names, emails and phone numbers before sending, then restores them
  here.
- **Request log.** Every time text left your Mac: time, meeting, provider, what
  was sent and tokens.

Ghira always shows the exact request before anything is sent.

## Models

- **Transcript after the meeting.** **Standard** or **High accuracy (Whisper)**.
  Shown only in builds that include Whisper.
- **Speed and quality.** Shows the preset Ghira chose for your Mac: Light,
  Balanced or Max.
- **Models on this computer.** Each model with its purpose, size, memory, status
  and license, and buttons to **Download**, **Pause**, **Resume** or **Download
  again** if a model fails its check.

See [Speech models](models.md).

## Privacy

- **Encrypt meetings on this computer.** Always on. The key never leaves your
  Mac.
- **Lock Ghira with Touch ID.** Hides your meetings until you unlock. Choose
  **Lock after**: only at launch and after sleep, or 1, 5, 15 minutes or 1 hour.
  **Lock now** locks at once. A recording keeps going while locked.
- **Your voice.** Shows your voice profile, and **Delete my voice data…** removes
  it.
- **Learn voices from confirmed speakers.** Off and locked. Other people's voices
  cannot be saved in this version.
- **Strict offline.** Blocks all network use: cloud AI and model downloads
  included. Sync with your paired phone on your own network stays allowed.
- **Keep audio for.** 7 days, 30 days, forever, or don't keep it. Transcripts and
  notes are kept either way.
- **Export everything.** Markdown, JSON and audio in one folder, protected by a
  password you choose. You cannot recover the password.
- **Danger zone.** **Delete all meetings and voice data…** deletes every meeting,
  note, audio file and voice profile, and the keys. You must type a word to
  confirm, and it cannot be undone.

## Sync

- **Sync with my phone.** Keeps your meetings in step with your iPhone over your
  own Wi-Fi.
- **Pair a phone.** Open Ghira on the phone and scan the code. Both devices need
  the same Wi-Fi. The code works for a short time, and you can show a new one.
- **Paired devices.** **Sync now**, **Unpair**, or **Unpair and wipe**.
- **Export for another device** and **Import from another device.** A file
  protected by a passphrase, for a device that cannot sync over the network.

Sync stays on your local network, and voice profiles stay on each device. The
iPhone app is still in testing.

## Shortcuts

The keys, and the switches for the two that work from any app. See
[Keyboard shortcuts](shortcuts.md).

## About

- **Updates.** A build from source says “This build doesn't update itself”. Once
  signed releases exist, **Check for updates** and **Check automatically** work
  here.
- **Open-source licenses.** The libraries, models and fonts Ghira is built with.
- **Diagnostics.** **Reveal reports and logs** opens the folder. Reports stay on
  your Mac and contain no meeting content.
