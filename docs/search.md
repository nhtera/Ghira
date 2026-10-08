# Search and Ask

Ghira has two ways to find things in your meetings. **Search** finds words in notes and transcripts. **Ask** answers a question in plain language and shows where in the meeting each part of the answer came from. Both run on your Mac.

## Search

Type in the box on **Meetings**: **Search notes and transcripts**. Results show matching meetings with the line that matched.

Accents are optional. Searching "hop" finds "họp", and "chot" finds "chốt". This is always on and works for English and Vietnamese together.

To narrow the list, use the filters **People**, **Source**, **Template**, **Date**, **Folder** and **Tags**. **Clear filters** resets them.

Other places to search:

- ⌘K opens **Search or jump to…**. Type a command or a meeting name. If nothing matches, press Enter to ask the question instead.
- Inside a meeting, **Find in transcript** looks only in that transcript.

## Ask

Ask answers from your meetings, written by the local model on your Mac. Each answer links to the moments it came from, so you can check it. When the meetings do not cover the question, Ghira says **Not discussed** instead of guessing.

There are two places to ask:

- **In one meeting.** Open the meeting, open the **Export** menu and choose **Ask**. Questions like "What was decided?" or "Who said they would send the file?" fit well.
- **Across meetings.** Open **Ask** in the sidebar and pick the scope: **This meeting**, **All meetings**, **A person** or **Date range** (last 7 days, last 30 days or this year). Ask a follow-up in the same thread, or choose **New question**.

Ask needs the notes model from **Settings → Models**. It waits while a recording is running or while notes are being written, and tells you when it can start.

### Ask on every preset

Ask works on every hardware preset. How it finds the right passages differs:

- **Light** (8 GB Macs) matches the words you type, with accents optional. The screen says **Searched by keywords only**.
- **Balanced** and **Max** (16 GB and up) also match by meaning, so a question can find a passage that uses other words. This needs the embedding model, which you download in **Settings → Models**. Ghira then indexes your meetings in the background, and a meeting becomes searchable by meaning once indexed. Meetings you edit are indexed again.

You can also use cloud AI for one question in one meeting. Choose **Cloud…** next to **Answer with**. Ghira shows the exact text first. See [Cloud AI](cloud-ai.md).

## Limits

- Ask answers only from meetings stored in Ghira.
- A sensitive meeting keeps its transcript, so you can search and ask about it. It never goes to the cloud.
- Answers come from a small local model. Open the cited moment to check anything important.
