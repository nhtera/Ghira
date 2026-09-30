// SPDX-License-Identifier: Apache-2.0
// Webview performance check (phase 7 step 5): 1,500 synthetic transcript lines
// in the virtualized list, scrolled programmatically top → bottom → top while
// requestAnimationFrame counts frames. Pass: ≥50 fps.
import { useMemo, useRef, useState } from "react";
import type { Line } from "./bindings";
import { TranscriptList } from "./transcript-list";

const LINES = 1500;
const SCROLL_MS = 8000;
const WORDS =
  "chúng ta chốt lịch beta vào thứ sáu the release notes need one more pass ngân sách quý ba tăng mười phần trăm action item owner follow up".split(
    " ",
  );

function synthetic(): Line[] {
  let seed = 7;
  const rand = () => (seed = (seed * 16807) % 2147483647) / 2147483647;
  return Array.from({ length: LINES }, (_, i) => {
    const n = 6 + Math.floor(rand() * 30);
    const text = Array.from({ length: n }, () => WORDS[Math.floor(rand() * WORDS.length)]).join(" ");
    return { start: i * 2.4, end: i * 2.4 + 2, speaker: 1 + (i % 4), text };
  });
}

type Result = { fps: number; low1: number; frames: number };

export function ScrollTest({ onClose }: { onClose: () => void }) {
  const lines = useMemo(() => synthetic(), []);
  const box = useRef<HTMLDivElement>(null);
  const [result, setResult] = useState<Result | null>(null);
  const [running, setRunning] = useState(false);

  function run() {
    const el = box.current?.querySelector<HTMLElement>(".transcript");
    if (!el) return;
    setRunning(true);
    setResult(null);
    el.scrollTop = 0;
    const deltas: number[] = [];
    const t0 = performance.now();
    let last = t0;
    const frame = (now: number) => {
      deltas.push(now - last);
      last = now;
      const p = (now - t0) / SCROLL_MS;
      const max = el.scrollHeight - el.clientHeight;
      // Down in the first half, back up in the second.
      el.scrollTop = max * (p < 0.5 ? p * 2 : 2 - p * 2);
      if (p < 1) {
        requestAnimationFrame(frame);
        return;
      }
      const sorted = deltas.slice(1).sort((a, b) => b - a);
      const worst = sorted.slice(0, Math.max(1, Math.floor(sorted.length / 100)));
      const avg = (now - t0) / deltas.length;
      setResult({
        fps: 1000 / avg,
        low1: 1000 / (worst.reduce((a, b) => a + b, 0) / worst.length),
        frames: deltas.length,
      });
      setRunning(false);
    };
    requestAnimationFrame(frame);
  }

  return (
    <main className="screen">
      <header className="bar">
        <h1>Scroll test</h1>
        <button className="ghost" onClick={onClose}>
          Back
        </button>
      </header>
      <div className="actions">
        <button className="record" disabled={running} onClick={run}>
          {running ? "Scrolling…" : `Scroll ${LINES.toLocaleString()} lines`}
        </button>
        {result && (
          <p className={result.fps >= 50 ? "pass" : "fail"}>
            {result.fps.toFixed(1)} fps avg · 1% low {result.low1.toFixed(1)} fps · {result.frames} frames
          </p>
        )}
      </div>
      <div ref={box} className="test-box">
        <TranscriptList lines={lines} partial="" />
      </div>
    </main>
  );
}
