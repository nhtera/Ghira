# Export

You can take a meeting out of Ghira as a file, as text for your clipboard, or as a draft email. Nothing is uploaded: files are written on your Mac, to a folder you choose.

## Export one meeting

1. Open the meeting, open the **Export** menu and choose **Export…**.
2. Pick a **Format**.
3. Under **Include**, turn **Notes** and **Transcript** on or off. Pick **EN** or **VI** for the headings. **Preview** shows the result.
4. Choose **Export…** to save a file. Ghira asks where to save, unless you already chose a folder.

The formats:

| Format | File |
|---|---|
| Markdown | `.md` |
| Word document | `.docx` |
| Plain text | `.txt` |
| Subtitles | `.srt` or `.vtt` |

Subtitles hold the transcript with speaker names, and nothing else.

Under **Export…** the same sheet has buttons for one meeting: **Copy as Markdown**, **Copy as text**, **Export to Obsidian…** and **Draft follow-up email**. **Copy Markdown** in the **Export** menu copies the notes in one step.

Ghira never overwrites a file. If the name exists, it saves "Name (2)". After saving, **Show in Finder** opens the file.

## Export several meetings

On **Meetings**, select the meetings and choose **Export…**. Ghira saves one file per meeting in the folder you pick, in the format you choose. To skip the folder question, set **Export folder** under **Settings → General**, or use **Change…** in the sheet.

## Obsidian

Choose **Export to Obsidian…** and pick the folder of your vault once. Ghira remembers it, and you can change it later with **Change…** or under **Obsidian vault folder** in **Settings → General**.

Each meeting becomes a Markdown note. The front matter has the title, date, duration, participants and tags. The tags always include `ghira`, plus the meeting's own tags, cleaned up so Obsidian accepts them.

## Draft a follow-up email

1. Open the **Export** menu and choose **Draft follow-up email**. (The export sheet has the same button.)
2. Check **To**. If the meeting was recorded from a calendar event, the attendees' addresses are filled in. See [Calendar](calendar.md).
3. Choose a **Tone**: **Friendly**, **Neutral** or **Formal**, and whether to **Include action items**.
4. Choose **Write draft**, and edit the **Subject** and **Message**. **Rewrite** makes a new draft.
5. Choose **Copy email**, or **Open in Mail**.

The local model writes the draft from the meeting's notes, on your Mac. Ghira never sends the email itself. If the message is long, the mail app may receive a shortened one. Use **Copy the full email** then.

## Export everything

**Settings → Privacy → Export everything…** saves all your meetings with their audio in one `.ghira` file, protected by a password of at least 8 characters. You need the password to open it, and it cannot be recovered. To move meetings to another device, see [iPhone and sync](iphone-sync.md).

## Limits

- Audio is not part of these formats.
- A sensitive meeting exports its transcript and notes only, because it has no audio.
- Exports are plain files. Once saved outside Ghira they are no longer encrypted by Ghira.
