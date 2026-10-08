// SPDX-License-Identifier: Apache-2.0

// The landing page's questions. Plain text only: the page renders them as
// <details>, and the structured data (FAQPage) reuses the same list.

export interface Faq {
  q: string;
  a: string;
}

export const FAQ_TITLE = "Questions people ask first";

export const FAQ: readonly Faq[] = [
  {
    q: "Does any of my audio go to the internet?",
    a: "No. Audio is recorded and transcribed on your Mac and never goes to the internet. Even when you choose cloud AI for one meeting, only the transcript text you reviewed is sent.",
  },
  {
    q: "Does a bot join my Zoom, Meet or Teams call?",
    a: "No. Ghira records your microphone and the meeting app's sound on your Mac, so nothing appears in the participant list.",
  },
  {
    q: "Do I need an account?",
    a: "No. There is nothing to sign up for. Ghira runs no server, so there is nowhere to log in.",
  },
  {
    q: "Which languages does it understand?",
    a: "English and Vietnamese, including both in the same sentence. Notes are written in the meeting's language, and you can switch them.",
  },
  {
    q: "What do I need to run it?",
    a: "A Mac with Apple Silicon and macOS 14.2 or later. The speech and notes models take a few GB of disk space and download once.",
  },
  {
    q: "How much does it cost?",
    a: "Ghira is open source under the Apache 2.0 license. If you choose cloud AI for a meeting, you use your own API key and pay that provider directly.",
  },
  {
    q: "Is there a Windows or Android app?",
    a: "Not yet. The Mac app comes first, the iPhone app is in testing, and Windows is planned.",
  },
];
