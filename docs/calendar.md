# Calendar

Connect a calendar and Ghira can name a meeting after its event, offer to record when the event starts, and use the attendee names to spell and label people correctly. Calendar access is optional and off until you connect it.

## Connect a calendar

Open **Settings → Recording** and find **Calendar**. There are two sources. You can use either or both.

**Your Calendar app (macOS).** Choose **Connect Calendar…** and allow access when macOS asks. If you said no before, turn on Ghira in **System Settings → Privacy & Security → Calendars**.

**A calendar file.** Choose **Import a calendar file (.ics)…** and pick one `.ics` file, for example one exported from Outlook. Ghira reads the file again when it changes. To update it, export the file again from your calendar app. **Remove calendar file** disconnects it.

## What Ghira does with events

- **Shows what is next.** **Meetings** shows an **Up next** line for your next meeting-like event: one with other attendees or a Zoom, Teams or Google Meet link. All-day events are ignored.
- **Offers to record.** Turn on **Ask to record when a calendar meeting starts**. When an event is about to start, Ghira asks "…is starting. Record it?". You choose; Ghira never starts on its own. It asks once per event. You can also switch the question on or off for a single event in the **Up next** line.
- **Names the meeting.** A recording started during an event takes the event's title, if you have not typed one.
- **Picks a template.** Ghira may choose the notes template from the event title and the number of attendees, such as **1:1** for one other person or **Standup** for a title that says so. See [Notes](notes.md).
- **Helps with names.** The attendees of a recorded meeting are used in four places:
  - to spell names correctly in the transcript
  - as the first choices when you rename a speaker; see [Speakers and voices](speakers.md)
  - in the names that **Hide names and personal data** replaces before a cloud send; see [Cloud AI](cloud-ai.md)
  - as the addresses in the **To** field of a follow-up email; see [Export](export.md)

## What is stored

Events are read when Ghira needs them and are not stored. The path of your `.ics` file stays inside the app.

Only a meeting you record keeps anything from its event: the title, the attendees' names and their addresses if the invite had them. This is encrypted with that meeting, and deleting the meeting deletes it. Nothing from your calendar is sent anywhere. See [Privacy](../PRIVACY.md).

## Limits

- Ghira does not read `.ics` files over 20 MB.
- Ghira does not edit your calendar or add notes to events.
- On iPhone, the app reads the phone's calendar and names a meeting at record start; see [iPhone and sync](iphone-sync.md).
