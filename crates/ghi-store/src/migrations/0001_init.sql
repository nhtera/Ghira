-- SPDX-License-Identifier: Apache-2.0
-- Schema v1. Every syncable row has a `gid` (UUIDv7, `new_gid()`) beside its
-- integer rowid (the rowid is what the FTS tables use). Text columns ending in
-- `_ct` are XChaCha20-Poly1305 ciphertext under the meeting's DEK.
-- Timestamps are unix milliseconds.

CREATE TABLE meetings (
    id                  INTEGER PRIMARY KEY,
    gid                 TEXT NOT NULL UNIQUE,
    title_ct            BLOB,
    started_at          INTEGER NOT NULL,
    duration_ms         INTEGER NOT NULL DEFAULT 0,
    source              TEXT NOT NULL DEFAULT 'live',
    mode                TEXT NOT NULL DEFAULT 'meeting',
    lang                TEXT,
    template            TEXT,
    status              TEXT NOT NULL DEFAULT 'recording',
    privacy_state       TEXT NOT NULL DEFAULT 'local',
    cloud_locked        INTEGER NOT NULL DEFAULT 0,
    sensitive           INTEGER NOT NULL DEFAULT 0,
    consent_confirmed   INTEGER NOT NULL DEFAULT 0,
    cloud_used          INTEGER NOT NULL DEFAULT 0,
    transcript_version  INTEGER NOT NULL DEFAULT 1,
    dek_wrapped         BLOB NOT NULL,        -- zeroed = crypto-shredded
    audio_retained_until INTEGER,
    lamport             INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX meetings_started_at ON meetings(started_at);

CREATE TABLE tracks (
    id          INTEGER PRIMARY KEY,
    gid         TEXT NOT NULL UNIQUE,
    meeting_id  INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('mic', 'system', 'file')),
    page_count  INTEGER NOT NULL DEFAULT 0,
    UNIQUE (meeting_id, kind)
);

-- Persons are shared across meetings, so they can't be under one meeting's DEK:
-- `name` is protected by SQLCipher only (not crypto-shredded with a meeting).
CREATE TABLE persons (
    id          INTEGER PRIMARY KEY,
    gid         TEXT NOT NULL UNIQUE,
    name        TEXT NOT NULL,
    color_slot  INTEGER NOT NULL DEFAULT 0,
    lamport     INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE speakers (
    id            INTEGER PRIMARY KEY,
    gid           TEXT NOT NULL UNIQUE,
    meeting_id    INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    label_idx     INTEGER NOT NULL,
    display_name_ct BLOB,             -- sealed under the meeting DEK
    person_id     INTEGER REFERENCES persons(id) ON DELETE SET NULL,
    color_slot    INTEGER NOT NULL DEFAULT 0,
    is_me         INTEGER NOT NULL DEFAULT 0,
    merged_into   INTEGER REFERENCES speakers(id) ON DELETE SET NULL,
    lamport       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX speakers_meeting ON speakers(meeting_id);
CREATE INDEX speakers_person ON speakers(person_id);

CREATE TABLE segments (
    id          INTEGER PRIMARY KEY,
    gid         TEXT NOT NULL UNIQUE,
    meeting_id  INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    version     INTEGER NOT NULL,
    speaker_id  INTEGER REFERENCES speakers(id) ON DELETE SET NULL,
    t0_ms       INTEGER NOT NULL,
    t1_ms       INTEGER NOT NULL,
    text_ct     BLOB NOT NULL,
    lang        TEXT,
    confidence  REAL,
    edited      INTEGER NOT NULL DEFAULT 0,
    lamport     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX segments_meeting_time ON segments(meeting_id, version, t0_ms);

-- Word timing only; the word text lives inside segments.text_ct.
CREATE TABLE words (
    segment_id  INTEGER NOT NULL REFERENCES segments(id) ON DELETE CASCADE,
    idx         INTEGER NOT NULL,
    t0_ms       INTEGER NOT NULL,
    t1_ms       INTEGER NOT NULL,
    conf        REAL,
    PRIMARY KEY (segment_id, idx)
) WITHOUT ROWID;

CREATE TABLE notes_blocks (
    id           INTEGER PRIMARY KEY,
    gid          TEXT NOT NULL UNIQUE,
    meeting_id   INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL,
    provenance   TEXT NOT NULL CHECK (provenance IN ('user', 'ai', 'ai_edited')),
    body_ct      BLOB NOT NULL,
    anchors_json TEXT NOT NULL DEFAULT '[]',
    pinned       INTEGER NOT NULL DEFAULT 0,
    lamport      INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX notes_blocks_meeting ON notes_blocks(meeting_id);

CREATE TABLE action_items (
    id                INTEGER PRIMARY KEY,
    gid               TEXT NOT NULL UNIQUE,
    meeting_id        INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    text_ct           BLOB NOT NULL,
    owner_speaker_id  INTEGER REFERENCES speakers(id) ON DELETE SET NULL,
    due               INTEGER,
    done              INTEGER NOT NULL DEFAULT 0,
    anchor_json       TEXT,
    lamport           INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX action_items_meeting ON action_items(meeting_id);

CREATE TABLE marks (
    id          INTEGER PRIMARY KEY,
    gid         TEXT NOT NULL UNIQUE,
    meeting_id  INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    t_ms        INTEGER NOT NULL,
    tag         TEXT NOT NULL CHECK (tag IN ('star', 'decision', 'action', 'question')),
    lamport     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX marks_meeting ON marks(meeting_id);

-- Biometric data; each profile has its own wrapped key (phase 14 fills these).
CREATE TABLE voice_profiles (
    id           INTEGER PRIMARY KEY,
    gid          TEXT NOT NULL UNIQUE,
    person_id    INTEGER REFERENCES persons(id) ON DELETE CASCADE,
    is_me        INTEGER NOT NULL DEFAULT 0,
    consent_json TEXT NOT NULL,
    key_wrapped  BLOB NOT NULL,
    created_at   INTEGER NOT NULL
);

CREATE TABLE voice_embeddings (
    profile_id  INTEGER NOT NULL REFERENCES voice_profiles(id) ON DELETE CASCADE,
    lang        TEXT NOT NULL,
    vec_ct      BLOB NOT NULL,
    PRIMARY KEY (profile_id, lang)
) WITHOUT ROWID;

-- Never garbage-collected: drives sync deletes (phase 15).
CREATE TABLE tombstones (
    gid         TEXT PRIMARY KEY,
    kind        TEXT NOT NULL,
    lamport     INTEGER NOT NULL,
    deleted_at  INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE jobs (
    id               INTEGER PRIMARY KEY,
    meeting_id       INTEGER REFERENCES meetings(id) ON DELETE CASCADE,
    kind             TEXT NOT NULL,
    state            TEXT NOT NULL DEFAULT 'queued'
                     CHECK (state IN ('queued', 'running', 'done', 'failed', 'cancelled')),
    progress         REAL NOT NULL DEFAULT 0,
    attempts         INTEGER NOT NULL DEFAULT 0,
    payload_version  INTEGER NOT NULL DEFAULT 1,
    payload_json     TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX jobs_state ON jobs(state, kind);

CREATE TABLE cloud_requests (
    id          INTEGER PRIMARY KEY,
    meeting_id  INTEGER REFERENCES meetings(id) ON DELETE CASCADE,
    provider    TEXT NOT NULL,
    model       TEXT NOT NULL,
    tokens_in   INTEGER NOT NULL DEFAULT 0,
    tokens_out  INTEGER NOT NULL DEFAULT 0,
    at          INTEGER NOT NULL
);

CREATE TABLE settings (
    key         TEXT PRIMARY KEY,
    value_json  TEXT NOT NULL
) WITHOUT ROWID;

-- Sync clock: every write to a syncable row takes the next value.
INSERT INTO settings (key, value_json) VALUES ('lamport', '0');

-- Contentless indexes over folded text (rowid = segments.id / notes_blocks.id).
-- They hold tokens, not text. See `store::Store::delete_meeting` for the known
-- limit on residual tokens after a delete.
CREATE VIRTUAL TABLE segments_fts USING fts5(
    text_norm, content='', contentless_delete=1,
    tokenize='unicode61 remove_diacritics 0'
);
CREATE VIRTUAL TABLE notes_fts USING fts5(
    body_norm, content='', contentless_delete=1,
    tokenize='unicode61 remove_diacritics 0'
);
