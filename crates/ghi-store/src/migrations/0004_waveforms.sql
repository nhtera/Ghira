-- SPDX-License-Identifier: Apache-2.0
-- Schema v4 (phase 11, meeting detail and retention).

-- The audio bar's waveform: loudness per 100 ms, computed once from the
-- audio and sealed under the meeting DEK (it is derived from the audio).
-- Removed with the audio (retention) and with the meeting.
CREATE TABLE waveforms (
    meeting_id  INTEGER PRIMARY KEY REFERENCES meetings(id) ON DELETE CASCADE,
    data_ct     BLOB NOT NULL
);

-- When the meeting (and its audio) came into this store: retention counts
-- from the later of this and `started_at`, so an old file imported today
-- keeps its audio for the whole period.
ALTER TABLE meetings ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0;
UPDATE meetings SET created_at = started_at;
