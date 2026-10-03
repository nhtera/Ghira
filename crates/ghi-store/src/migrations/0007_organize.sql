-- SPDX-License-Identifier: Apache-2.0
-- Schema v7 (phase 14d): folders and tags, the detected source app, calendar
-- info, per-participant track speech spans and persisted overlap. Additive only.

-- One optional folder per meeting. Names are plaintext under SQLCipher (like
-- persons); `name_key` is the name as NFC, trimmed and lowercased (accents
-- count: "Họp" and "Hộp" differ), unique.
CREATE TABLE folders (
    id          INTEGER PRIMARY KEY,
    gid         TEXT NOT NULL UNIQUE,
    name        TEXT NOT NULL,
    name_key    TEXT NOT NULL UNIQUE,
    created_at  INTEGER NOT NULL,
    lamport     INTEGER NOT NULL DEFAULT 0
);
ALTER TABLE meetings ADD COLUMN folder_id INTEGER REFERENCES folders(id) ON DELETE SET NULL;
CREATE INDEX meetings_folder ON meetings(folder_id) WHERE folder_id IS NOT NULL;

-- Many tags per meeting. A link row has its own gid, fresh each time a tag is
-- added again, so a sync tombstone for an older link can't hide a new one.
CREATE TABLE tags (
    id          INTEGER PRIMARY KEY,
    gid         TEXT NOT NULL UNIQUE,
    name        TEXT NOT NULL,
    name_key    TEXT NOT NULL UNIQUE,
    created_at  INTEGER NOT NULL,
    lamport     INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE meeting_tags (
    meeting_id  INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    tag_id      INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    gid         TEXT NOT NULL UNIQUE,
    lamport     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (meeting_id, tag_id)
) WITHOUT ROWID;
CREATE INDEX meeting_tags_tag ON meeting_tags(tag_id);

-- zoom | teams | meet | plaud | voice_memos (NULL: unknown or recorded live).
ALTER TABLE meetings ADD COLUMN source_app TEXT;
-- Sealed under the meeting key (AAD `meetings.calendar_ct:{gid}`): JSON with the
-- event, attendees and calendar of the event a recording was started from.
ALTER TABLE meetings ADD COLUMN calendar_ct BLOB;
-- Sealed likewise (`meetings.track_speakers_ct:{gid}`): per-participant speech
-- spans of a multi-track import, used as ground-truth speakers.
ALTER TABLE meetings ADD COLUMN track_speakers_ct BLOB;
-- The line was spoken over by another speaker.
ALTER TABLE segments ADD COLUMN overlap INTEGER NOT NULL DEFAULT 0;
