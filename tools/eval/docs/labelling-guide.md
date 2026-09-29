# Labelling guide

Labelling means writing down, for each recording, **who spoke when** and **what they said**. The result is the "answer key" the software is scored against. Careful labels matter more than fast labels.

Plan **4 to 6 hours of work per hour of audio**, so about 50 hours for a 10-hour set. Work in sessions of at most 2 hours; quality drops after that. Use good closed-back headphones.

The labeller should be a native Vietnamese speaker who is comfortable with English words used at work. One or two people is better than many; they apply the rules more consistently.

Everything stays on the encrypted volume. Never copy audio or labels to a personal drive, email or a chat app.

## 1. Workflow

```
audio  ->  draft (software)  ->  correct by hand  ->  export  ->  labels/<id>.rttm + refs/<id>.txt
```

All commands are run in the `tools/eval` folder. `DATASET` below is the path to your dataset folder on the encrypted volume.

### Step 1. Make a draft

The software makes a first guess at the text and the speakers. Correcting a guess is much faster than typing from nothing.

```
uv run ghi-eval run --dataset DATASET --system nemo-ref
```

This needs the NeMo models (installed with `setup.sh --with-nemo` / `setup.ps1 -WithNemo`). **The first run downloads the models from Hugging Face**, so it needs the internet (the kit turns off Hugging Face telemetry and sends none of your audio or text). After that, set `HF_HUB_OFFLINE=1` so later runs make no network calls. For a second opinion on the text, you can also run `--system whisper-ref` (needs `uv sync --locked --extra whisper`). The run prints a line `run: DATASET/runs/nemo-ref-<date>-<time>`. Note that folder. You will see scores if labels already exist. For a new set they are empty, which is normal.

### Step 2. Turn the draft into files you can edit

For ELAN (text and speakers together, recommended):

```
uv run ghi-eval convert draft-eaf --run DATASET/runs/nemo-ref-<date>-<time>
```

This writes one `<id>.eaf` file per recording into the `draft-eaf` folder inside the run folder. Each speaker is one tier. Each annotation is one utterance with its text.

For speaker turns only, in Audacity:

```
uv run ghi-eval convert rttm-audacity DATASET/runs/nemo-ref-<date>-<time>/hyp/m001.rttm --out m001.draft-labels.txt
```

In Audacity, File > Import > Labels, and select that file.

### Step 3. Correct by hand

**ELAN** (free, https://archive.mpi.nl/tla/elan). File > Open the `.eaf`. The audio is linked. Then:

- Listen to each utterance. Fix the **text** first, then the **start and end** if they are wrong.
- Fix the **tier** (speaker) if it is the wrong person. Rename tiers to `spk1`, `spk2`, ... (Tier > Change Tier Attributes). Keep the same name for the same person for the whole recording.
- Add a tier for a speaker the software missed (Tier > Add New Tier). Give it the same name style.
- Delete annotations where nobody speaks.
- Within one tier ELAN does not allow overlaps. Two speakers overlapping are simply two tiers, so overlap is fine.
- Save often (File > Save).

**Audacity** (free, https://www.audacityteam.org), for speaker turns only: open the audio, add a label track (Tracks > Add New > Label Track), select a stretch of audio and press Ctrl+B to add a label. The label text is the speaker name (`spk1`, ...). Export with File > Export Other > Export Labels (a `.txt` file). If you do this, the transcript (`refs/<id>.txt`) still has to come from somewhere else, so ELAN is the better choice when you need both.

### Step 4. Export into the dataset

ELAN file, both files at once:

```
uv run ghi-eval convert eaf path/to/m001.eaf --id m001 --dataset DATASET
```

This writes `DATASET/labels/m001.rttm` and `DATASET/refs/m001.txt`.

Audacity label file (speaker turns only):

```
uv run ghi-eval convert audacity path/to/m001-labels.txt --id m001 --dataset DATASET
```

Then check everything with `uv run ghi-eval validate --dataset DATASET`. It warns if the number of speakers in the RTTM differs from `speakers` in the manifest. Fix whichever is wrong.

## 2. Transcript rules (text)

Write **exactly what was said**. Do not correct grammar, do not summarise, do not translate.

| Topic | Rule | Example |
|---|---|---|
| Vietnamese | Full diacritics, correct tone marks. Type with Unicode (NFC). | `mình chốt scope cho beta nhé` |
| English words inside Vietnamese speech | Write them **in English**, as the speaker said them. | `anh gửi cái deadline cho team review` |
| Loanwords that are ordinary Vietnamese now | Write the Vietnamese spelling. | `cà phê`, `ti vi`, `sô cô la` |
| Whole English sentences | Write normal English. | `let's move on to the next item` |
| Acronyms | Upper case, one token, however it is pronounced. | `API`, `KPI`, `CEO` |
| Product and company names | The usual spelling. | `Zoom`, `Slack`, `iPhone` |
| Numbers | **As spoken**, in words. | `hai nghìn không trăm hai mươi sáu`, `mười lăm phần trăm`, `twenty twenty six` |
| Punctuation and capitals | Optional and ignored in scoring. Use them if they help you read. | |
| Repetitions and false starts | Keep the whole words that were said. | `là là là cái này` |
| Cut-off word fragments | Leave out the fragment. If you cannot tell what was said, write `[unk]`. | |
| Filler sounds (ờ, ừm, ưm, à, hmm, uh, um, er) | **Leave out.** | |
| Answers that carry meaning (ừ, dạ, vâng, ok, yeah, right, uh-huh meaning yes) | **Write them.** | `ừ đúng rồi` |
| Unintelligible speech | `[unk]` for each stretch you cannot make out (one `[unk]` per stretch is enough). Use it sparingly, see below. | `mình sẽ [unk] vào thứ sáu` |
| Non-speech sounds | Square-bracket tags, for example `[laugh]`, `[cough]`, `[noise]`, `[music]`. Optional. Anything in `[...]` is removed before scoring. | |
| Names of people | Normal spelling with diacritics. Also add the name to `names:` in the manifest. | `chị Linh` |
| Speech that is not part of the meeting (TV, a person in the hallway) | Skip it unless someone in the meeting replies to it. | |

*Numbers:* the scoring software reads digits out as words on both sides (Vietnamese and English), so "15%" and "mười lăm phần trăm" count as the same. Spoken variants such as "mốt", "tư", "lăm", "ngàn" and "lẻ" are also treated as equal to "một", "bốn", "năm", "nghìn" and "linh". Times ("10:30"), codes and phone numbers are not converted, so write those the same way the speaker would say them everywhere. When in doubt, keep writing numbers in words.

Keep one utterance per line in the exported text. The tool joins the lines, so where you break lines does not change the score. Break at natural pauses.

**Use `[unk]` sparingly, only for speech you really cannot make out even after listening three times.** `[unk]` is removed from the reference before scoring, so whatever the software writes in that place counts as extra words (insertions) against it. Too many `[unk]` therefore make the software look worse than it is. Never guess at words you cannot hear, but try hard first: slow the audio down, use headphones, ask a colleague.

## 3. Speaker-turn rules (who spoke when)

- **Speaker names:** `spk1`, `spk2`, ... in the order they first speak. Same person, same name, for the whole recording. Fill `persons:` in the manifest to link them to person codes.
- **One turn** is a continuous stretch of speech by one person.
- A pause **shorter than 0.3 s** inside one person's speech stays inside the same turn. A pause of **0.3 s or more** ends the turn; a new turn from the same person can follow.
- Put the start where the voice starts and the end where it stops. Do not add extra room before or after. A difference of 0.1 s does not matter.
- **Overlap:** when two people talk at the same time, both get turns, and the turns overlap. Do not cut the main speaker's turn to make room for the other.
- **Short answers** (ừ, dạ, yeah) that are clearly spoken get their own short turn, even if they overlap someone. Ignore sounds shorter than about 0.2 s.
- **Not speech:** laughter, coughs, breathing, typing and the sound of chairs are not turns. Laughing while speaking is part of the turn.
- **Far or quiet speakers** who you can still hear and identify: label them. If you cannot tell who it is, use the name `spk_unk` and, if it is not intelligible, `[unk]`.
- **A speaker heard only through the call audio** (a remote participant) is a speaker like any other.
- Long turns: for the transcript, split a turn into utterances of about 15 seconds at pauses. The speaker tier stays the same.

## 4. Quality control

- **Self-check:** after the first hour of audio, re-listen to a random 5 minutes and compare with the rules. Fix systematic mistakes across all files done so far.
- **Two labellers on 10% of the audio.** Choose about 10% of the files (at least 1 hour in total, from different languages and settings). A second labeller labels the turns **from scratch, without seeing the first labeller's work and without the software draft**, using Audacity label tracks (turns only). Then measure how much they disagree: the **inter-annotator DER**.

  1. Make a small dataset folder for the double-labelled files, for example `DATASET/iaa/`, containing:
     - `manifest.yaml`: copy the main manifest, keep only the double-labelled files, and change the paths to point to the main dataset (for example `audio: ../audio/m001.wav`, `rttm: ../labels/m001.rttm`). The `rttm` here is labeller A's file.
     - `hyp/m001.rttm`: the second labeller's turns, exported with `convert audacity ... --id m001 --rttm DATASET/iaa/hyp/m001.rttm`.
  2. Score labeller B against labeller A. The `files:` system treats the RTTMs in `hyp/` as a system's output:

     ```
     uv run ghi-eval run --dataset DATASET/iaa --system files:DATASET/iaa --tasks diar --run-id iaa
     ```

  3. Read the `der` in the report inside `DATASET/iaa/runs/iaa/`. Ignore the gate lines; they are for the software, not people.

  Rough guide: **5% or lower** means the rules are understood the same way. **Above 10%** means the two people read the rules differently. Sit down together on the largest differences, agree what the rule is, and fix both labels. Note the final number for your records. The Ghira team may ask for the figure, not the files.
- **Text check:** for the same 10%, the second person also reads the first person's text while listening and marks doubtful lines with `[unk]` or a comment. Discuss the differences. There is no automatic text score for this.
- The numbers the software will be judged on are only as good as these labels, so if you change a rule midway, go back and update the earlier files.

## 5. Reference notes for the AI-summary test

For **5 to 10 meetings** (a mix of languages), someone who was in the meeting, or who listens to it carefully, writes what the good notes should contain. This is used to check the note-taking feature later. One file per meeting, `notes/<id>.yaml`, and add `notes_ref: notes/<id>.yaml` in the manifest.

```yaml
action_items:
  - text: Send the beta scope doc
    owner: Linh                # or null when nobody was named
  - text: Đặt lịch họp review với khách hàng
    owner: null
decisions:
  - text: Beta ships without calendar sync
```

Rules:

- **Action item:** something a person agreed, or was clearly asked, to do after the meeting. Write it as one short sentence in the language of the meeting. Include a due date only if it was said.
- **Decision:** something the group settled. A topic that was only discussed is not a decision.
- **Owner:** the person named for the task, with the same spelling as in `names:`. Use `null` if nobody was named. Do not guess.
- List **every clear** item, not only the important ones. Leave out anything you are not sure about.
- Do not copy long passages from the transcript. Write each item in your own words, briefly.

### Judging the software's notes (`judgements.csv`)

After a run that includes notes, the harness creates a sheet where a person checks each item the software produced. Open it in a spreadsheet program and save it back as CSV (see below).

```
uv run ghi-eval judge --run DATASET/runs/<run-id>
```

This writes `judgements.csv` in the run folder. It has one row per item the software wrote, and one row per reference item that no software item matched. The columns `ref_idx`, `ref_text`, `ref_owner` hold a **suggested** match. You check every row, then rebuild the report:

```
uv run ghi-eval report --run DATASET/runs/<run-id>
```

**Excel.** The file is UTF-8 with a byte-order mark, so Vietnamese letters show correctly. Do not double-click a file that is already open in another program. Best is Data > From Text/CSV, File Origin **65001: Unicode (UTF-8)**, and choose the delimiter (comma, semicolon or tab). The harness accepts `,`, `;` or tab. On a computer with Vietnamese regional settings Excel often uses `;`, and that is fine. When you save, use **File > Save As > CSV UTF-8 (Comma delimited) (*.csv)**, not plain "CSV" and not "Excel Workbook": plain CSV loses the diacritics.

**LibreOffice Calc.** File > Open, choose the file; in the import dialog set Character set **Unicode (UTF-8)** and tick the delimiter that matches. To save: File > Save As > file type **Text CSV (.csv)**, tick **Edit filter settings**, then Character set **Unicode (UTF-8)** and a comma as field delimiter.

Cells that start with `=`, `+`, `-` or `@` have a leading apostrophe (`'`) added by the tool, so a spreadsheet does not treat them as formulas. Leave it in place.

Columns to fill in:

| Column | What to do |
|---|---|
| `ref_idx`, `ref_text`, `ref_owner` | Is the suggested match really the same task? If yes, leave it. If not, **clear all three cells**. If it matches a different reference item, type that item's number in `ref_idx` (the number is in the reference-only rows). A software item with an empty `ref_idx` counts as "not in the reference". |
| `owner_ok` | Only for matched rows. `y` if the software named the same person as the reference. `n` if the person is different or missing. |
| `citation_ok` | `y` if the transcript passages the software cites (columns `sys_citations`, the segment numbers, and `cited_text`, their text) really say this. `n` if they do not, or if there is no citation. `na` if the item has no citation by design. |
| `hallucinated` | `y` if the item is **not supported by the meeting at all**, anything invented: a task nobody mentioned, a wrong number or date, a name that was not in the meeting. `n` otherwise, even if the item is unimportant or not in your reference notes. |

How to decide `citation_ok`: read only the cited passages. Would a reasonable person, looking at just those lines, agree with the item? If the item is true but the cited lines do not show it, that is `n`. If the cited lines show it only partly (the task is there but not the owner), that is `n` for the citation and can still be `n` for `hallucinated`.

How to decide `hallucinated`: listen to or read the whole meeting. If you can find the content anywhere, it is not hallucinated. A vague or badly phrased item is `n`. An item that says something the meeting did not say is `y`. When in doubt, listen again before choosing `y`.

The report only shows numbers for the notes test once every software row has `citation_ok` and `hallucinated` filled in.
