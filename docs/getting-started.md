# Record your first meeting

Set up Ghira once, then record a call or a meeting in the room. Everything in
this guide happens on your Mac.

## Before you start

- A Mac with Apple Silicon and macOS 14.2 or later, with Ghira
  [built from source](install.md).
- About 4 GB of free disk space for the speech and notes models.
- An internet connection for the first model download. After that, Ghira works
  offline.

## Set up Ghira

The first launch walks you through these steps. You can change most of them
later in [Settings](settings.md).

1. **Choose your languages.** Pick **English**, **Tiếng Việt**, or **Both, often
   mixed**. With both, Ghira understands mixed sentences like “chốt scope cho
   bản beta”.
2. **Download speech models.** Ghira picks a preset for your Mac's memory:
   Light, Balanced or Max. The download starts by itself, and you can continue
   while it runs. See [Speech models](models.md).
3. **Allow the microphone and system audio.** The microphone hears you and the
   room. **Screen & System Audio Recording** hears the other people on Zoom,
   Meet or Teams. Ghira records sound only, never your screen. Notifications
   and Calendar are optional.
4. **Add your voice.** Optional. Read a short passage, about 20 seconds, so Ghira
   can label you as Me. This step shows up once the voice model is downloaded or
   downloading. You agree to store the profile, and it stays encrypted on your
   Mac.
5. **Make a test recording.** Ten seconds is enough to check that both sources
   are heard and a line of transcript appears. Play a video or speak.
6. **Keep a recovery key.** Optional. Your meetings are encrypted with a key kept
   in your Mac's keychain. A 24-word recovery key brings them back if the
   keychain is lost. Write it on paper and keep it away from your Mac.

No internet? Skip ahead and record. Ghira keeps the audio and processes it once
the models are installed.

## Record a call

Choose **Record call** or press ⌘⇧R. When Ghira notices a meeting app using the
microphone, it asks “Record this call?”. It never starts on its own. The
transcript appears as people talk, and each new voice gets a color and an
initial.

Pick **Room** instead of **Call** to record a meeting around you with the
microphone alone.

While you record:

- Press ⌘M to **Mark moment** and flag what you want to come back to.
- Type in **Your notes** and press Enter. Each line links to the moment you typed
  it.
- Choose **Pause** to stop recording for a while. **Stop** ends the meeting.
- Use **Copy consent message** and paste it in the meeting chat. Recording rules
  differ by country and workplace, and Ghira cannot decide this for you.

All the keys are listed in [Keyboard shortcuts](shortcuts.md).

## After the call

When you stop recording, Ghira refines the speakers, improves the transcript
and writes the notes. You can leave the meeting open or
close it. A notification tells you when the notes are ready.

Open the meeting to read the **Notes** or the **Transcript**. Every sentence in
the notes links to the moment it was said.

## From the command line

The `ghi` command does the same work without the app, which is handy for files
you already have. Build it with the speech engines, then point it at a file:

```sh
cargo build --release -p ghi-cli --features nemo
target/release/ghi transcribe standup.wav --lang vi
```

It needs the models in `./models` (see [Install from source](install.md)). The
[Command line](cli.md) page lists every command.

## Limits

- The preset is chosen from your Mac's memory. You cannot pick another one.
- Vietnamese recordings made in a room are best effort.
- Notes need the notes model. Until it is installed, Ghira keeps the recording
  and waits.
