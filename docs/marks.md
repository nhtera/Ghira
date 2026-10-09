# Marks

A mark is a bookmark you press while recording: "this part matters". Ghira uses your marks to write better notes, and shows you which of them the notes cover.

## Mark a moment

Press **Mark moment** (⌘M) on the **Live** screen. The count shows as "marked". See [Recording](recording.md).

When you tag a note on the **Live** screen as **Decision**, **Action** or **Question**, that moment is marked too, with the tag. A mark from **Mark moment** has no tag; it is just a star.

## What a mark does

- **Steers the notes.** When the local model writes the notes, it is told which transcript lines you marked and asked to cover each one, in the matching section when the mark has a tag. It still writes everything else as usual.
- **Sits on a line.** A mark belongs to the last transcript line that started at or before it, if that line ended no more than 10 seconds earlier. If people talked over each other and a longer line that started earlier is still running, the mark sits on that line. A mark in a long silence keeps only its time.
- **Shows in the transcript.** The line shows a star and the mark's tag if it has one.
- **Shows on the waveform.** The audio bar has a tick for each mark.

## Which marks the notes cover

After the notes are written, Ghira checks each mark against the notes. A note sentence or action item **covers** a mark when one of the moments it cites overlaps the marked line.

- A covering sentence or action shows a star. Hover it to see "You marked this at 12:04".
- Marks nothing covers are listed under **Moments you marked**, with the time, the line and a **Play from** button. The section only appears when there is something in it.
- Topics, your own notes and anything else that is not a note sentence do not count as covering, so a mark is never hidden by them.

Nothing about this is stored: Ghira works it out each time you open the notes. So it stays right after you edit or regenerate the notes.

## On iPhone

The iPhone shows marks on transcript lines and lists **Moments you marked**. It is read-only.

## Privacy

Marks stay on your devices. They steer the notes written on your Mac, but they are **never sent to a cloud model**, even when you improve a meeting with cloud AI. Because of that, cloud-written notes do not know about your marks. **Moments you marked** still lists the ones they left out. See [Cloud AI](cloud-ai.md) and [Privacy](../PRIVACY.md).
