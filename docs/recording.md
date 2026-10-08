# Recording

Ghira records a meeting while it happens and writes a live transcript next to it. Audio and text stay on your Mac. You choose what to record: a call, or a room.

## Call or Room

- **Call** records two sources: your microphone (you) and the sound of the meeting app (everyone else). Ghira labels your microphone as **Me** and tells the other voices apart.
- **Room** records one microphone for a whole room. Ghira tells the voices apart from that single recording.

Pick the mode with the **Call** and **Room** switch on the **Live** screen, or start from the menu bar icon with **Record call** or **Record room**. Press ⌘⇧R to start or stop a recording from anywhere in the app.

**Call** needs the macOS permission **Screen & System Audio Recording**. Ghira records sound only, never your screen. Without that permission, or when no system sound is available, Ghira records in **Room** mode with your microphone. See [Privacy](../PRIVACY.md) for what stays on your device.

## Ask to record when a call starts

Ghira can notice that a meeting app has started using your microphone and ask whether to record. It never starts on its own.

1. Open **Settings → General** and turn on **Ask to record when a call starts**.
2. Turn on **Open Ghira at login** so Ghira can notice calls. Allow notifications when macOS asks.
3. In **Settings → Recording**, choose which apps count under **Ask to record when these apps start a call**: Zoom, Microsoft Teams, Google Meet, Slack huddles, Zalo, Webex and FaceTime. Calls in a web browser (Chrome, Edge, Safari, Arc, Firefox or Brave) follow the **Google Meet** setting. Discord is always detected while detection is on.

When Ghira asks, you can choose **Start**, **Not now**, or **Never for** that app. After you answer, Ghira stays quiet about that app for 30 minutes. If a calendar event starts, Ghira can offer to record it too; see [Calendar](calendar.md).

## Echo cancellation

When a call plays through your speakers, your microphone also hears the other people. Ghira removes that echo so nobody is counted twice.

- It turns on automatically in **Call** mode when sound plays on the speakers.
- It turns off when you connect headphones. Ghira tells you when it switches.
- You can turn it off in **Settings → Recording** with **Echo cancellation on speakers**.

**Record only the meeting app's sound** in the same section leaves out music and notifications from other apps.

## Tell people you are recording

Ghira shows a consent message you can paste into the meeting chat.

1. During a recording, choose **Copy consent message**.
2. Paste it in the chat of your meeting app.

You can change the wording, in English and in Vietnamese, under **Consent message** in **Settings → Recording**. **Use the built-in message** restores the original.

## While you record

- **Pause** stops recording. Nothing is recorded while paused, and the speakers and notes you have so far are kept. **Resume** continues in the same meeting.
- **Mark moment** (⌘M) saves a bookmark at the current time. The count shows as "marked".
- **Your notes** is a notepad on the **Live** screen. Type a line and press Enter. Each line links to the moment you typed it. You can tag the next note as **Decision**, **Action** or **Question**. See [Notes](notes.md) for how Ghira uses them.
- **Discard the last minutes** (in **More actions**) removes the audio, transcript lines, notes and marks from the last few minutes. It cannot be undone.
- **Speakers** appear in the order they talk. See [Speakers and voices](speakers.md).
- **Layout** switches between **Transcript** and **Focus**, and **Mini recorder** opens a small recorder.

Ghira warns you when something needs attention: a microphone that was unplugged, system audio that stopped, no sound from the meeting app, a full disk, or the Mac going to sleep. After a sleep the gap is marked and recording continues when the Mac wakes.

## Live transcript speed

**Settings → Recording → Live transcript** has three choices: **Auto**, **Fast** and **Accurate**. **Auto** picks one for your Mac. On a Mac with 8 GB of memory, Ghira uses **Fast**: captions appear a little later but your Mac works less. The choice applies from the next recording.

## Without speech models

If the speech models are not installed yet, Ghira still records the audio. The live transcript, speakers and notes are made after the models finish downloading.

## Sensitive meetings

A sensitive meeting keeps only its transcript.

- No audio is saved.
- It is never sent to cloud AI.
- Ghira does not learn anyone's voice from it.

To record one, turn on the **Sensitive** switch on the **Live** screen before you start. You can also choose **Sensitive meeting…** in **More actions** during a recording. Ghira asks first, because audio recorded so far is deleted when you stop, and you cannot turn the mode off during that recording. For a saved meeting, choose **Sensitive meeting** in the meeting's **Export** menu. Its audio is deleted at once and only the transcript stays.

Sensitive mode needs the speech models, because without a live transcript nothing would be kept. The meeting shows **Sensitive · no audio kept** in the library.

## Limits

- The macOS app is the only desktop app. It needs an Apple Silicon Mac with macOS 14.2 or later. There is no signed download yet; you build it from source.
- Ghira does not detect meetings in apps outside the list above.
