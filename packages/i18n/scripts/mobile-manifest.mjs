// SPDX-License-Identifier: Apache-2.0
// Classification manifest for extract-mobile.mjs: every entry of the `P` copy
// dict in "Ghi Mobile.dc.html" (the mobile design's EN/VI strings) is listed
// here, or the run fails.
//
// Directives (value of an entry):
//   "ns.key"      copy -> locale key mobile.ns.key ("{x}" -> "{{x}}", brand -> {{app}})
//   "@reason"     ignore (prototype-only, sample data, Android, OS-provided text)
//   ">ns.key"     same text as another entry's key (verified), no new key
//   { k, v, sub } options: v renames "{x}" placeholders ({x: "name"});
//                 sub replaces text with "{{var}}" ([[from, to]] or {en, vi});
//                 tpl [en, vi] is a hand-written template
//   { items: [...] }  the entry is [[en...], [vi...]]: one directive per index
const IGN = (why) => `@${why}`;
const SAMPLE = IGN("sample data in the mock");
const ANDROID = IGN("Android only (phase 17)");
const DESIGN_NOTE = IGN("design annotation, not app copy");
const OS = IGN("system dialog; the text comes from Info.plist / the OS");
const DEVICE = [["MacBook", "{{device}}"]];

export const MOBILE_MANIFEST = {
  // record (M2)
  title: "record.title",
  onPhone: "record.onPhone",
  muffled: "record.pocket.title",
  muffledSub: "record.pocket.hint",
  speaker: "record.speaker",
  identifying: "record.identifying",
  listening: "record.listening",
  speaking: { k: "record.speaking", v: { n: "name" } },
  mark: "record.mark",
  stop: "record.stop",
  pause: "record.pause",
  resume: "record.resume",
  recNow: "record.recording",
  pausedCall: "record.pausedCall.title",
  pausedCallS: { k: "record.pausedCall.body", v: { t: "time" } },
  resumeRec: "record.resumeRecording",
  stopSave: "record.stopAndSave",
  laSub: "ios.liveActivitySubtitle",
  lockDate: SAMPLE,

  // processing target
  finalOn: "target.finalOn",
  procOpts: IGN("superseded by procOpts2"),
  procOpts2: { items: ["target.phone", "target.desktop", "target.cloud"] },
  procHintMac: { k: "target.hintDesktop", sub: DEVICE },
  procHintPhone: "target.hintPhone",
  procDefault: "onboarding.processing.title",

  // shell / meetings (M3)
  tabs: { items: ["tabs.meetings", "tabs.record", "tabs.search", "tabs.settings"] },
  recTab: ">tabs.record",
  meetings: ">tabs.meetings",
  paired: { k: "meetings.paired", sub: DEVICE },
  searchPh: "search.placeholder",
  today: "meetings.today",
  yesterday: "meetings.yesterday",
  chips: { items: ["meetings.filter.all", "meetings.filter.needsNames", "meetings.filter.imported"] },
  stSynced: "chip.synced",
  stWifi: "chip.waitingForWifi",
  stFinal: { k: "chip.finalOnDesktop", sub: [["MacBook", "{{device}}"], ["62", "{{percent}}"]] },
  stPhone: "chip.processedOnPhone",
  stFail: "chip.failed",

  // common
  cont: "common.continue",
  skip: "common.skip",
  replay: "common.replay",
  notNow: "common.notNow",
  cancel: "common.cancel",
  openSettings: "common.openSettings",

  // onboarding (M1)
  obLangT: "onboarding.languages.title",
  obLangS: "onboarding.languages.subtitle",
  langOpts: { items: ["onboarding.languages.en", "onboarding.languages.vi", "onboarding.languages.both"] },
  obMicT: "onboarding.mic.title",
  obMicS: "onboarding.mic.body",
  osMic: OS,
  osMicS: OS,
  dontAllow: OS,
  allow: OS,
  micOk: "onboarding.mic.allowed",
  micOff: "onboarding.mic.denied",
  voiceOpt: "voice.title",
  voiceOptS: "voice.body",
  recordVoice: "voice.record",
  voiceConsent: "voice.consent",
  readAloud: "voice.start",
  listeningV: "voice.listening",
  voiceSaved: "voice.saved",
  pairT: "pair.title",
  pairS: "pair.body",
  wifi: "pair.wifiNote",
  scanning: "pair.scanning",
  pairedOk: { k: "pair.paired", sub: [["MacBook Pro", "{{device}}"]] },
  pairedOkPc: IGN("same string as pairedOk with another device name"),
  pairedSub: "pair.syncNote",
  doneT: "onboarding.done.title",
  doneS: "onboarding.done.body",
  startRec: "onboarding.done.start",
  doneRows: SAMPLE,

  // meeting view (M4)
  m4Meta: SAMPLE,
  m4Priv: SAMPLE,
  localOnly: "privacy.localOnly",
  localLine: "privacy.audioStaysLine",
  notes: "detail.notes",
  transcript: "detail.transcript",
  youWrote: "detail.youWrote",
  ghiWrote: "detail.aiWrote",
  playFrom: { k: "detail.playFrom", v: { t: "time" } },
  unassigned: "detail.unassigned",

  // import / share (M5)
  vmTitle: IGN("the source app's name in the mock"),
  importTo: "import.title",
  importBtn: "import.action",
  detected: SAMPLE,
  detectedPlaud: {
    k: "import.detected",
    tpl: ["Detected: {{source}} · {{duration}}", "Nhận diện: {{source}} · {{duration}}"],
  },
  lang: "import.language",
  langs4: { items: ["import.lang.auto", "import.lang.en", "import.lang.vi", "import.lang.both"] },
  processOn: "import.processOn",
  procTwo: IGN("superseded by procTwoPc"),
  procTwoPc: { items: ["import.target.phone", "import.target.desktop"] },
  importedT: "import.added",
  doneMac: { k: "import.doneDesktop", sub: DEVICE },
  donePhone: "import.donePhone",
  doneA: ANDROID,
  apps: IGN("apps in the mock share sheet"),
  shareActs: OS,
  aApps: ANDROID,

  // phone-call limit (M6)
  callLimitT: "callLimit.title",
  callLimit: "callLimit.body",
  consentRemind: "callLimit.consent",
  useRoom: "callLimit.useRoom",
  useRoomShort: "callLimit.useRoomShort",
  m6Ready: "callLimit.ready",

  // Android permissions, accessibility annotations, mock-only controls
  aPermT: ANDROID,
  aPermS: ANDROID,
  permMic: ANDROID,
  permMicW: ANDROID,
  permNotif: ANDROID,
  permNotifW: ANDROID,
  allowed: ANDROID,
  aPermNo: ANDROID,
  dlgMic: ANDROID,
  dlgNotif: ANDROID,
  whileUsing: ANDROID,
  onlyThis: ANDROID,
  dontAllowA: ANDROID,
  a11yT: DESIGN_NOTE,
  a11yS: DESIGN_NOTE,
  bigIosNote: DESIGN_NOTE,
  bigAndNote: ANDROID,
  pbHint: ANDROID,
  pbPlay: ANDROID,
  qs: ANDROID,
};
