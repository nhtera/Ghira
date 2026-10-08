# iPhone and sync

Ghira has an iPhone app that records meetings and syncs them with your Mac over your own Wi-Fi. The Mac is the hub and the iPhone is the spoke.

## Status

The iPhone app is in testing. It is not on the App Store, and there is no download yet. It is built and tested in the iOS Simulator, and runs on a physical iPhone are not fully checked yet. Expect rough edges. There is no Windows or Android app yet.

## What the iPhone app does

- Records a meeting and writes a transcript on the phone. The speech models run on the phone, and you download them once under **Settings → Models**.
- Imports audio files from the share sheet. See [Import](import.md).
- Keeps your voice profile, custom vocabulary, consent message and calendar settings on the phone.
- Can lock with Face ID.

## Notes come from your Mac

The iPhone app does not write notes with a local model. Notes come from one of two places:

- **Your paired Mac.** The phone sends the recording to the Mac, the Mac refines the transcript and writes the notes, and both come back to the phone. While the Mac does this, the meeting shows **Refining on** your Mac, and you cannot edit the transcript until it returns.
- **The cloud.** Turn on **Offer cloud notes** in **Settings → Cloud notes**. It is off by default, and each send is previewed first. See [Cloud AI](cloud-ai.md).

If your Mac is away, you can choose how long a meeting waits under **If your computer is away**. After 6 hours, 12 hours, 1 day or 3 days, a phone that is able to do so processes it itself. The transcript is made on the phone, but notes still need the Mac or the cloud.

## Pair your iPhone with your Mac

You need both on the same Wi-Fi. No internet is needed.

1. On the Mac, open **Settings → Sync** and turn on **Sync with my phone**.
2. The Mac shows a code under **Pair a phone**. It works for about two minutes. **Show a new code** makes another.
3. On the iPhone, open **Settings**, choose **Sync with computer**, then **Pair a computer** and **Scan the code**. Allow the camera and the local network when iOS asks.
4. When it says **Paired with** your Mac, syncing starts. Choose **Sync now** any time.

Both apps need to be open to sync.

## What syncs

- Notes and transcripts go both ways.
- Recordings you make on the phone are sent to the Mac for the final pass.
- Audio of meetings recorded on the Mac stays on the Mac. On the phone those meetings show **Audio is on** your Mac, with the notes and transcript.
- Names, edits and deletes sync. If both devices edit the same thing, you see **Edited on** the other device and choose **Use this** or **Dismiss**. When you delete a meeting, Ghira asks whether to **Delete everywhere** or **Delete here only**.
- **Voice profiles never sync.** They stay on each device. Ghira is not able to move a voiceprint to your phone.

## Privacy and security

- Sync uses your local network only. Nothing goes through the internet and no Ghira server is involved.
- Everything is end-to-end encrypted between your two devices with the Noise protocol. Keys are exchanged when you scan the code, and each device pins the other's key.
- Ghira accepts connections only from private network addresses.
- **Strict offline** blocks internet use but keeps sync on your private network.

See [Privacy](../PRIVACY.md) and [Security](../SECURITY.md).

## Unpair

Choose **Unpair** on either device. Meetings already on each device stay. **Unpair and wipe** on the Mac, or **Unpair and delete on** your Mac on the phone, also asks the other device to delete the meetings you share. That happens the next time it is on your Wi-Fi with Ghira open. Until then, anyone who can unlock that phone and open Ghira can still read them.

## If your phone cannot find your Mac

Some Wi-Fi networks, such as hotels and offices, stop devices from seeing each other. Turn on **Personal Hotspot** on the iPhone and join it from the Mac, or turn on Internet Sharing on the Mac and join it from the phone. Then try again.

## Export for another device

If sync does not work for you, or you are moving to a new computer, save your meetings as a file instead.

1. On the Mac, open **Settings → Sync** and choose **Export for another device**.
2. Enter a passphrase and save the file. It holds all your finished meetings with their audio.
3. Move the file yourself, for example with AirDrop or a USB drive.
4. On the other computer, choose **Import from another device**, enter the passphrase and pick the file. Ghira merges the meetings into the library. Meetings you deleted there stay deleted.

You need the passphrase to open the file, and it cannot be recovered. The iPhone app cannot open this file yet.
