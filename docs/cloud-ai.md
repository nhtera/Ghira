# Cloud AI

Ghira writes notes and answers questions on your Mac. Cloud AI is an optional extra for the cases where you want a larger model, such as a long meeting or one that mixes languages. It is off by default, and you decide for each meeting.

## What is sent, and what never is

- Only transcript text is sent. Audio never leaves your Mac.
- The notes you typed yourself are not sent.
- Before anything is sent, Ghira shows the exact text, the address it goes to, a checksum and an estimated cost. Ghira sends those bytes and nothing else.
- A meeting marked **Sensitive**, or one you mark **Never send to cloud**, cannot be sent.
- **Strict offline** blocks cloud requests together with every other network use.

Ghira runs no server of its own. Your text goes straight from your Mac to the provider you picked, under your own account with them. The provider handles the text under its own terms, and Ghira cannot see or control how long it keeps it. See [Privacy](../PRIVACY.md).

## Set it up

1. Open **Settings → AI**.
2. Under **Your API keys**, paste a key for a provider and choose **Save key**. Ghira supports OpenAI, Anthropic and Gemini. Keys are stored in the macOS Keychain, and a saved key is never shown again. **Remove key** deletes it.
3. Under **Default for cloud requests**, choose the provider and model the send sheet starts with.
4. Review the switches under **Rules for cloud requests**: **Ask before each meeting is sent** and **Never send meetings tagged Sensitive**.

The **AI mode** is **Per meeting**: nothing is sent unless you choose it for that meeting. Without a key, the send sheet asks you to add one first.

## Send a meeting

1. Open the meeting. Choose **Improve with cloud…** to write the notes again, or in the **Export** menu choose **Ask**, then **Cloud…** to ask one question.
2. Pick the **Provider** and **Model**. Prices are shown in US dollars per million tokens.
3. Decide on **Hide names and personal data**. It is on by default; **Settings → AI** can change that default.
4. Read **Exactly what is sent**. Choose **Show exact data** to see what was hidden.
5. Choose **Send and improve**, or **Keep local** to send nothing.

When the reply comes back, Ghira puts the real names back on your Mac. Your own notes, the lines you edited and the actions you ticked stay as they are. If the cloud request fails, your local notes are kept.

## Redaction

With **Hide names and personal data** on, Ghira replaces these with placeholders before sending: names, organizations, emails, phone numbers, links, ID numbers, card numbers, account numbers and terms. People in the meeting's calendar event are covered too. See [Calendar](calendar.md).

Redaction is not perfect. If something may still be personal, the sheet lists it under **Might still be personal**. Read the exact text before you send. With redaction off, names are sent as written, and the sheet warns you.

## Request log

**Settings → AI → Request log** lists every time text left your Mac: time, meeting, provider, what was sent (transcript, with or without redaction) and tokens. The log never keeps the text itself. A meeting that used the cloud carries a **Cloud-enhanced** mark, and Ghira shows **Cloud used for this meeting** while you view it.

## Turn it off

Remove your API keys, or turn on **Strict offline** in **Settings → Privacy**. For a single meeting, choose **Never send to cloud** in its **Export** menu.

## Limits

- Cloud AI works on one meeting at a time. Ask across all meetings always runs on your Mac.
- You cannot add other providers or your own server in the app.
- Cloud notes cost money. The sheet shows an estimate and a maximum, but your provider sets the final price.
- On iPhone, cloud notes are off until you turn on **Offer cloud notes**. See [iPhone and sync](iphone-sync.md).
