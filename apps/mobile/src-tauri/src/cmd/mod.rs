// SPDX-License-Identifier: Apache-2.0
//! The mobile command surface (phase 16 contracts). Mobile-owned commands
//! live in the modules below (bodies arrive in W1: 16-D record, 16-G the
//! rest); everything that reads or edits stored meetings is the `ghi-app`
//! command, registered here by path so the DTOs match the desktop's.
//!
//! Left out on purpose: `regenerate_notes` and `list_templates` (the phone never
//! runs local notes, D5: regenerating would leave a meeting stuck in
//! processing; cloud notes go through the cloud send sheet), `ask_*`, people,
//! folders, tags, the calendar's ICS file and ticker (EventKit only, see
//! `calendar`) and the desktop import queue (the phone imports
//! through the share inbox).
//!
//! The list in [`builder`] must match `build.rs` (`.commands(..)`) and
//! `capabilities/default.json`; `commands_are_granted` in `lib.rs` checks it.

pub mod calendar;
pub mod events;
pub mod import;
pub mod lifecycle;
pub mod meetings;
pub mod models;
pub mod onboarding;
pub mod privacy;
pub mod record;
pub mod settings;
pub mod store;
pub mod types;
pub mod voice;

pub use events::MobileEvent;

use ghi_app::core::CoreEvent;
use tauri_specta::{Builder, collect_commands, collect_events};

pub fn builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            // Mobile-owned.
            record::record_start,
            record::record_stop,
            record::record_pause,
            record::record_resume,
            record::record_mark,
            record::record_snapshot,
            record::record_consent_message,
            record::record_call_active,
            record::record_resume_prompt,
            record::record_set_sensitive,
            record::record_discard_preview,
            record::record_discard_from,
            calendar::calendar_status,
            calendar::calendar_connect,
            calendar::calendar_disconnect,
            calendar::calendar_current_event,
            lifecycle::app_version,
            lifecycle::lifecycle_state,
            lifecycle::device_tier,
            lifecycle::open_app_settings,
            models::models_status,
            models::models_download,
            models::models_cancel,
            privacy::privacy_export_all_share,
            privacy::privacy_delete_all,
            store::store_status,
            store::store_start_fresh,
            store::log_ui_failure,
            meetings::meeting_chips,
            meetings::share_meeting_export,
            settings::mobile_settings,
            settings::set_mobile_settings,
            import::inbox_list,
            import::inbox_confirm,
            import::inbox_dismiss,
            onboarding::onboarding_state,
            onboarding::onboarding_complete_step,
            onboarding::mic_permission,
            onboarding::request_mic_permission,
            voice::voice_set_consent,
            voice::voice_enroll_start,
            voice::voice_enroll_stop,
            voice::voice_enroll_cancel,
            voice::voice_delete_me,
            // Shared with the desktop (ghi-app).
            ghi_app::library::list_meetings,
            ghi_app::library::set_meeting_title,
            ghi_app::library::delete_meeting,
            ghi_app::library::retry_meeting,
            ghi_app::library::take_recovered_meetings,
            ghi_app::library::known_speaker_names,
            ghi_app::library::set_meeting_sensitive,
            ghi_app::speakers_cmd::meeting_speakers,
            ghi_app::speakers_cmd::rename_meeting_speaker,
            ghi_app::speakers_cmd::set_speaker_me,
            ghi_app::speakers_cmd::clear_speaker_me,
            ghi_app::speakers_cmd::merge_meeting_speakers,
            ghi_app::speakers_cmd::split_meeting_speaker,
            ghi_app::speakers_cmd::set_speaker_not_person,
            ghi_app::detail::meeting_detail,
            ghi_app::detail::meeting_notes,
            ghi_app::detail::meeting_transcript,
            ghi_app::detail::update_segment_text,
            ghi_app::detail::set_segment_speaker,
            ghi_app::detail::update_note_block,
            ghi_app::detail::add_note_block,
            ghi_app::detail::delete_note_block,
            ghi_app::detail::add_action_item,
            ghi_app::detail::update_action_item,
            ghi_app::detail::set_action_done,
            ghi_app::detail::set_action_owner,
            ghi_app::detail::delete_action_item,
            ghi_app::detail::search_meetings,
            ghi_app::detail::retranscribe,
            ghi_app::audio_protocol::issue_audio_play,
            ghi_app::audio_protocol::waveform_peaks,
            ghi_app::export_cmd::meeting_as_text,
            ghi_app::cloud_cmd::cloud_keys,
            ghi_app::cloud_cmd::set_cloud_key,
            ghi_app::cloud_cmd::delete_cloud_key,
            ghi_app::cloud_cmd::cloud_models,
            ghi_app::cloud_cmd::cloud_preview,
            ghi_app::cloud_cmd::cloud_send,
            ghi_app::cloud_cmd::set_meeting_cloud_locked,
            ghi_app::cloud_cmd::cloud_request_log,
            ghi_app::system::get_settings,
            ghi_app::system::update_settings,
            ghi_app::settings_cmd::vocabulary,
            ghi_app::settings_cmd::set_vocabulary,
            ghi_app::settings_cmd::ignore_learned_term,
            ghi_app::lock_cmd::lock_state,
            ghi_app::lock_cmd::lock_now,
            ghi_app::lock_cmd::unlock,
            ghi_app::lock_cmd::set_app_lock,
            ghi_app::voice_cmd::voice_status,
            ghi_app::voice_cmd::enroll_voice_level,
            ghi_app::calendar_cmd::meeting_attendees,
            ghi_app::calendar_cmd::meeting_contacts,
            ghi_app::sync_cmd::sync_status,
            ghi_app::sync_cmd::sync_set_enabled,
            ghi_app::sync_cmd::sync_devices,
            ghi_app::sync_cmd::sync_unpair,
            ghi_app::sync_cmd::sync_unpair_and_wipe,
            ghi_app::sync_cmd::sync_now,
            ghi_app::sync_cmd::sync_conflicts,
            ghi_app::sync_cmd::sync_conflict_resolve,
            ghi_app::sync_cmd::sync_confirm_mass_delete,
            ghi_app::sync_cmd::sync_delete_everywhere_status,
            ghi_app::sync_cmd::sync_delete_everywhere_skip,
            ghi_app::sync_cmd::sync_pair_scan_start,
            ghi_app::sync_cmd::sync_pair_scan_stop,
            ghi_app::sync_cmd::sync_lease_revoke,
        ])
        .events(collect_events![
            CoreEvent,
            MobileEvent,
            ghi_app::lock_cmd::LockChanged,
            ghi_app::sync_cmd::SyncEvent
        ])
}
