// SPDX-License-Identifier: Apache-2.0

// The landing page's copy (English). It describes what ships today (plan
// decision D10): where the approved prototype said more than the code does,
// the words here follow the code. Components never hold copy.

import type { IconName } from "@/components/site/icons";

export const landing = {
  hero: {
    announceTag: "New",
    announce: "Ghira for iPhone is in testing",
    title: "Meeting notes that stay on your Mac.",
    sub: "Ghira records your calls and in-person meetings in English, Vietnamese, or both in one sentence. It shows who said what as people talk, then writes notes you can check line by line. All of it runs on your Mac.",
    primary: "Get the source on GitHub",
    secondary: "Read the docs",
    trustLabel: "What Ghira promises",
    trust: [
      { icon: "lock", label: "Runs on your Mac" },
      { icon: "nobot", label: "No bot joins your call" },
      { icon: "user", label: "No account needed" },
      { icon: "code", label: "Open source, Apache 2.0" },
    ] satisfies { icon: IconName; label: string }[],
    status: "Pre-release for Macs with Apple Silicon and macOS 14.2 or later.",
  },
  demo: {
    label: "Example: a meeting being recorded and transcribed live",
    notesLabel: "Your notes during the call",
    caption: "A sample meeting from Ghira's test set, replayed faster than real time.",
    pause: "Pause",
    play: "Play",
    langLabel: "Meeting language",
    english: "English",
    vietnamese: "Tiếng Việt",
  },
  listen: {
    id: "listen",
    title: "Nothing joins your call",
    lead: "Ghira listens through your Mac, so there is no extra participant to explain and nothing for the other side to approve.",
    items: [
      {
        icon: "nobot",
        title: "No bot in the meeting",
        body: "Ghira records your microphone and the meeting app's sound on your Mac. Nobody new appears in the participant list.",
      },
      {
        icon: "apps",
        title: "Any meeting app",
        body: "Zoom, Google Meet, Microsoft Teams or anything else that plays sound. Ghira notices when a call starts and offers to record.",
      },
      {
        icon: "room",
        title: "In the room too",
        body: "Switch to Room for an in-person meeting, on the Mac or the iPhone. Speakers are still told apart.",
      },
    ] satisfies { icon: IconName; title: string; body: string }[],
  },
  after: {
    id: "after",
    title: "After the call, notes you can check against what was said",
    lead: "A model on your Mac writes the summary, decisions, action items with owners, and the open questions. Every sentence links to the moment it came from. Select a time to see the line it cites.",
    modeLabel: "Notes view",
    typed: "What you typed",
    full: "Full notes",
    typedNote: "Four quick lines, typed while listening. Switch to Full notes to see what Ghira wrote from them.",
    transcriptLabel: "Transcript",
  },
  apps: {
    id: "apps",
    title: "On your Mac, and in your pocket",
    lead: "The Mac app records calls and writes the notes. The iPhone app records meetings in the room and hands them to your Mac over your own network.",
    tabsLabel: "Mac app screen",
    tabs: {
      live: {
        label: "During a call",
        caption: "Speakers get a color and a number as they talk. Rename them during the call, and type your own notes beside the transcript.",
        light: "Ghira on a Mac during a call: the live transcript with each speaker's color and number, and your own notes on the right.",
        dark: "Ghira on a Mac during a call, dark theme: the live transcript and your own notes.",
      },
      notes: {
        label: "After the call",
        caption: "After the call: summary, decisions and action items, each with the time it was said. Play any moment from the bar at the bottom.",
        light: "A meeting's notes in Ghira on a Mac: summary, decisions and action items, each with the time it was said.",
        dark: "A meeting's notes in Ghira on a Mac, dark theme.",
      },
    },
    phone: {
      title: "Ghira for iPhone",
      lead: "Record a workshop or a 1:1 away from your desk. The transcript appears as people talk. When you're back, your Mac writes the notes.",
      tag: "In testing",
      shots: [
        {
          id: "phone-live",
          caption: "Recording a meeting in the room",
          light: "Ghira on iPhone recording a meeting in the room, with Vietnamese and English lines from three speakers.",
          dark: "Ghira on iPhone recording a meeting, dark theme.",
        },
        {
          id: "phone-meetings",
          caption: "Your meetings, searchable on the go",
          light: "The meetings list on iPhone, with search where accents are optional.",
          dark: "The meetings list on iPhone, dark theme.",
        },
      ] as const,
    },
  },
  privacy: {
    id: "privacy",
    title: "What leaves your Mac",
    lead: "Ghira works offline. These are the rules, and anyone can check them in the code: every network call goes through one small module.",
    rules: [
      {
        what: "Your meetings",
        small: "Audio, transcripts, notes, voice profiles, titles, people",
        verdict: "Never sent",
        tone: "never",
        how: "Two exceptions, each one you turn on yourself: cloud AI for a single meeting (text only, after you review it), and sync with your own devices over your local network (end-to-end encrypted).",
      },
      {
        what: "Model downloads and update checks",
        small: "No meeting content",
        verdict: "Allowlist only",
        tone: "allow",
        how: "Model downloads: Hugging Face only. Update checks: off until signed releases exist (then GitHub Releases only, no ID).",
        strong: "Strict offline",
        after: "turns both off.",
      },
      {
        what: "Telemetry and analytics",
        verdict: "None",
        tone: "never",
        how: "Crash reports and the event log stay on your Mac. You can open them, read them, and send them yourself.",
      },
      {
        what: "Fonts and icons",
        verdict: "Bundled",
        tone: "never",
        how: "Nothing is loaded from the internet while you use the app. The models download once, then run on your Mac.",
      },
    ] as {
      what: string;
      small?: string;
      verdict: string;
      tone: "never" | "allow";
      how: string;
      strong?: string;
      after?: string;
    }[],
    cloud: {
      title: "Cloud AI is off until you choose it for one meeting",
      lead: "Before anything is sent, you see the exact text. Names, email addresses and phone numbers can be hidden. Audio is never sent, and neither are the notes you type yourself.",
      onMac: "On your Mac",
      sent: "What the provider receives",
      sentNote: "with names hidden",
      // A line is text with spans; `pii` and `mask` mark the replaced pieces.
      before: [
        [{ pii: "Linh" }, ": Em đã gom feedback từ 12 user test, gửi qua ", { pii: "linh.tran@studio.vn" }, "."],
        [{ pii: "Sarah" }, ": Call me on ", { pii: "+84 90 123 4567" }, " if the mockups are late."],
      ] as readonly (string | { pii: string })[][],
      after: [
        [{ mask: "[PERSON 2]" }, ": Em đã gom feedback từ 12 user test, gửi qua ", { mask: "[EMAIL]" }, "."],
        [{ mask: "[PERSON 4]" }, ": Call me on ", { mask: "[PHONE]" }, " if the mockups are late."],
      ] as readonly (string | { mask: string })[][],
      footnote: "You bring your own API key. Ghira runs no server, so it never sees your key or your data. This website has no analytics or cookies either.",
    },
  },
  compare: {
    id: "compare",
    title: "How it compares with cloud note takers",
    lead: "Most AI note takers send your meeting to their servers. Ghira does the same work on your own computer.",
    question: "Question",
    cloud: "Typical cloud note taker",
    cloudShort: "Cloud tools",
    ours: "Ghira",
    rows: [
      {
        q: "Where your audio is processed",
        cloud: "On their servers",
        ours: "On your Mac",
        sources: [
          { label: "tl;dv security", href: "https://tldv.io/features/security-commitment/" },
          { label: "Otter processing times", href: "https://help.otter.ai/hc/en-us/articles/360048322493-Transcription-processing-time-FAQ" },
        ],
      },
      {
        q: "Something joins the call",
        cloud: "Often a bot",
        ours: "Nothing joins",
        sources: [
          { label: "Fireflies", href: "https://guide.fireflies.ai/articles/6388921822-how-to-add-fireflies-to-a-meeting-as-a-participant" },
          { label: "Fathom on Teams", href: "https://help.fathom.video/en/articles/13114369" },
        ],
      },
      {
        q: "Account to sign up for",
        cloud: "Required",
        ours: "None",
        sources: [
          { label: "Fathom quick start", href: "https://help.fathom.video/en/articles/276608" },
          { label: "Otter terms, section 3.1", href: "https://otter.ai/terms-of-service" },
        ],
      },
      {
        q: "Works with no internet",
        cloud: "Needs a connection to transcribe",
        ours: "Yes, after the models download",
        sources: [
          { label: "Otter processing times", href: "https://help.otter.ai/hc/en-us/articles/360048322493-Transcription-processing-time-FAQ" },
          { label: "Fireflies troubleshooting", href: "https://guide.fireflies.ai/articles/5736968288-troubleshooting-transcription-issues" },
        ],
      },
      {
        q: "Vietnamese, and both in one sentence",
        cloud: "Varies by tool",
        cloudMark: "varies",
        ours: "Built for it",
        sources: [
          { label: "Otter languages", href: "https://help.otter.ai/hc/en-us/articles/360047247414-Supported-languages" },
          { label: "Fireflies multi-language mode", href: "https://guide.fireflies.ai/articles/2585231364-transcribe-fireflies-meetings-in-multiple-languages-with-multi-language-mode-beta" },
        ],
      },
      {
        q: "Source code you can check",
        cloud: "Closed source",
        ours: "Open, Apache 2.0",
        sources: [
          { label: "Otter terms, section 11", href: "https://otter.ai/terms-of-service" },
          { label: "Fireflies terms", href: "https://fireflies.ai/terms-of-service" },
        ],
      },
    ] as {
      q: string;
      cloud: string;
      cloudMark?: "varies";
      ours: string;
      sources: { label: string; href: string }[];
    }[],
    sourcesTitle: "Sources",
    sourcesLead: "Typical means the public documentation of well-known cloud note takers, read in October 2026. Tools differ, and plans differ within a tool.",
  },
  features: {
    id: "features",
    title: "Everything else it does",
    items: [
      {
        title: "Speakers and voices",
        body: "People are told apart as they talk, and you can rename them during the call. Ghira can learn your own voice, with your consent. Profiles for other people come later.",
      },
      {
        title: "Search every meeting",
        body: "Accents are optional, so “hop” finds “họp”. Ask a question across meetings and get an answer with its sources. Matching by meaning needs the Balanced or Max models.",
      },
      { title: "Import recordings", body: "Voice Memos, Plaud, Zoom recordings with a file per participant, or any audio or video file." },
      { title: "Export anywhere", body: "Markdown, Word, plain text, subtitles, or an Obsidian folder. Draft a follow-up email from the notes." },
      { title: "Encrypted on disk", body: "Each meeting has its own key. Deleting a meeting destroys the key, so its audio and text can't be recovered." },
      { title: "Your calendar", body: "Optional. Ghira offers to record when a meeting starts and uses the attendees' names to spell them right." },
      {
        title: "iPhone and sync",
        body: "Record on your phone and let your Mac write the notes. Sync runs over your own network, end-to-end encrypted.",
        tag: "In testing",
      },
      { title: "App lock", body: "Lock Ghira with Touch ID or your password. A recording in progress keeps going while it's locked." },
    ] as { title: string; body: string; tag?: string }[],
  },
  faq: { id: "faq" },
  get: {
    id: "get",
    title: "Try it today",
    lead: "There is no signed download yet. You can build Ghira from source on a Mac with Apple Silicon.",
    platformsLabel: "Platforms",
    platforms: [
      { name: "Mac", note: "Apple Silicon, macOS 14.2 or later", pill: "Pre-release", tone: "go" },
      { name: "iPhone", note: "Built and tested on the Simulator", pill: "In testing", tone: "test" },
      { name: "Windows", note: "The code compiles, the app isn't shipped", pill: "Not yet", tone: "no" },
      { name: "Android", note: "No app", pill: "Not yet", tone: "no" },
    ] as { name: string; note: string; pill: string; tone: "go" | "test" | "no" }[],
    codeTitle: "Build from source (Terminal)",
    codeComment: "# Needs Rust, Node 22 with pnpm, CMake 3.26+ and Xcode",
    copy: "Copy",
    copied: "Copied",
    selected: "Selected",
    noteBefore: "On first launch Ghira downloads the speech models for your Mac, a few GB, each checked against a pinned SHA-256. Step-by-step guide in ",
    noteLink: "Install from source",
    noteAfter: ".",
  },
} as const;
