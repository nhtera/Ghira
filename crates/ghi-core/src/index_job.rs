// SPDX-License-Identifier: Apache-2.0
//! `embed_index`: embeds a finished meeting's transcript for semantic search.
//!
//! The transcript is cut into ~60 s chunks on segment boundaries (text is
//! "Speaker: text" lines, at most [`MAX_CHARS`] characters), embedded in
//! batches and stored sealed ([`Store::put_embeddings`]). It is queued after
//! `notes_final`, and by [`queue_missing`] for meetings that have none (at
//! startup, and when the embedding model arrives). Like every job it waits
//! while the model is missing, yields to a recording, and is rebuilt from
//! scratch when resumed (a few seconds of work).

use std::sync::Arc;

use ghi_llm::embed::{Embedder, Kind};
use ghi_store::embeddings::EmbeddingChunk;
use ghi_store::store::Store;

use crate::jobs::{JobCtx, JobHandler, Outcome, Ready};
use crate::notes_job::speaker_label;
use crate::session::JOB_PAYLOAD_VERSION;

pub const EMBED_INDEX_JOB: &str = "embed_index";
/// Registry id of the embedding model the app indexes with.
pub const MODEL_ID: &str = "qwen3-embedding-0.6b";
/// A chunk spans about this long (it closes at the first segment boundary
/// past it).
pub const CHUNK_MS: i64 = 60_000;
/// A chunk's text is at most this many characters.
pub const MAX_CHARS: usize = 1_500;
/// Texts per embedding request, and the points where the job checks for a
/// recording.
const BATCH: usize = 16;

fn store_err(e: ghi_store::StoreError) -> String {
    e.to_string()
}

/// One piece of the cut transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub t0_ms: i64,
    pub t1_ms: i64,
    pub text: String,
}

/// Cuts `lines` (t0, t1, "Speaker: text") into chunks on line boundaries: a
/// chunk closes once it spans [`CHUNK_MS`] or the next line would take it past
/// [`MAX_CHARS`]. A single line longer than that is split by characters.
pub fn chunk_lines(lines: &[(i64, i64, String)]) -> Vec<Chunk> {
    let mut out = Vec::new();
    let mut cur: Option<Chunk> = None;
    let mut cur_chars = 0usize;
    let mut flush = |cur: &mut Option<Chunk>, cur_chars: &mut usize| {
        if let Some(c) = cur.take() {
            out.push(c);
        }
        *cur_chars = 0;
    };
    for (t0, t1, text) in lines {
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        let chars = text.chars().count();
        if chars > MAX_CHARS {
            flush(&mut cur, &mut cur_chars);
            let all: Vec<char> = text.chars().collect();
            for piece in all.chunks(MAX_CHARS) {
                out_push(&mut cur, *t0, *t1, &piece.iter().collect::<String>());
                flush(&mut cur, &mut cur_chars);
            }
            continue;
        }
        // +1 for the newline between lines.
        if cur.is_some() && cur_chars + 1 + chars > MAX_CHARS {
            flush(&mut cur, &mut cur_chars);
        }
        match &mut cur {
            Some(c) => {
                c.t1_ms = c.t1_ms.max(*t1);
                c.text.push('\n');
                c.text.push_str(text);
                cur_chars += 1 + chars;
            }
            None => {
                cur = Some(Chunk {
                    t0_ms: *t0,
                    t1_ms: *t1,
                    text: text.to_string(),
                });
                cur_chars = chars;
            }
        }
        if cur.as_ref().is_some_and(|c| c.t1_ms - c.t0_ms >= CHUNK_MS) {
            flush(&mut cur, &mut cur_chars);
        }
    }
    flush(&mut cur, &mut cur_chars);
    out
}

fn out_push(cur: &mut Option<Chunk>, t0: i64, t1: i64, text: &str) {
    *cur = Some(Chunk {
        t0_ms: t0,
        t1_ms: t1,
        text: text.to_string(),
    });
}

/// The meeting's transcript as chunks, with the transcript version they were
/// cut from (read first: a replacement in between only makes the rows stale).
pub fn meeting_chunks(store: &Store, meeting: &str) -> Result<(i64, Vec<Chunk>), String> {
    let version = store
        .get_meeting(meeting)
        .map_err(store_err)?
        .transcript_version;
    let names: std::collections::HashMap<String, String> = store
        .speakers(meeting)
        .map_err(store_err)?
        .into_iter()
        .map(|s| {
            let label = speaker_label(&s);
            (s.gid, label)
        })
        .collect();
    let lines: Vec<(i64, i64, String)> = store
        .segments(meeting)
        .map_err(store_err)?
        .into_iter()
        .map(|s| {
            let text = match s.speaker_gid.as_ref().and_then(|g| names.get(g)) {
                Some(who) => format!("{who}: {}", s.text.trim()),
                None => s.text,
            };
            (s.t0_ms, s.t1_ms, text)
        })
        .collect();
    Ok((version, chunk_lines(&lines)))
}

/// Opens the embedding model.
pub type EmbedderFactory = Arc<dyn Fn() -> Result<Box<dyn Embedder + Send>, String> + Send + Sync>;

pub struct IndexJob {
    pub embedder: EmbedderFactory,
    /// The embedding model is installed (else the job waits for it).
    pub ready: Ready,
}

impl JobHandler for IndexJob {
    fn kind(&self) -> &'static str {
        EMBED_INDEX_JOB
    }

    fn ready(&self) -> bool {
        (self.ready)()
    }

    fn run(&self, ctx: &JobCtx) -> Result<Outcome, String> {
        if ctx.preempted() {
            return Ok(Outcome::Yield(ctx.job.payload.clone()));
        }
        let meeting = ctx.meeting()?;
        let (version, chunks) = meeting_chunks(ctx.store, meeting)?;
        if chunks.is_empty() {
            return Ok(Outcome::Done);
        }
        let mut embedder = (self.embedder)()?;
        let model = embedder.model_id().to_string();
        let mut rows = Vec::with_capacity(chunks.len());
        for (b, batch) in chunks.chunks(BATCH).enumerate() {
            // Recording preempts: the work restarts later, nothing is stored.
            if ctx.preempted() {
                return Ok(Outcome::Yield(ctx.job.payload.clone()));
            }
            ctx.progress(None, (b * BATCH) as f32 / chunks.len() as f32);
            let texts: Vec<String> = batch.iter().map(|c| c.text.clone()).collect();
            let vecs = embedder
                .embed(&texts, Kind::Document)
                .map_err(|e| e.to_string())?;
            if vecs.len() != batch.len() {
                return Err("the embedder returned the wrong number of vectors".into());
            }
            for (c, vec) in batch.iter().zip(vecs) {
                rows.push(EmbeddingChunk {
                    chunk: rows.len() as u32,
                    t0_ms: c.t0_ms,
                    t1_ms: c.t1_ms,
                    vec,
                });
            }
        }
        drop(embedder); // frees the model
        ctx.store
            .put_embeddings(meeting, &model, version, rows)
            .map_err(store_err)?;
        Ok(Outcome::Done)
    }
}

/// Queues an `embed_index` job for the meeting unless one is already waiting.
pub fn queue_one(store: &Store, meeting: &str) -> Result<(), String> {
    if !enabled(store) {
        return Ok(());
    }
    if store
        .active_job(meeting, EMBED_INDEX_JOB)
        .map_err(store_err)?
        .is_none()
    {
        store
            .enqueue_job(
                Some(meeting),
                EMBED_INDEX_JOB,
                JOB_PAYLOAD_VERSION,
                &serde_json::json!({}),
            )
            .map_err(store_err)?;
    }
    Ok(())
}

/// Queues the indexer for every meeting without current embeddings of
/// [`MODEL_ID`] (the app calls it at startup and when the model arrives).
/// Returns how many meetings were looked at; the runner needs a `notify`.
pub fn queue_missing(store: &Store) -> Result<usize, String> {
    queue_missing_for(store, MODEL_ID)
}

/// Store setting: this machine indexes meetings for semantic search (the
/// app sets it per hardware tier; Light machines search by keywords only).
pub const ENABLED_SETTING: &str = "embeddings.enabled";

/// Whether embeddings are on (see [`ENABLED_SETTING`]; off unless set).
pub fn enabled(store: &Store) -> bool {
    store
        .get_setting(ENABLED_SETTING)
        .ok()
        .flatten()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Turns indexing on or off; off also cancels the queued index jobs.
pub fn set_enabled(store: &Store, on: bool) -> Result<(), String> {
    store
        .set_setting(ENABLED_SETTING, &serde_json::json!(on))
        .map_err(store_err)?;
    if !on {
        for j in store.active_jobs().map_err(store_err)? {
            if j.kind == EMBED_INDEX_JOB && j.state == ghi_store::jobs::JobState::Queued {
                store.cancel_job(j.id).map_err(store_err)?;
            }
        }
    }
    Ok(())
}

/// [`queue_missing`] for another model id (tests).
pub fn queue_missing_for(store: &Store, model: &str) -> Result<usize, String> {
    let gids = store
        .meetings_needing_embeddings(model, usize::MAX >> 1)
        .map_err(store_err)?;
    for g in &gids {
        queue_one(store, g)?;
    }
    Ok(gids.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::bus;
    use crate::jobs::{JobRunner, always_ready};
    use crate::session::RecordingHooks;
    use ghi_llm::embed::{FakeEmbedder, cosine};
    use ghi_store::jobs::JobState;
    use ghi_store::keys::{MemoryKeyStore, Protection};
    use ghi_store::store::{NewMeeting, NewSegment, NewSpeaker};

    fn line(t0: i64, t1: i64, text: &str) -> (i64, i64, String) {
        (t0, t1, text.to_string())
    }

    #[test]
    fn chunks_close_after_a_minute_on_a_segment_boundary() {
        let lines: Vec<_> = (0..30)
            .map(|i| line(i * 10_000, i * 10_000 + 9_000, &format!("câu {i}")))
            .collect();
        let chunks = chunk_lines(&lines);
        // 300 s: each chunk takes lines until it spans 60 s (6 lines of 10 s
        // steps end at 59 s, the 7th reaches 69 s).
        assert_eq!(chunks.len(), 5);
        assert_eq!((chunks[0].t0_ms, chunks[0].t1_ms), (0, 69_000));
        assert!(chunks[0].text.starts_with("câu 0\ncâu 1"));
        assert!(chunks[0].text.ends_with("câu 6"));
        assert_eq!(chunks[1].t0_ms, 70_000);
        // Every line is in exactly one chunk, in order.
        let joined: Vec<&str> = chunks.iter().flat_map(|c| c.text.lines()).collect();
        assert_eq!(joined.len(), 30);
    }

    #[test]
    fn chunks_respect_the_character_cap_and_split_long_lines() {
        let long = "a".repeat(1_000);
        let lines = vec![
            line(0, 1_000, &long),
            line(1_000, 2_000, &long),
            line(2_000, 3_000, "ngắn"),
        ];
        let chunks = chunk_lines(&lines);
        assert_eq!(chunks.len(), 2);
        assert!(chunks.iter().all(|c| c.text.chars().count() <= MAX_CHARS));
        assert_eq!(chunks[1].text, format!("{long}\nngắn"));

        let huge = "b".repeat(MAX_CHARS * 2 + 10);
        let chunks = chunk_lines(&[line(5, 9, &huge), line(9, 12, "sau")]);
        assert_eq!(chunks.len(), 4);
        assert!(chunks.iter().all(|c| c.text.chars().count() <= MAX_CHARS));
        assert_eq!((chunks[0].t0_ms, chunks[0].t1_ms), (5, 9));
        assert_eq!(chunks[3].text, "sau");
        assert!(chunk_lines(&[line(0, 1, "  ")]).is_empty());
    }

    fn setup() -> (tempfile::TempDir, Arc<Store>, String) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(
            Store::open(
                tmp.path(),
                Arc::new(MemoryKeyStore::default()),
                Protection::default(),
            )
            .unwrap(),
        );
        set_enabled(&store, true).unwrap();
        let m = store
            .create_meeting(NewMeeting {
                title: "Họp".into(),
                started_at: 1_700_000_000_000,
                ..Default::default()
            })
            .unwrap()
            .gid;
        let an = store
            .add_speaker(
                &m,
                NewSpeaker {
                    label_idx: 0,
                    display_name: Some("An".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let script = [
            (0, "Chúng ta cần chốt ngân sách marketing quý bốn."),
            (20_000, "Ngân sách quý bốn là ba tỷ đồng."),
            (70_000, "Tuần sau mua laptop mới cho nhóm kỹ thuật."),
            (140_000, "Hẹn gặp lại thứ Sáu."),
        ];
        let segs = script
            .iter()
            .map(|&(t0, text)| NewSegment {
                t0_ms: t0,
                t1_ms: t0 + 9_000,
                text: text.into(),
                speaker_gid: Some(an.clone()),
                ..Default::default()
            })
            .collect();
        store.add_segments(&m, segs).unwrap();
        store.set_meeting_status(&m, "ready").unwrap();
        (tmp, store, m)
    }

    fn runner(store: &Arc<Store>, handler: IndexJob) -> Arc<JobRunner> {
        let (tx, _rx) = bus();
        JobRunner::new(store.clone(), tx, vec![Arc::new(handler)])
    }

    fn fake_job() -> IndexJob {
        IndexJob {
            embedder: Arc::new(|| Ok(Box::new(FakeEmbedder::new()) as Box<dyn Embedder + Send>)),
            ready: always_ready(),
        }
    }

    #[test]
    fn indexes_a_scripted_transcript_and_search_finds_the_right_chunk() {
        let (_tmp, store, m) = setup();
        assert_eq!(queue_missing_for(&store, FakeEmbedder::MODEL).unwrap(), 1);
        // Idempotent while the job is waiting.
        assert_eq!(queue_missing_for(&store, FakeEmbedder::MODEL).unwrap(), 1);
        assert_eq!(store.jobs_for_meeting(&m).unwrap().len(), 1);

        let runner = runner(&store, fake_job());
        assert_eq!(runner.run_pending(), 1);
        let job = &store.jobs_for_meeting(&m).unwrap()[0];
        assert_eq!(job.state, JobState::Done);

        let rows = store.embeddings(&m, FakeEmbedder::MODEL).unwrap();
        // The first chunk closes with the line that takes it past 60 s (70-79 s);
        // the line at 140 s starts the second.
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!((rows[0].t0_ms, rows[0].t1_ms), (0, 79_000));
        assert_eq!((rows[1].t0_ms, rows[1].t1_ms), (140_000, 149_000));
        assert_eq!(rows.iter().map(|r| r.chunk).collect::<Vec<_>>(), [0, 1]);
        assert!(
            store
                .meetings_needing_embeddings(FakeEmbedder::MODEL, 10)
                .unwrap()
                .is_empty()
        );
        assert_eq!(queue_missing_for(&store, FakeEmbedder::MODEL).unwrap(), 0);

        // The speaker's name is part of the chunk text, so a query on words in
        // one chunk lands closest to it.
        let mut e = FakeEmbedder::new();
        let q = e
            .embed(&["ngân sách quý bốn".into()], Kind::Query)
            .unwrap()
            .remove(0);
        let best = rows
            .iter()
            .max_by(|a, b| cosine(&q, &a.vec).total_cmp(&cosine(&q, &b.vec)))
            .unwrap();
        assert_eq!(best.chunk, 0);
    }

    #[test]
    fn a_new_transcript_version_is_reindexed_and_failures_leave_nothing() {
        let (_tmp, store, m) = setup();
        let r = runner(&store, fake_job());
        queue_one(&store, &m).unwrap();
        r.run_pending();
        store
            .replace_transcript(
                &m,
                vec![NewSegment {
                    t0_ms: 0,
                    t1_ms: 5_000,
                    text: "Bản chính thức".into(),
                    ..Default::default()
                }],
            )
            .unwrap();
        assert_eq!(queue_missing_for(&store, FakeEmbedder::MODEL).unwrap(), 1);
        r.run_pending();
        let rows = store.embeddings(&m, FakeEmbedder::MODEL).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].t1_ms, 5_000);

        // A failing embedder: the job fails and the stored rows are unchanged.
        store
            .replace_transcript(
                &m,
                vec![NewSegment {
                    t0_ms: 0,
                    t1_ms: 5_000,
                    text: "Bản ba".into(),
                    ..Default::default()
                }],
            )
            .unwrap();
        let failing = IndexJob {
            embedder: Arc::new(|| {
                Ok(Box::new(FakeEmbedder {
                    fail: true,
                    ..Default::default()
                }) as Box<dyn Embedder + Send>)
            }),
            ready: always_ready(),
        };
        let r2 = runner(&store, failing);
        queue_one(&store, &m).unwrap();
        let (job, out) = r2.run_one().unwrap();
        assert!(out.is_err());
        assert_eq!(store.job(job.id).unwrap().state, JobState::Failed);
        assert!(
            store
                .embeddings(&m, FakeEmbedder::MODEL)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn waits_for_the_model_and_yields_to_a_recording() {
        let (_tmp, store, m) = setup();
        let not_ready = IndexJob {
            ready: Arc::new(|| false),
            ..fake_job()
        };
        let r = runner(&store, not_ready);
        queue_one(&store, &m).unwrap();
        assert!(r.run_one().is_none(), "model missing: the job waits");
        let j = &store.jobs_for_meeting(&m).unwrap()[0];
        assert_eq!((j.state, j.attempts), (JobState::Queued, 0));

        let r = runner(&store, fake_job());
        r.recording_started();
        assert!(r.run_one().is_none(), "recording: nothing runs");
        r.recording_stopped();
        assert_eq!(r.run_pending(), 1);
        assert_eq!(store.embeddings(&m, FakeEmbedder::MODEL).unwrap().len(), 2);
    }

    #[test]
    fn meetings_without_a_transcript_are_not_queued() {
        let (_tmp, store, _m) = setup();
        let empty = store.create_meeting(NewMeeting::default()).unwrap().gid;
        store.set_meeting_status(&empty, "ready").unwrap();
        assert_eq!(queue_missing_for(&store, "x").unwrap(), 1);
    }

    #[test]
    fn indexing_is_off_unless_the_machine_turns_it_on() {
        let (_tmp, store, m) = setup();
        set_enabled(&store, false).unwrap();
        queue_one(&store, &m).unwrap();
        assert!(store.active_job(&m, EMBED_INDEX_JOB).unwrap().is_none());
        set_enabled(&store, true).unwrap();
        queue_one(&store, &m).unwrap();
        assert!(store.active_job(&m, EMBED_INDEX_JOB).unwrap().is_some());
        // Turning it off cancels what was queued.
        set_enabled(&store, false).unwrap();
        assert!(store.active_job(&m, EMBED_INDEX_JOB).unwrap().is_none());
    }
}
