-- SPDX-License-Identifier: Apache-2.0
-- Schema v8 (resumable final pass).

-- What a final pass finished so far (one row per ASR chunk of a track, and one
-- for the diarization), so a pass that yielded or was killed carries on where
-- it stopped. Derived from the audio, so it is sealed under the meeting DEK
-- (crypto-shred makes it unreadable; the rows go with the meeting) and removed
-- with the audio. `stamp` names what the rows were computed from (audio,
-- engine, chunking): rows of another stamp are never served and are dropped.
CREATE TABLE final_pass_ckpt (
    meeting_id  INTEGER NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    part        TEXT NOT NULL,
    stamp       TEXT NOT NULL,
    data_ct     BLOB NOT NULL,
    PRIMARY KEY (meeting_id, part)
) WITHOUT ROWID;
