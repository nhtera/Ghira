// SPDX-License-Identifier: Apache-2.0
// The mind map: an SVG of the notes tree (pan, zoom, fit, collapse) with a
// `role="tree"` list as its keyboard and screen-reader surface. The list is
// hidden until it has focus, then shows over the map; moving in it moves the
// map's highlight. A leaf click plays its first citation, a modified click
// shows it in the transcript.
import { Avatar, Button, Icon, cn, usePlatform } from "@ghi/ui";
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import type { MeetingSpeaker } from "../../bindings";
import { findSpeaker, speakerDisplay } from "../meeting/speaker-display";
import { BADGE_H, GUTTER_W, LINE_H, PAD_X, PAD_Y, tidyLayout, wrapLines, type Placed } from "./layout";
import type { MapNode } from "./tree";

const MIN_K = 0.1;
const MAX_K = 2.5;
const MARGIN = 24;
const DRAG_PX = 4;
/** The keyboard list's overlay (w-80 + its offset): the map is only free to the right of it while the list has focus. */
const LIST_W = 328;

/** `prefers-reduced-motion: reduce`, live. */
function useReducedMotion() {
  const query = "(prefers-reduced-motion: reduce)";
  const [reduced, setReduced] = useState(() => typeof matchMedia === "function" && matchMedia(query).matches);
  useEffect(() => {
    if (typeof matchMedia !== "function") return;
    const m = matchMedia(query);
    const on = () => setReduced(m.matches);
    m.addEventListener("change", on);
    return () => m.removeEventListener("change", on);
  }, []);
  return reduced;
}

type View = { x: number; y: number; k: number };

export type MindMapProps = {
  root: MapNode;
  speakers: readonly MeetingSpeaker[];
  /** A leaf was activated: `show` is the modified click (the transcript instead of the audio). */
  onActivate: (node: MapNode, show: boolean) => void;
  onCopy: () => void;
  className?: string;
};

const clampK = (k: number) => Math.min(MAX_K, Math.max(MIN_K, k));

/** The visible nodes in reading order, with their parent, for the tree list. */
function visible(root: MapNode, collapsed: ReadonlySet<string>) {
  const out: { node: MapNode; level: number; parent: string | null; pos: number; size: number }[] = [];
  const walk = (n: MapNode, level: number, parent: string | null, pos: number, size: number) => {
    out.push({ node: n, level, parent, pos, size });
    if (!collapsed.has(n.id)) n.children.forEach((c, i) => walk(c, level + 1, n.id, i + 1, n.children.length));
  };
  walk(root, 1, null, 1, 1);
  return out;
}

export function MindMap({ root, speakers, onActivate, onCopy, className }: MindMapProps) {
  const { t } = useTranslation();
  const platform = usePlatform();
  const reduced = useReducedMotion();
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(new Set());
  const [view, setView] = useState<View>({ x: MARGIN, y: MARGIN, k: 1 });
  const [wantAnimate, setAnimate] = useState(false);
  const animate = wantAnimate && !reduced;
  const [spaceDown, setSpaceDown] = useState(false);
  const [cursor, setCursor] = useState<string | null>(null);
  const port = useRef<HTMLDivElement>(null);
  const size = useRef({ w: 0, h: 0 });
  const fitted = useRef(false);
  const moved = useRef(false);
  const items = useMemo(() => visible(root, collapsed), [root, collapsed]);
  const layout = useMemo(() => tidyLayout(root, collapsed), [root, collapsed]);
  const placed = useMemo(() => new Map(layout.nodes.map((n) => [n.id, n])), [layout]);
  const focusId = cursor && items.some((i) => i.node.id === cursor) ? cursor : (items[1]?.node.id ?? root.id);

  const fitView = useCallback(
    (animated = true) => {
      const { w, h } = size.current;
      if (!w || !h) return;
      const k = clampK(Math.min(1.2, (w - 2 * MARGIN) / layout.width, (h - 2 * MARGIN) / layout.height));
      setAnimate(animated);
      setView({ k, x: (w - layout.width * k) / 2, y: (h - layout.height * k) / 2 });
    },
    [layout.width, layout.height],
  );

  // The first time the viewport has a size, fit; later resizes keep the reader's view.
  useEffect(() => {
    const el = port.current;
    if (!el) return;
    const measure = () => {
      size.current = { w: el.clientWidth, h: el.clientHeight };
      if (!fitted.current && size.current.w) {
        fitted.current = true;
        fitView(false);
      }
    };
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [fitView]);

  const zoomAt = useCallback((factor: number, px?: number, py?: number, animated = false) => {
    setAnimate(animated);
    setView((v) => {
      const cx = px ?? size.current.w / 2;
      const cy = py ?? size.current.h / 2;
      const k = clampK(v.k * factor);
      return { k, x: cx - ((cx - v.x) * k) / v.k, y: cy - ((cy - v.y) * k) / v.k };
    });
  }, []);

  // Wheel and pinch (ctrl+wheel) zoom around the pointer; React's wheel listener is passive, so this one is native.
  useEffect(() => {
    const el = port.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      // The keyboard list scrolls on its own.
      if ((e.target as Element).closest("[role=tree]")) return;
      e.preventDefault();
      const r = el.getBoundingClientRect();
      zoomAt(Math.exp(-e.deltaY * (e.ctrlKey ? 0.01 : 0.0015)), e.clientX - r.left, e.clientY - r.top);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [zoomAt]);

  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0 || (e.target as Element).closest("button, [role=tree]")) return;
    moved.current = false;
    const start = { x: e.clientX, y: e.clientY, vx: view.x, vy: view.y };
    const move = (m: PointerEvent) => {
      const dx = m.clientX - start.x;
      const dy = m.clientY - start.y;
      if (!moved.current && Math.hypot(dx, dy) < DRAG_PX) return;
      moved.current = true;
      setAnimate(false);
      setView((v) => ({ ...v, x: start.vx + dx, y: start.vy + dy }));
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };

  const toggle = useCallback((id: string) => {
    setCollapsed((c) => {
      const n = new Set(c);
      if (!n.delete(id)) n.add(id);
      return n;
    });
  }, []);

  const activate = useCallback(
    (n: MapNode, show: boolean) => {
      if (n.kind === "section") toggle(n.id);
      else if (n.kind === "leaf") onActivate(n, show);
    },
    [toggle, onActivate],
  );

  /** Pans so the node is on screen, in the part the keyboard list leaves free. */
  const reveal = (id: string) => {
    const p = placed.get(id);
    const { w, h } = size.current;
    if (!p || !w) return;
    const free = w > 2 * LIST_W ? LIST_W : 0;
    const left = p.x * view.k + view.x;
    const top = p.y * view.k + view.y;
    if (left >= free && top >= 0 && left + p.w * view.k <= w && top + p.h * view.k <= h) return;
    setAnimate(true);
    setView({ ...view, x: free + (w - free) / 2 - (p.x + p.w / 2) * view.k, y: h / 2 - (p.y + p.h / 2) * view.k });
  };

  const focusItem = (id: string) => {
    setCursor(id);
    port.current?.querySelector<HTMLElement>(`[data-tree-id="${CSS.escape(id)}"]`)?.focus();
  };

  const onTreeKey = (e: KeyboardEvent<HTMLUListElement>) => {
    const at = items.findIndex((i) => i.node.id === focusId);
    const cur = items[at];
    if (!cur) return;
    const go = (i: number) => items[i] && focusItem(items[i]!.node.id);
    const n = cur.node;
    switch (e.key) {
      case "ArrowDown":
        go(Math.min(items.length - 1, at + 1));
        break;
      case "ArrowUp":
        go(Math.max(0, at - 1));
        break;
      case "Home":
        go(0);
        break;
      case "End":
        go(items.length - 1);
        break;
      case "ArrowRight":
        if (n.children.length && collapsed.has(n.id)) toggle(n.id);
        else if (n.children.length) go(at + 1);
        break;
      case "ArrowLeft":
        if (n.children.length && !collapsed.has(n.id) && n.kind === "section") toggle(n.id);
        else if (cur.parent) go(items.findIndex((i) => i.node.id === cur.parent));
        break;
      case "Enter":
      case " ":
        activate(n, e.metaKey || e.ctrlKey);
        break;
      default:
        return;
    }
    e.preventDefault();
    e.stopPropagation();
  };

  const onViewportKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.target !== e.currentTarget || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === "+" || e.key === "=") zoomAt(1.25, undefined, undefined, true);
    else if (e.key === "-" || e.key === "_") zoomAt(0.8, undefined, undefined, true);
    else if (e.key === "0") fitView();
    else if (e.key === " ") setSpaceDown(true);
    // The first Tab stop is the map itself: the arrows go into the tree.
    else if (["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key)) focusItem(focusId);
    else return;
    e.preventDefault();
  };

  const ownerOf = useCallback(
    (n: MapNode) => {
      const s = n.ownerGid ? findSpeaker(speakers, n.ownerGid) : undefined;
      return s ? speakerDisplay(s, t) : null;
    },
    [speakers, t],
  );

  // Panning and zooming only change the canvas transform: the nodes, links and list are rebuilt only when the tree, focus or language change.
  const nodeEls = useMemo(
    () =>
      layout.nodes.map((p: Placed) => {
    const n = p.node;
    const lines = wrapLines(n.text);
    const owner = ownerOf(n);
    const lead = n.kind === "section" || n.ownerGid !== undefined;
    const top = n.proposed ? BADGE_H : 0;
    const selected = p.id === focusId;
    const isSection = n.kind === "section";
    const open = isSection && !collapsed.has(n.id);
    return (
      <g
        key={p.id}
        data-node={p.id}
        data-kind={n.kind}
        data-current={selected ? "true" : undefined}
        transform={`translate(${p.x} ${p.y})`}
        className={cn(n.kind !== "root" && "cursor-pointer")}
        onClick={(e) => {
          if (moved.current || n.kind === "root") return;
          activate(n, e.metaKey || e.ctrlKey);
        }}
      >
        <title>{n.full}</title>
        <rect
          width={p.w}
          height={p.h}
          rx={n.kind === "root" ? 12 : 8}
          className={cn(
            n.kind === "root" ? "fill-accent stroke-accent" : isSection ? "fill-surface2 stroke-line2" : "fill-surface stroke-line2",
            selected && "stroke-accent",
            n.kind === "leaf" && "hover:stroke-accent",
          )}
          strokeWidth={selected ? 2.5 : 1.25}
          strokeDasharray={n.proposed ? "4 3" : undefined}
        />
        {n.proposed && (
          <text x={PAD_X} y={13} className="fill-ai text-[10.5px] font-semibold">
            {t("mindmap.proposed")}
          </text>
        )}
        {isSection && (
          <g aria-hidden="true" className="fill-muted" transform={`translate(${PAD_X - 2} ${p.h / 2 - 7})`}>
            <path d={open ? "M2 4 L12 4 L7 11 Z" : "M4 2 L11 7 L4 12 Z"} />
          </g>
        )}
        {owner && n.ownerGid && (
          <foreignObject x={PAD_X - 4} y={top + (p.h - top - 20) / 2} width={22} height={22}>
            <Avatar kind={owner.isMe ? "me" : "person"} name={owner.name} initial={owner.initial} colorSlot={owner.colorSlot} size="sm" />
          </foreignObject>
        )}
        {n.ownerGid === null && (
          <foreignObject x={PAD_X - 4} y={top + (p.h - top - 20) / 2} width={22} height={22}>
            <Avatar kind="unknown" size="sm" />
          </foreignObject>
        )}
        <text
          className={cn("text-[13px]", n.kind === "root" ? "fill-on-accent font-semibold" : "fill-ink", isSection && "font-semibold", n.done && "fill-muted")}
          textDecoration={n.done ? "line-through" : undefined}
        >
          {lines.map((l, i) => (
            <tspan key={i} x={PAD_X + (lead ? GUTTER_W : 0)} y={top + PAD_Y + LINE_H * (i + 0.78)}>
              {l}
            </tspan>
          ))}
        </text>
        {n.starred && (
          <g aria-hidden="true" transform={`translate(${p.w - 8} -2)`}>
            <circle r={9} className="fill-surface stroke-warn" strokeWidth={1.25} />
            <path d="M0 -5.5 L1.6 -1.7 L5.6 -1.4 L2.5 1.2 L3.4 5.2 L0 3 L-3.4 5.2 L-2.5 1.2 L-5.6 -1.4 L-1.6 -1.7 Z" className="fill-warn" />
          </g>
        )}
      </g>
    );
  }),
    [layout, collapsed, focusId, ownerOf, t, activate],
  );

  const linkEls = useMemo(
    () =>
      layout.links.map((l) => {
        const a = placed.get(l.from)!;
        const b = placed.get(l.to)!;
        const x1 = a.x + a.w;
        const y1 = a.y + a.h / 2;
        const x2 = b.x;
        const y2 = b.y + b.h / 2;
        const mx = (x1 + x2) / 2;
        return <path key={`${l.from}>${l.to}`} d={`M${x1} ${y1} C${mx} ${y1} ${mx} ${y2} ${x2} ${y2}`} />;
      }),
    [layout, placed],
  );

  const revealRef = useRef(reveal);
  useEffect(() => {
    revealRef.current = reveal;
  });
  const listEls = useMemo(
    () =>
      items.map(({ node: n, level, pos, size: setSize }) => {
        const owner = ownerOf(n);
        return (
          <li
            key={n.id}
            role="treeitem"
            data-tree-id={n.id}
            tabIndex={n.id === focusId ? 0 : -1}
            aria-level={level}
            aria-posinset={pos}
            aria-setsize={setSize}
            aria-expanded={n.children.length ? !collapsed.has(n.id) : undefined}
            aria-selected={n.id === focusId}
            onFocus={(e) => {
              if (e.target !== e.currentTarget) return;
              setCursor(n.id);
              revealRef.current(n.id);
            }}
            style={{ paddingLeft: (level - 1) * 14 + 8 }}
            className={cn("rounded-ctl py-1 pr-2 text-[13px] outline-none focus-visible:outline-2 focus-visible:outline-accent", n.id === focusId && "bg-accent-soft")}
          >
            <span className={cn(n.kind !== "leaf" && "font-semibold")}>{n.full}</span>
            {owner && <span className="text-muted">{` · ${owner.name}`}</span>}
            {n.done && <span className="text-muted">{` · ${t("notes.markDone")}`}</span>}
            {n.proposed && <span className="text-ai">{` · ${t("mindmap.proposed")}`}</span>}
            {n.starred && <span className="text-warn">{` · ${t("mindmap.marked")}`}</span>}
          </li>
        );
      }),
    [items, collapsed, focusId, ownerOf, t],
  );

  return (
    <div className={cn("flex min-h-0 flex-col gap-2", className)}>
      <div className="flex flex-wrap items-center gap-1.5">
        <Button size="sm" icon="add" aria-label={t("mindmap.zoomIn")} onClick={() => zoomAt(1.25, undefined, undefined, true)} />
        <Button size="sm" icon="remove" aria-label={t("mindmap.zoomOut")} onClick={() => zoomAt(0.8, undefined, undefined, true)} />
        <Button size="sm" icon="open_in_full" onClick={() => fitView()}>
          {t("mindmap.fit")}
        </Button>
        <span className="text-mono min-w-10 text-[12px] text-muted" aria-hidden="true">{`${Math.round(view.k * 100)}%`}</span>
        <Button size="sm" icon="content_copy" onClick={onCopy} className="ml-auto">
          {t("mindmap.copyOutline")}
        </Button>
      </div>
      <div
        ref={port}
        role="group"
        tabIndex={0}
        aria-label={t("mindmap.label")}
        aria-describedby="mindmap-hint"
        data-testid="mind-map"
        onPointerDown={onPointerDown}
        onKeyDown={onViewportKey}
        onKeyUp={(e) => e.key === " " && setSpaceDown(false)}
        onBlur={() => setSpaceDown(false)}
        className={cn(
          "relative h-[min(68vh,720px)] min-h-[360px] touch-none overflow-hidden rounded-panel border border-line2 bg-surface2 select-none focus-visible:outline-2 focus-visible:outline-accent",
          spaceDown ? "cursor-grabbing" : "cursor-grab",
        )}
      >
        <svg aria-hidden="true" width="100%" height="100%" className="absolute inset-0">
          <g
            data-testid="mind-map-canvas"
            style={{ transform: `translate(${view.x}px, ${view.y}px) scale(${view.k})`, transformOrigin: "0 0" }}
            className={cn(animate && "transition-transform duration-200 ease-out motion-reduce:transition-none")}
          >
            <g fill="none" className="stroke-line2" strokeWidth={1.5}>
              {linkEls}
            </g>
            {nodeEls}
          </g>
        </svg>
        <ul
          role="tree"
          aria-label={t("mindmap.tree")}
          onKeyDown={onTreeKey}
          className="m-0 list-none p-0 sr-only focus-within:not-sr-only focus-within:absolute focus-within:top-2 focus-within:left-2 focus-within:z-10 focus-within:max-h-[calc(100%-1rem)] focus-within:w-80 focus-within:overflow-auto focus-within:rounded-panel focus-within:border focus-within:border-line2 focus-within:bg-surface focus-within:p-1 focus-within:shadow-float"
        >
          {listEls}
        </ul>
      </div>
      <p id="mindmap-hint" className="m-0 flex items-center gap-1.5 text-[12px] text-muted">
        <Icon name="info" size={14} />
        {t("mindmap.hint", { context: platform })}
      </p>
    </div>
  );
}
