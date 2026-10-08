# Import

Import turns a recording you already have into a meeting: a transcript with speakers, then notes, the same as a live recording. Everything runs on your Mac.

## What you can import

- **Voice Memos.** Drag the recordings from Voice Memos or from Finder. Ghira reads the title and date from the file name.
- **Zoom.** Drop a Zoom meeting folder. If Zoom saved a separate audio file for each participant, Ghira uses them. See below.
- **Teams and Google Meet downloads.** Ghira recognizes the file names of both and sets the date and title.
- **Plaud.** Plaud exports are ordinary audio files. Drop them in like any other file; Ghira recognizes Plaud files by name.
- **Any audio or video file.** Ghira reads MP3, M4A, WAV, MP4, MOV and OGG, and also AAC, FLAC, Opus, WebM, CAF and AIFF. A video file is imported by its sound. Ghira does not include FFmpeg: it uses its own decoders and the ones built into macOS.

If Ghira cannot read a file, it says so and suggests importing audio or video instead.

## Import files

1. Open **Import** in the sidebar.
2. Drop files on the window, or choose **Choose files…**. You can also drop files anywhere in the Ghira window, or on the Ghira icon in the Dock.
3. Check the files under **Ready to import**. Remove any you do not want.
4. Set **Options for these files**:
   - **Language**: **Auto-detect**, **English**, **Tiếng Việt** or **Both, mixed**.
   - **Expected speakers**: leave it on **Auto**, or enter how many people spoke.
   - **Split stereo channels**: for stereo recordings where you are on the left channel and the others on the right. Ghira uses that to tell speakers apart and treats the file as a call.
5. Choose **Import**, for example **Import 2 files**.

Each file moves through **Converting**, **Transcribing**, **Finding speakers** and **Writing notes** in the **Queue**, with an estimate of the time left. You can keep working or close the window. Ghira shows a notification when each file is done, and the meeting appears in **Meetings**. Choose **Open notes** to see it.

Ghira recognizes a file it already imported and says it is already there, with the meeting's title. Choose **Import as a copy** if you want it again. Imports wait while you record.

## Zoom participant tracks

Zoom can save each participant's audio as a separate file when you turn on **Record a separate audio file for each participant**. In Ghira, drop the meeting folder (or its **Audio Record** folder). Ghira shows **Zoom recording · N participants** and imports the tracks as one meeting.

Each person has their own track, so Ghira knows who said what without guessing. Names come from the file names. You can fix names afterwards; see [Speakers and voices](speakers.md).

- A recording can have up to 49 participant tracks.
- The mixed recording in the same folder is left out, since the tracks replace it.
- To import the tracks as separate meetings instead, choose **Import tracks separately**.

## iPhone share sheet

On iPhone, share an audio file to Ghira from Voice Memos, Files or another app. Choose the **Language** and **Process on**: **This phone**, **My desktop** (after you pair a Mac) or **Cloud notes**. Then choose **Import**.

The phone accepts M4A, MP3, WAV, AAC, FLAC, OGG, Opus, AIFF and MP4. It does not accept CAF files. If you close the share sheet early, the file waits in **Waiting to import** until you review it. See [iPhone and sync](iphone-sync.md).

## Limits

- A file over 4 hours can be imported, but it takes a while. For a long file, Ghira shows how long it will take and how much disk space it needs.
- If the audio stops being readable partway, Ghira offers to import the readable part.
- Speaker separation on a single mixed file is an estimate. Splitting stereo or using Zoom tracks is more accurate.
- Imports need the speech models. Until they are installed, files wait.
