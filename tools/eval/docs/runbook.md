# Runbook: running the Ghira evaluation on your machines

You record meetings, label them, and run the test tool on your own computers. Nothing you record leaves your machines. At the end you send the Ghira team **one small report of aggregate numbers**.

Target machines: **macOS on Apple silicon (for example an M1 with 16 GB)** and **Windows 10/11 x64**.

Plan: about 1 hour for steps 1 to 4, then recording and labelling (roughly 10 hours of audio and 50 hours of labelling), then a run of a few hours, then step 8.

Words in `CAPITALS` in commands (`DATASET`, `RUN`) are for you to replace with your own path. All commands are run in the `tools/eval` folder of the kit.

## 1. Before you start: written commitments

The Ghira team asks [Customer company name] to confirm the following **in writing** (an email is enough) before recording:

- [ ] **A named labeller** (and, if possible, a second person for the 10% double check): [name(s)].
- [ ] **Hours per week** the labeller can spend: [hours]. (About 50 hours are needed in total.)
- [ ] **Report turnaround:** you send the report **within 3 business days** after each agreed run.
- [ ] **Consent clause for listening:** participants have the optional box in the consent form that lets **one named member of the Ghira team** listen to short failure excerpts on-site or over a screen share, with no copies. Only excerpts of people who ticked it may be played.
- [ ] **A data contact** at [Customer company name] who keeps the signed consent forms and handles withdrawal requests: [name, email].
- [ ] **Signed consent forms** from every participant (`consent-form-vi.md` / `consent-form-en.md`). Have your legal adviser look at the forms first; they are drafts.
- [ ] Who may open the data: [names or roles].

## 2. Create an encrypted volume first

All recordings, labels and transcripts must live on encrypted storage. Do this **before** the first recording. Choose a size of 100 GB (it only takes the space it uses). Use a strong passphrase, keep it in your company's password manager, and do not store it in the same place as the data.

### macOS

Terminal:

```
hdiutil create -size 100g -type SPARSEBUNDLE -fs APFS -encryption AES-256 -volname GhiraEval ~/GhiraEval.sparsebundle
```

It asks for a passphrase. Do **not** tick "Remember password in my keychain" when it asks. Open the volume (double-click the file, or `hdiutil attach ~/GhiraEval.sparsebundle`). It appears as `/Volumes/GhiraEval`. Eject it when you stop working. The data is only protected while it is ejected.

The same thing in Disk Utility: File > New Image > Blank Image. Format **APFS**, Encryption **256-bit AES**, Image Format **sparse bundle**.

Also turn on FileVault (System Settings > Privacy & Security), and keep the volume out of cloud sync and Time Machine (System Settings > General > Time Machine > Options > exclude the `.sparsebundle`).

### Windows

**Windows 10/11 Pro, Enterprise or Education** include BitLocker. Two ways:

1. **A whole drive** (an internal or USB drive): right-click the drive > Turn on BitLocker.
2. **A VHDX file** (a virtual drive in one file, works on any folder):
   1. Start > Disk Management. Action > Create VHD. Location `C:\GhiraEval.vhdx`, size 100 GB, format **VHDX**, **Dynamically expanding**.
   2. Right-click the new disk > Initialize Disk (GPT). Then right-click the unallocated space > New Simple Volume, format NTFS, name `GhiraEval`, give it a drive letter, for example `E:`.
   3. In File Explorer, right-click `E:` > Turn on BitLocker. Choose a password. Save the recovery key somewhere that is **not** on this computer or this volume.
   4. When you stop working: right-click `E:` > Eject (or Disk Management > Detach VHD). To work again, double-click the `.vhdx` and unlock it.

**Windows Home does not include BitLocker management.** Some Home laptops have "Device encryption" (Settings > Privacy & security > Device encryption), which encrypts the whole system drive. That is acceptable only if it is on and your IT contact confirms it. Otherwise, either upgrade that computer to Pro, use another computer that has Pro, or use a free encryption program such as VeraCrypt (open source; ask your IT contact to approve it). Do not go on without one of these.

Create the dataset folder inside the volume, for example `/Volumes/GhiraEval/customer-2026q4` or `E:\customer-2026q4`. In this runbook that is `DATASET`.

## 3. Install the kit

You need an internet connection for this step and for the dry run. Recording and labelling need none apart from those model downloads, and the kit does not send your data anywhere. The NeMo and Whisper reference systems (`nemo-ref`, `whisper-ref`) **download their models from Hugging Face the first time they run**; the kit switches off Hugging Face telemetry. After the first download, set `HF_HUB_OFFLINE=1` (macOS: `export HF_HUB_OFFLINE=1`; PowerShell: `$env:HF_HUB_OFFLINE='1'`) so that later runs make no network calls at all.

macOS (Terminal):

```
cd tools/eval
./scripts/setup.sh
```

Windows (PowerShell):

```
cd tools\eval
.\scripts\setup.ps1
```

If Windows says running scripts is disabled, run instead: `powershell -ExecutionPolicy Bypass -File .\scripts\setup.ps1`.

What the script does: it looks for **uv** (the tool that installs the Python parts). If uv is missing, it shows the official installer command and asks "Run it now? [y/N]". It never installs anything without your answer. Then it installs the kit and runs a small check. It ends with `OK: the eval kit is installed`.

Optional: add `--with-nemo` (`-WithNemo` on Windows) to install NeMo, which is used to make the first drafts for labelling. It is large (several GB). Do it once, on the machine used for labelling.

The system under test is the `ghi` command-line program that the Ghira team gives you. Put it in a folder you can find, for example next to the kit. If it is not on your `PATH`, give its location in `--system`: `--system ghi:./ghi` on macOS, `--system ghi:.\ghi.exe` on Windows. On macOS, if it is blocked as "from an unidentified developer", run `xattr -d com.apple.quarantine ./ghi` once. On Windows, if SmartScreen blocks `ghi.exe`, choose More info > Run anyway, if the team told you to expect this. Test it with `ghi version --json`. The team's hand-over note says which `--system` to use in the commands below. Examples:

| `--system` value | What runs |
|---|---|
| `ghi` or `ghi:/path/to/ghi` | The Ghira engine (from the team's hand-over note). |
| `whisper-ref:small` | A Whisper model, text only. Needs `uv sync --locked --extra whisper` once. |
| `nemo-ref` | NeMo models, text and speakers. Needs the `--with-nemo` install. |

## 4. Dry run on public data (about 30 minutes plus downloads)

This checks that everything works before your real recordings exist. The data is public, so nothing here is private.

Download two small public sets (about 80 MB):

```
uv run python scripts/fetch_public_sets.py --sets fleurs-vi,ami-sdm
```

It creates `data/fleurs-vi` (25 short Vietnamese read sentences) and `data/ami-sdm` (two English meetings recorded with a distant microphone). It can be stopped and restarted; finished files are skipped. If you only want to see what it would download, add `--dry-run`.

Run the test on each set (use the `--system` value from the hand-over note):

```
uv run ghi-eval validate --dataset data/fleurs-vi
uv run ghi-eval run --dataset data/fleurs-vi --system ghi --tasks asr

uv run ghi-eval validate --dataset data/ami-sdm
uv run ghi-eval run --dataset data/ami-sdm --system ghi --tasks diar
```

Each `run` ends with two lines starting `report:`. Those are the report files, in the run folder under the dataset. The `validate` output will warn that the public sets are below the 10-hour target. Ignore that in the dry run.

A dry run is **done** when both runs printed `report:` lines and no `error:`. You do not need to judge the numbers. If it fails, see "Troubleshooting", then contact the Ghira team.

Attribution for the public data (also written next to each set as `LICENSE-NOTICE.txt`):

| Set | Licence | Source |
|---|---|---|
| FLEURS-vi | CC BY 4.0 | FLEURS (Conneau et al., 2022), Google, https://huggingface.co/datasets/google/fleurs |
| AMI (far-field `Array1-01`) | CC BY 4.0; reference RTTMs Apache-2.0 | AMI Meeting Corpus (Carletta et al., 2005), http://groups.inf.ed.ac.uk/ami. Reference RTTMs: BUTSpeechFIT/AMI-diarization-setup, "only_words" |
| VoxConverse (team/CI only) | CC BY 4.0; video copyright stays with the owners | Chung et al., Interspeech 2020, https://github.com/joonson/voxconverse |
| ViMedCSS (optional) | CC BY 4.0 (as declared by the authors); audio is cut from public YouTube videos, do not redistribute | https://huggingface.co/datasets/tensorxt/ViMedCSS (LREC 2026) |

## 5. Record, then label

- Recording: `recording-guide.md`.
- Labelling and reference notes: `labelling-guide.md`.

Put everything in `DATASET` on the encrypted volume. When you have labels for a few files, run `uv run ghi-eval validate --dataset DATASET` regularly. It lists the errors and what is missing.

## 6. The real run

When labelling is complete (or complete enough, as agreed with the team):

```
uv run ghi-eval validate --dataset DATASET
```

Fix every `error:`. Then run the system, once per pass the team asked for. Typical:

```
uv run ghi-eval run --dataset DATASET --system ghi --pass final
uv run ghi-eval run --dataset DATASET --system ghi --pass live --realtime
```

`--realtime` feeds the audio at normal speed to measure how late captions appear, so that run takes as long as the audio itself (10 hours). Start it in the evening. The first run is also the one that downloads any models.

For the note-taking test, after a run that includes notes:

```
uv run ghi-eval judge --run RUN
```

Fill in the sheet as described in the labelling guide, then:

```
uv run ghi-eval report --run RUN
```

If the volume now has a different mount point or drive letter than when the run was made, add `--dataset DATASET` with the new path (`judge` and `report` both accept it).

`RUN` is the run folder printed after `run:`, for example `DATASET/runs/ghi-20261020-101500`.

**Before you send anything, read the gates table** in the `.md` report. A gate can say `incomplete`: some files had no valid output from the system (a crash, an unsupported task, a bad file), so the number would not describe the whole set. Do not send an `incomplete` report. Look at the `error:` lines that `run` printed, fix the cause (a broken audio file, a wrong `--system`, a full disk), and run again. Send the report only when the gates show `pass`, `best_effort` or `fail`, not `incomplete`.

## 7. What you send back

**Send only the two report files** from the run folder, for example:

```
RUN/report-ghi-final-20261020.json
RUN/report-ghi-final-20261020.md
```

They contain only averaged numbers per group (language, setting, number of speakers), pass/fail against the targets, and counts. No audio, no text, no names, no file names.

The tool checks this itself. Before writing a report it searches it for names from your `names:` list and for any three words in a row from your transcripts and notes, and refuses to write the report if it finds one. Before you send, run the check again on **both** files (the `.md` is checked too):

```
uv run ghi-eval lint-report --report RUN/report-ghi-final-20261020.json --dataset DATASET
uv run ghi-eval lint-report --report RUN/report-ghi-final-20261020.md --dataset DATASET
```

Both must print `privacy lint: ok`. If it does not, **do not send the report**. Read the "privacy lint" item under Troubleshooting.

Send them by [channel agreed with the team, for example email to [Ghira contact]], within **3 business days**.

**Never send:** audio, `labels/`, `refs/`, `notes/`, `manifest.yaml`, anything in `runs/` other than the two report files (`hyp/`, `scores.json`, `judgements.csv`, `draft-eaf/` contain text or file ids), consent forms, the code-to-name list, screenshots or copy-and-paste of the terminal that show your meeting content.

If the team asks to listen to failures, that happens only on-site or in a screen share you run, only for people who ticked the optional consent box, with no copies.

## 8. Deletion at the end

At the end of the retention period stated in the consent form ([date]), or earlier if the test is over:

1. Eject the encrypted volume, then **delete the volume file** (`GhiraEval.sparsebundle`, or the `.vhdx`). Empty the Trash / Recycle Bin.
2. **Delete the passphrase and the BitLocker recovery key.** Without them, any leftover copy of the encrypted file is unreadable.
3. Delete originals and leftovers outside the volume: the OBS `.mkv` recordings, Audacity projects, files in Downloads, and any backup copy (Time Machine, cloud drives).
4. Write to the data contact that you did all of this. They keep the confirmation with the consent forms.
5. The report files you sent are aggregate numbers and can stay.

**A participant withdraws** (at any time): they tell the data contact. Within the number of days written in the consent form, do all of the following, and confirm to them in writing:

1. Delete, for every recording where their voice appears: the audio (`audio/<id>.wav` and the `.mic.wav` / `.system.wav` tracks), `labels/<id>.rttm`, `refs/<id>.txt`, `notes/<id>.yaml`, and the entry in `manifest.yaml`.
2. Delete **every `runs/*` folder that includes that recording, as a whole folder**. A run folder holds `hyp/` (text of the meeting), `judgements.csv`, `draft-eaf/`, `scores.json` and the reports, all of which contain their words or file ids. Do not try to pick files out of it.
3. Delete `trials.tsv` and `speaker_scores.tsv` (they link people across meetings), wherever they are (the dataset folder or a run folder).
4. In `manifest.yaml` remove their entries from `persons:` and their name from `names:`, and delete their code from the code-to-name list. Check `notes/*.yaml` of other meetings for tasks owned by them and remove those owners.
5. Re-run whatever you still need (`ghi-eval run ...`, `judge`, `report`), and tell the Ghira team that the set changed. Reports already sent contain only aggregate numbers and are not affected.

## Troubleshooting

| Problem | What to do |
|---|---|
| `uv: command not found` after installing | Close the terminal and open a new one. On macOS the installer puts it in `~/.local/bin`. |
| PowerShell: "running scripts is disabled" | Use `powershell -ExecutionPolicy Bypass -File .\scripts\setup.ps1`. |
| Download fails with `CERTIFICATE_VERIFY_FAILED` | Your Python cannot find the certificate list. Company proxy: ask IT for the company root certificate file and set `SSL_CERT_FILE` to its path. python.org Python on macOS: run "Install Certificates.command" in /Applications/Python 3.x. Then run the command again. |
| The download stops or is slow | Run the same command again. Finished files are kept. The VoxConverse server is slow; that set is for the team, not the dry run. |
| `validate` says `error: ... not a readable PCM WAV` | The file is not a 16-bit PCM WAV (for example an M4A renamed to WAV). Export it again as WAV PCM 16-bit. |
| `validate` warns "manifest says 4 speakers, RTTM has 5" | Either the manifest count or a label is wrong. Listen and fix one of them. |
| `ghi` exits with code 3 (`engine_unavailable` or `not_implemented`) | `run` does not fail: it skips that task, prints a note, and the gates that need it show `n/a` or `incomplete`. The engine has no support for that task yet. Use the `--system` from the hand-over note, or `--tasks` the engine supports. |
| `run` says "NeMo is not installed" | Run `./scripts/setup.sh --with-nemo` (`.\scripts\setup.ps1 -WithNemo`). |
| Out of memory during a NeMo or Whisper run | Close other programs. Use a smaller Whisper model (`whisper-ref:small`). NeMo drafts may be slow on CPU; run them overnight. |
| The draft finds fewer speakers than the meeting had | The default draft model (Nemotron 3 Diarization) handles up to 8 speakers, and quiet speakers can be missed. Add the missing speakers by hand when correcting. |
| Privacy lint failed | The message shows what matched (on your screen only; nothing is written). The usual causes: a name in `names:` that is also a common word, the dataset `name:` in `manifest.yaml`, or a very short, generic reference line. Change the `name:` field, or remove the offending entry from `names:` if it is a common word, and run `uv run ghi-eval report --run RUN` again. If it still fails, do not send anything and contact the team without quoting the text. |
| `judge` says `judgements.csv exists` | It protects your work. Only use `--force` if you want to start the sheet again from scratch. |
| A recording is too big for the volume | Extend the volume (create a larger one and copy), or use 16 kHz mono WAV: about 115 MB per hour. |

## Appendix: team sanity check (not for the customer)

The Ghira team uses this to check that the harness reproduces a published diarization result. It needs the VoxConverse audio (a slow download from Oxford) and a Hugging Face account, because the published outputs are in a **gated repository**.

Before you run it:

1. Log in to Hugging Face and **accept the access conditions** at https://huggingface.co/pyannote/speaker-diarization.
2. Create a read token at https://huggingface.co/settings/tokens and set it in your shell: `export HF_TOKEN=...` (PowerShell: `$env:HF_TOKEN='...'`). The script sends it only to huggingface.co, never to other hosts, and never prints or stores it. Without the token the script stops before downloading anything.

```
uv run python scripts/fetch_public_sets.py --sets voxconverse --with-published-hyp
uv run ghi-eval run --dataset data/voxconverse --system files:data/voxconverse/published-hyp --tasks diar --collar 0
```

The published system is **pyannote.audio 2.1** (`pyannote/speaker-diarization`, MIT licence), folder `reproducible_research/2.1/`. Its authors report **DER 12.76%** over the whole VoxConverse test set, computed with **collar 0 s, overlapped speech scored, no oracle VAD, against VoxConverse v0.0.2 references**. `--with-published-hyp` therefore switches the references to v0.0.2. For the chosen subset, `data/voxconverse/published.json` holds the DER that the authors' own per-file table gives; the harness result must be **within 1 point** of `subset_der_percent`. The kit's normal DER uses a 0.25 s collar, so this check needs `--collar 0`. Do not compare it to the gates.

The gated files are not hash-pinned yet (they could only be fetched with a token). The script prints their SHA-256; compare with the values in the comment above `PUB_RTTM` in the script (taken from a public copy of the same folder) and paste them in to pin. If they differ, check the folder is still `2.1` and re-pin.

Checked on 2026-09-29 with the 7-file small subset (about 7 minutes of audio), using a public copy of the same outputs: published 7.66%, harness with `--collar 0` 7.69%.

Sources: https://huggingface.co/pyannote/speaker-diarization, https://github.com/joonson/voxconverse.
