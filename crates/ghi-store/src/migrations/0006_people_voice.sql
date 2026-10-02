-- SPDX-License-Identifier: Apache-2.0
-- Schema v6 (phase 14c, people and voice profiles). Run by a Rust step
-- (`migrate::people_voice`), which also creates the Me person and links Me
-- speakers to it.

-- Persons: Me is a row (`is_me`, empty name) so counts and filters are uniform.
-- `name` stays plaintext under SQLCipher only (see 0001).
ALTER TABLE persons ADD COLUMN is_me INTEGER NOT NULL DEFAULT 0;
ALTER TABLE persons ADD COLUMN created_at INTEGER;
CREATE UNIQUE INDEX persons_me ON persons(is_me) WHERE is_me = 1;
-- The comparison form of the name (`people::name_key`: NFC, trimmed, lowercase;
-- NULL for Me). Filled by the Rust step, which also merges duplicates.
ALTER TABLE persons ADD COLUMN name_key TEXT;
CREATE UNIQUE INDEX persons_name_key ON persons(name_key) WHERE is_me = 0;

-- One voice profile per person; its key is wrapped by the KeyRing (AAD
-- `voice:{gid}`) and zeroed on delete (crypto-shred).
ALTER TABLE voice_profiles ADD COLUMN consent_clip_ct BLOB;
ALTER TABLE voice_profiles ADD COLUMN updated_at INTEGER;
ALTER TABLE voice_profiles ADD COLUMN lamport INTEGER NOT NULL DEFAULT 0;
CREATE UNIQUE INDEX voice_profiles_person ON voice_profiles(person_id);

-- Never written before this version (the step asserts it).
DROP TABLE voice_embeddings;
CREATE TABLE voice_embeddings (
    profile_id  INTEGER NOT NULL REFERENCES voice_profiles(id) ON DELETE CASCADE,
    model       TEXT NOT NULL,
    lang        TEXT NOT NULL,
    dim         INTEGER NOT NULL,
    n           INTEGER NOT NULL,
    -- centroid, exemplars and exemplar sources, sealed under the profile key.
    vec_ct      BLOB NOT NULL,
    PRIMARY KEY (profile_id, model, lang)
) WITHOUT ROWID;

-- "Sounds like ..." suggestions from the final pass.
ALTER TABLE speakers ADD COLUMN suggest_person_id INTEGER REFERENCES persons(id) ON DELETE SET NULL;
ALTER TABLE speakers ADD COLUMN suggest_score REAL;

CREATE INDEX speakers_suggest ON speakers(suggest_person_id) WHERE suggest_person_id IS NOT NULL;
CREATE INDEX action_items_owner ON action_items(owner_speaker_id);

-- Bumped whenever something a chunk's text is built from changes (speaker
-- names, line text or owner). An indexer stores vectors only for the
-- generation it read, and a row built from an older one is stale.
ALTER TABLE meetings ADD COLUMN index_gen INTEGER NOT NULL DEFAULT 0;
ALTER TABLE embeddings ADD COLUMN index_gen INTEGER NOT NULL DEFAULT 0;

-- A person with a live voice profile can't be deleted by accident: delete
-- the profile first (zeroing its key), which is what the store does.
CREATE TRIGGER persons_keep_voice BEFORE DELETE ON persons
WHEN EXISTS (SELECT 1 FROM voice_profiles v
             WHERE v.person_id = OLD.id AND v.key_wrapped <> zeroblob(length(v.key_wrapped)))
BEGIN
    SELECT RAISE(ABORT, 'person still has a voice profile');
END;

-- Cluster voices of unnamed speakers (third-party profiles only), sealed under
-- the meeting DEK and removed with the speaker.
CREATE TABLE speaker_voices (
    speaker_id  INTEGER PRIMARY KEY REFERENCES speakers(id) ON DELETE CASCADE,
    model       TEXT NOT NULL,
    lang        TEXT NOT NULL,
    dim         INTEGER NOT NULL,
    vec_ct      BLOB NOT NULL
);
