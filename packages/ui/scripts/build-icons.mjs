// SPDX-License-Identifier: Apache-2.0
// Writes src/icons/icon-data.ts: SVG path data for the icons the app uses, so
// no icon font ships. mac: Material Symbols Rounded (Apache-2.0); Windows:
// Fluent System Icons 20 px Regular (MIT) where mapped (design notes), else
// the Material glyph. Add a name here and run: pnpm --filter @ghi/ui gen:icons
import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const pkgDir = (p) => dirname(require.resolve(`${p}/package.json`));
const MATERIAL = join(pkgDir("@material-symbols/svg-400"), "rounded");
const FLUENT = join(pkgDir("@fluentui/svg-icons"), "icons");

/**
 * name → [material file (rounded; "-fill" for filled), fluent file or null].
 * Names are Material Symbols names except where the set renamed a glyph.
 */
export const ICONS = {
  add: ["add", "add_20_regular"],
  arrow_back: ["arrow_back", "arrow_left_20_regular"],
  arrow_forward: ["arrow_forward", "arrow_right_20_regular"],
  arrow_upward: ["arrow_upward", "arrow_up_20_regular"],
  auto_awesome: ["wand_stars", "sparkle_20_regular"],
  battery_full: ["battery_full", "battery_10_20_regular"],
  block: ["block", "prohibited_20_regular"],
  bookmark: ["bookmark", "bookmark_20_regular"],
  calendar_today: ["calendar_today", "calendar_ltr_20_regular"],
  call_merge: ["call_merge", "merge_20_regular"],
  call_split: ["call_split", "branch_fork_20_regular"],
  campaign: ["campaign", "megaphone_20_regular"],
  check: ["check", "checkmark_20_regular"],
  check_box: ["check_box", "checkbox_checked_20_regular"],
  check_box_outline_blank: ["check_box_outline_blank", "checkbox_unchecked_20_regular"],
  check_circle: ["check_circle", "checkmark_circle_20_regular"],
  chevron_left: ["chevron_left", "chevron_left_20_regular"],
  chevron_right: ["chevron_right", "chevron_right_20_regular"],
  close: ["close", "dismiss_20_regular"],
  cloud: ["cloud", "cloud_20_regular"],
  cloud_done: ["cloud_done", "cloud_checkmark_20_regular"],
  cloud_off: ["cloud_off", "cloud_off_20_regular"],
  cloud_upload: ["cloud_upload", "cloud_arrow_up_20_regular"],
  content_copy: ["content_copy", "copy_20_regular"],
  crop_square: ["crop_square", "square_20_regular"],
  dark_mode: ["dark_mode", "weather_moon_20_regular"],
  delete: ["delete", "delete_20_regular"],
  delete_forever: ["delete_forever", "delete_20_regular"],
  description: ["description", "document_20_regular"],
  desktop_windows: ["desktop_windows", "desktop_20_regular"],
  do_not_disturb_on: ["do_not_disturb_on", "subtract_circle_20_regular"],
  done_all: ["done_all", "checkmark_20_regular"],
  download: ["download", "arrow_download_20_regular"],
  edit: ["edit", "edit_20_regular"],
  edit_off: ["edit_off", "edit_off_20_regular"],
  error: ["error", "error_circle_20_regular"],
  event_note: ["event_note", "calendar_ltr_20_regular"],
  event_upcoming: ["event_upcoming", "calendar_clock_20_regular"],
  expand_less: ["keyboard_arrow_up", "chevron_up_20_regular"],
  expand_more: ["keyboard_arrow_down", "chevron_down_20_regular"],
  face: ["face", "scan_person_20_regular"],
  fast_forward: ["fast_forward", "fast_forward_20_regular"],
  filter_list: ["filter_list", "filter_20_regular"],
  fingerprint: ["fingerprint", "fingerprint_20_regular"],
  flag: ["flag", "flag_20_regular"],
  folder: ["folder", "folder_20_regular"],
  forum: ["forum", "chat_multiple_20_regular"],
  graphic_eq: ["graphic_eq", "data_area_20_regular"],
  group: ["group", "people_20_regular"],
  groups: ["groups", "people_community_20_regular"],
  hard_drive: ["hard_drive", "hard_drive_20_regular"],
  headphones: ["headphones", "headphones_20_regular"],
  help: ["help", "question_circle_20_regular"],
  hourglass_top: ["hourglass_top", "hourglass_20_regular"],
  inbox: ["inbox", "mail_inbox_20_regular"],
  info: ["info", "info_20_regular"],
  ios_share: ["ios_share", "share_ios_20_regular"],
  keyboard_command_key: ["keyboard_command_key", null],
  label: ["label", "tag_20_regular"],
  language: ["language", "globe_20_regular"],
  laptop_mac: ["laptop_mac", "laptop_20_regular"],
  left_panel_close: ["left_panel_close", "panel_left_contract_20_regular"],
  left_panel_open: ["left_panel_open", "panel_left_expand_20_regular"],
  light_mode: ["light_mode", "weather_sunny_20_regular"],
  link_off: ["link_off", "link_dismiss_20_regular"],
  lock: ["lock", "lock_closed_20_regular"],
  manage_search: ["manage_search", "search_info_20_regular"],
  menu: ["menu", "navigation_20_regular"],
  mic: ["mic", "mic_20_regular"],
  mic_off: ["mic_off", "mic_off_20_regular"],
  mobile: ["mobile", "phone_20_regular"],
  more_horiz: ["more_horiz", "more_horizontal_20_regular"],
  music_note: ["music_note", "music_note_2_20_regular"],
  notifications: ["notifications", "alert_20_regular"],
  open_in_full: ["open_in_full", "arrow_maximize_20_regular"],
  open_in_new: ["open_in_new", "open_20_regular"],
  pause: ["pause", "pause_20_regular"],
  pause_circle: ["pause_circle", "pause_circle_20_regular"],
  person: ["person", "person_20_regular"],
  person_remove: ["person_remove", "person_delete_20_regular"],
  picture_in_picture_alt: ["picture_in_picture_alt", "window_20_regular"],
  play_arrow: ["play_arrow", "play_20_regular"],
  progress_activity: ["progress_activity", null],
  question_mark: ["question_mark", "question_20_regular"],
  radio_button_checked: ["radio_button_checked", "record_20_regular"],
  radio_button_unchecked: ["radio_button_unchecked", "circle_20_regular"],
  record_voice_over: ["record_voice_over", "person_voice_20_regular"],
  refresh: ["refresh", "arrow_clockwise_20_regular"],
  remove_moderator: ["remove_moderator", "shield_prohibited_20_regular"],
  remove: ["remove", "subtract_20_regular"],
  schedule: ["schedule", "clock_20_regular"],
  science: ["science", "beaker_20_regular"],
  search: ["search", "search_20_regular"],
  search_off: ["search_off", "search_20_regular"],
  sell: ["sell", "tag_20_regular"],
  settings: ["settings", "settings_20_regular"],
  speaker: ["speaker", "speaker_2_20_regular"],
  star: ["star-fill", "star_20_filled"],
  star_outline: ["star", "star_20_regular"],
  stop: ["stop-fill", "stop_20_filled"],
  subdirectory_arrow_right: ["subdirectory_arrow_right", "arrow_reply_20_regular"],
  subtitles: ["subtitles", "subtitles_20_regular"],
  swap_horiz: ["swap_horiz", "arrow_swap_20_regular"],
  sync: ["sync", "arrow_sync_20_regular"],
  system_update_alt: ["system_update_alt", "arrow_download_20_regular"],
  task_alt: ["task_alt", "task_list_ltr_20_regular"],
  timer: ["timer", "timer_20_regular"],
  translate: ["translate", "translate_20_regular"],
  update: ["update", "arrow_sync_circle_20_regular"],
  upload_file: ["upload_file", "arrow_upload_20_regular"],
  verified: ["verified", "checkmark_starburst_20_regular"],
  verified_user: ["verified_user", "shield_checkmark_20_regular"],
  videocam: ["videocam", "video_20_regular"],
  view_timeline: ["view_timeline", "timeline_20_regular"],
  visibility_off: ["visibility_off", "eye_off_20_regular"],
  volume_off: ["volume_off", "speaker_mute_20_regular"],
  volume_up: ["volume_up", "speaker_2_20_regular"],
  warning: ["warning", "warning_20_regular"],
  wifi: ["wifi", "wifi_1_20_regular"],
  wifi_off: ["wifi_off", "wifi_off_20_regular"],
};

/** Every `d` attribute of an SVG file. */
function paths(file) {
  const svg = readFileSync(file, "utf8");
  const ds = [...svg.matchAll(/<path[^>]*\sd="([^"]+)"/g)].map((m) => m[1]);
  if (!ds.length || /<(circle|rect|polygon|ellipse|g)\b/.test(svg)) throw new Error(`${file}: only <path> elements are supported`);
  return ds;
}

export function iconData() {
  const material = {};
  const fluent = {};
  for (const [name, [m, f]] of Object.entries(ICONS)) {
    material[name] = paths(join(MATERIAL, `${m}.svg`));
    if (f) fluent[name] = paths(join(FLUENT, `${f}.svg`));
  }
  const names = Object.keys(ICONS);
  return `// SPDX-License-Identifier: Apache-2.0
// Generated by scripts/build-icons.mjs: do not edit.
// Material Symbols (Apache-2.0, Google) and Fluent UI System Icons (MIT,
// Microsoft): see packages/ui/THIRD_PARTY_NOTICES.md.

export const ICON_NAMES = ${JSON.stringify(names)} as const;
export type IconName = (typeof ICON_NAMES)[number];

/** Material Symbols Rounded, weight 400 (viewBox 0 -960 960 960). */
export const MATERIAL: Record<IconName, string[]> = ${JSON.stringify(material)};

/** Fluent System Icons 20 px (viewBox 0 0 20 20); others fall back to Material. */
export const FLUENT: Partial<Record<IconName, string[]>> = ${JSON.stringify(fluent)};
`;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const out = join(dirname(fileURLToPath(import.meta.url)), "../src/icons/icon-data.ts");
  writeFileSync(out, iconData());
  console.log(`wrote ${Object.keys(ICONS).length} icons to src/icons/icon-data.ts`);
}
