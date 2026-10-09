# Notes

Ghira writes notes for every meeting: a summary, decisions, action items with owners, and open questions. A model on your Mac writes them from the transcript. Each point links back to the moment it was said.

## What you get

The notes have these sections, depending on the template:

- **Summary**
- **Your notes, filled in**: the lines you typed, expanded with points from the transcript
- **Decisions**
- **Action items**, each with an owner you can change and a **Done** box
- **Open questions**
- **Key quotes**

Select a point to see where it came from. **Show in transcript** jumps to the line, and **Play from** plays the audio at that moment. When Ghira cannot find support for something in the transcript, it says "Not found in the transcript" and keeps the point as your note.

Your own words stay yours. The page labels each part as **You wrote**, **Written by Ghira from the transcript**, or **Edited by you · kept on regenerate**. **My notes only** shows just what you wrote.

## Your notes while recording

Type notes in **Your notes** on the **Live** screen. Press Enter after each line. Tag a note as **Decision**, **Action** or **Question** to put it in that section. Ghira keeps your lines, links each to the moment you typed it, and expands them with details from the transcript. If you wrote nothing, the page says so and the notes come from the transcript alone.

## When notes are written

1. When you stop recording, Ghira writes first notes from the live transcript.
2. After the final pass improves the transcript, Ghira writes the notes again from the better text.

Anything you wrote, edited, pinned or ticked off stays through both steps. If the speech or notes models are not installed yet, the recording is saved and notes are written after the models download.

## Templates

A template decides which sections the notes have. Ghira has seven built in, and you can make your own (see [Templates](templates.md)):

| Template | Extra sections |
|---|---|
| **General** | The standard sections only |
| **1:1** | Feedback, Concerns, Growth |
| **Client call** | Client requests, Client feedback |
| **Standup** | Done, Next, Blockers |
| **Interview** | Experience, Strengths, Concerns |
| **Lecture / Workshop** | Key concepts, Examples |
| **Sales call** | Pain points, Budget and timeline, Objections |

If a meeting has a calendar event, Ghira may pick a template from the event title and the number of attendees. Otherwise it uses **General**. See [Calendar](calendar.md).

To change it, open the meeting, open the **Export** menu and pick a template from the list. Ghira then asks to write the notes again with it.

## Edit and regenerate

You can edit the notes and the transcript. Editing a transcript line keeps the notes' citations.

To write the notes again, open the **Export** menu and choose **Regenerate notes**. Ghira shows the template and notes language it will use, and asks you to confirm. What you wrote, edited, pinned or ticked stays.

To fix a wrong transcript from the audio instead, choose **Transcribe again…** in the same menu. Speaker names and lines you edited stay, and the notes are written again afterwards.

## Notes language

Set **Write notes in** under **Settings → Languages**: **Same as the meeting**, **English** or **Tiếng Việt**. In a meeting, the **EN** and **VI** switch above the notes picks the language for the next regenerate.

## The local model

Notes, summaries, answers and follow-up emails come from a local model, Qwen3 4B, downloaded in **Settings → Models**. It runs on your Mac and nothing leaves it.

Ghira picks a speed and quality preset from your Mac's memory, and shows it under **Speed and quality** in **Settings → Models**:

| Preset | Memory |
|---|---|
| Light | 8 GB |
| Balanced | 16 to 24 GB |
| Max | 32 GB or more |

All presets use the same notes model. On Light, Ghira does not run the notes model while you record, so notes wait until the recording stops. If the model stops while writing, the meeting shows **Try again on this Mac**, and your transcript and audio are safe.

## Better notes with cloud AI

For long or mixed-language meetings, you can ask a cloud model to write the notes again with **Improve with cloud…**. It is off by default, asks every time, and shows the exact text first. See [Cloud AI](cloud-ai.md).

## On iPhone

The iPhone app does not write notes with a local model. Notes come from your paired Mac, or from the cloud if you turn that on. See [iPhone and sync](iphone-sync.md).

## Limits

- Notes quality depends on the transcript. Correct names and wrong lines, then regenerate.
