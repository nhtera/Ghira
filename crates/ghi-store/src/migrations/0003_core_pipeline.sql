-- SPDX-License-Identifier: Apache-2.0
-- Schema v3 (phase 8, core pipeline).

-- "Not a person" (a TV, a notification sound): kept, but not a speaker in notes.
ALTER TABLE speakers ADD COLUMN not_person INTEGER NOT NULL DEFAULT 0;

-- SHA-256 of an imported file, for duplicate detection; NULL for live meetings.
ALTER TABLE meetings ADD COLUMN source_hash TEXT;
CREATE INDEX meetings_source_hash ON meetings(source_hash) WHERE source_hash IS NOT NULL;

-- Discard [RT-1]. The text side is deleted in the same transaction that adds
-- this row; the audio side (rotating each track's bundle down to the kept
-- pages) completes it and is redone at startup while `audio_state` is pending.
-- The timeline is not collapsed: later times stay valid and the span plays as
-- silence.
CREATE TABLE discards (
    id              INTEGER PRIMARY KEY,
    meeting_id      INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    t0_ms           INTEGER NOT NULL,
    t1_ms           INTEGER NOT NULL,
    -- {"mic": pages kept, "system": pages kept}
    keep_pages_json TEXT NOT NULL DEFAULT '{}',
    audio_state     TEXT NOT NULL DEFAULT 'pending' CHECK (audio_state IN ('pending', 'done')),
    created_at      INTEGER NOT NULL
);
CREATE INDEX discards_meeting ON discards(meeting_id);
