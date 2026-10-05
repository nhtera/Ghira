-- SPDX-License-Identifier: Apache-2.0
-- Schema v9 (phase 15, LAN sync; doc 07 §7.2). Run by a Rust step
-- (`migrate::sync_v9`), which then backfills `ord`, `sync.feed_id`,
-- `sync.device_gid` and `sync_log`, and creates the `sync_log` triggers.
-- Additive only: new tables, and columns that are NULL or defaulted.

-- Paired devices (pins). `role`: the peer's role. `state` 'wipe_pending' means
-- only a session that delivers Wipe is allowed. `static_pub` is the peer's
-- X25519 key. `pair_psk_wrapped` is wrapped by the KeyRing (AAD `device:<gid>`).
-- push_seq / pull_seq are the cursors, `pull_feed_id` the feed they belong to.
CREATE TABLE devices (
    id               INTEGER PRIMARY KEY,
    gid              TEXT NOT NULL UNIQUE,
    name             TEXT NOT NULL,
    platform         TEXT NOT NULL,
    role             TEXT NOT NULL CHECK (role IN ('hub', 'spoke')),
    static_pub       BLOB NOT NULL UNIQUE,
    key_alg          TEXT NOT NULL DEFAULT 'x25519',
    pair_psk_wrapped BLOB NOT NULL,
    state            TEXT NOT NULL DEFAULT 'paired' CHECK (state IN ('paired', 'wipe_pending')),
    paired_at        INTEGER NOT NULL,
    last_seen        INTEGER,
    last_addr        TEXT,
    push_seq         INTEGER NOT NULL DEFAULT 0,
    pull_feed_id     TEXT,
    pull_seq         INTEGER NOT NULL DEFAULT 0
);

-- The change feed: the latest change per gid (a tombstone replaces its row's
-- entry). Written by the triggers below; `seq` is the cursor.
CREATE TABLE sync_log (
    seq   INTEGER PRIMARY KEY AUTOINCREMENT,
    kind  TEXT NOT NULL,
    gid   TEXT NOT NULL UNIQUE
);

-- Final-pass / notes leases (doc 07 §8). `role`: this device is the grantor
-- (the phone) or the holder (the desktop). Deadlines are durations on a
-- sleep-inclusive clock plus the boot they were measured in.
CREATE TABLE leases (
    id               INTEGER PRIMARY KEY,
    job_uuid         TEXT NOT NULL UNIQUE,
    meeting_gid      TEXT NOT NULL,
    role             TEXT NOT NULL CHECK (role IN ('grantor', 'holder')),
    epoch            INTEGER NOT NULL,
    kinds            TEXT NOT NULL DEFAULT '[]',
    state            TEXT NOT NULL,
    ttl_ms           INTEGER NOT NULL,
    deadline_cont_ns INTEGER,
    boot_id          TEXT,
    wall_deadline_ms INTEGER,
    progress         REAL NOT NULL DEFAULT 0,
    peer_id          INTEGER REFERENCES devices(id) ON DELETE SET NULL
);
CREATE INDEX leases_meeting ON leases(meeting_gid);

-- Every meeting exchanged with a peer, in either direction: the scope of a
-- Wipe, and whether its DEK has gone out.
CREATE TABLE peer_meetings (
    device_id    INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    meeting_gid  TEXT NOT NULL,
    key_sent     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (device_id, meeting_gid)
) WITHOUT ROWID;

-- The losing side of a concurrent free-text edit. `value_ct` is sealed under
-- the meeting DEK (AAD `conflict_copies.value_ct:{gid}`) and goes with it.
CREATE TABLE conflict_copies (
    id            INTEGER PRIMARY KEY,
    gid           TEXT NOT NULL UNIQUE,
    meeting_id    INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    target_kind   TEXT NOT NULL,
    target_gid    TEXT NOT NULL,
    field         TEXT NOT NULL,
    value_ct      BLOB NOT NULL,
    lamport       INTEGER NOT NULL DEFAULT 0,
    origin        INTEGER,
    base_lamport  INTEGER,
    base_origin   INTEGER,
    created_at    INTEGER NOT NULL
);
CREATE INDEX conflict_copies_meeting ON conflict_copies(meeting_id);
CREATE INDEX conflict_copies_target ON conflict_copies(target_gid);

-- Synced settings: one row per allowlisted key, last writer wins. `gid` is
-- UUIDv5(key), so every device names a key the same way.
CREATE TABLE synced_settings (
    id            INTEGER PRIMARY KEY,
    gid           TEXT NOT NULL UNIQUE,
    key           TEXT NOT NULL UNIQUE,
    value_json    TEXT NOT NULL,
    lamport       INTEGER NOT NULL DEFAULT 0,
    origin        INTEGER,
    base_lamport  INTEGER,
    base_origin   INTEGER
);

-- Records that arrived before their parent (bounded: 10 000 rows / 7 days).
CREATE TABLE sync_pending (
    gid          TEXT PRIMARY KEY,
    kind         TEXT NOT NULL,
    parent_gid   TEXT NOT NULL,
    record       BLOB NOT NULL,
    received_at  INTEGER NOT NULL
) WITHOUT ROWID;
CREATE INDEX sync_pending_parent ON sync_pending(parent_gid);

-- Where a row came from (`devices.id`; NULL = this device) and the version of
-- the hub's copy it was last based on (spoke only; doc 07 §7.3).
ALTER TABLE meetings ADD COLUMN origin INTEGER;
ALTER TABLE meetings ADD COLUMN base_lamport INTEGER;
ALTER TABLE meetings ADD COLUMN base_origin INTEGER;
ALTER TABLE tracks ADD COLUMN origin INTEGER;
ALTER TABLE tracks ADD COLUMN base_lamport INTEGER;
ALTER TABLE tracks ADD COLUMN base_origin INTEGER;
ALTER TABLE persons ADD COLUMN origin INTEGER;
ALTER TABLE persons ADD COLUMN base_lamport INTEGER;
ALTER TABLE persons ADD COLUMN base_origin INTEGER;
ALTER TABLE speakers ADD COLUMN origin INTEGER;
ALTER TABLE speakers ADD COLUMN base_lamport INTEGER;
ALTER TABLE speakers ADD COLUMN base_origin INTEGER;
ALTER TABLE segments ADD COLUMN origin INTEGER;
ALTER TABLE segments ADD COLUMN base_lamport INTEGER;
ALTER TABLE segments ADD COLUMN base_origin INTEGER;
ALTER TABLE notes_blocks ADD COLUMN origin INTEGER;
ALTER TABLE notes_blocks ADD COLUMN base_lamport INTEGER;
ALTER TABLE notes_blocks ADD COLUMN base_origin INTEGER;
ALTER TABLE action_items ADD COLUMN origin INTEGER;
ALTER TABLE action_items ADD COLUMN base_lamport INTEGER;
ALTER TABLE action_items ADD COLUMN base_origin INTEGER;
ALTER TABLE marks ADD COLUMN origin INTEGER;
ALTER TABLE marks ADD COLUMN base_lamport INTEGER;
ALTER TABLE marks ADD COLUMN base_origin INTEGER;
ALTER TABLE voice_profiles ADD COLUMN origin INTEGER;
ALTER TABLE voice_profiles ADD COLUMN base_lamport INTEGER;
ALTER TABLE voice_profiles ADD COLUMN base_origin INTEGER;
ALTER TABLE folders ADD COLUMN origin INTEGER;
ALTER TABLE folders ADD COLUMN base_lamport INTEGER;
ALTER TABLE folders ADD COLUMN base_origin INTEGER;
ALTER TABLE tags ADD COLUMN origin INTEGER;
ALTER TABLE tags ADD COLUMN base_lamport INTEGER;
ALTER TABLE tags ADD COLUMN base_origin INTEGER;
ALTER TABLE meeting_tags ADD COLUMN origin INTEGER;
ALTER TABLE meeting_tags ADD COLUMN base_lamport INTEGER;
ALTER TABLE meeting_tags ADD COLUMN base_origin INTEGER;

-- A track has its own version (finish, remove audio, cut) and `cut_pages`
-- (min-merged: the audio kept after a discard).
ALTER TABLE tracks ADD COLUMN lamport INTEGER NOT NULL DEFAULT 0;
ALTER TABLE tracks ADD COLUMN cut_pages INTEGER;

-- Why a row was deleted: user | meeting | regenerate | discard | retention |
-- transcript | superseded (NULL: written before this version), and who did it.
ALTER TABLE tombstones ADD COLUMN cause TEXT;
ALTER TABLE tombstones ADD COLUMN origin INTEGER;

-- The device that recorded a meeting (NULL: this one, or an import), and the
-- fencing epochs of its final-pass and AI results (doc 07 §8).
ALTER TABLE meetings ADD COLUMN audio_origin INTEGER;
ALTER TABLE meetings ADD COLUMN transcript_epoch INTEGER NOT NULL DEFAULT 0;
ALTER TABLE meetings ADD COLUMN ai_epoch INTEGER NOT NULL DEFAULT 0;
ALTER TABLE segments ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0;
ALTER TABLE notes_blocks ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0;
ALTER TABLE action_items ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0;

-- Display order of notes and action items: a fractional index compared as
-- bytes (rowids differ across devices). Backfilled by the Rust step.
ALTER TABLE notes_blocks ADD COLUMN ord TEXT;
ALTER TABLE action_items ADD COLUMN ord TEXT;
