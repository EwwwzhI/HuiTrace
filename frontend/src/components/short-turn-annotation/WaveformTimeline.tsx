'use client';

import RegionsPlugin from 'wavesurfer.js/dist/plugins/regions.esm.js';
import MinimapPlugin from 'wavesurfer.js/dist/plugins/minimap.esm.js';
import TimelinePlugin from 'wavesurfer.js/dist/plugins/timeline.esm.js';
import WaveSurfer from 'wavesurfer.js';
import { useEffect, useRef, useState } from 'react';

export type WaveformEvent = { event_id: string; start_ms: number; end_ms: number; kind: string };

type Props = {
  media: HTMLMediaElement | null;
  sourceUrl: string;
  events: WaveformEvent[];
  selectedId: string | null;
  currentMs: number;
  onSeek: (milliseconds: number) => void;
  onCreate: (startMs: number, endMs: number) => void;
  onSelect: (eventId: string) => void;
  onBoundsChange: (eventId: string, startMs: number, endMs: number) => void;
};

/**
 * WaveSurfer Regions are a rendering/control surface only.  The parent owns
 * canonical AnnotationEvent state and persists it through the Tauri backend.
 */
export function WaveformTimeline({ media, sourceUrl, events, selectedId, currentMs, onSeek, onCreate, onSelect, onBoundsChange }: Props) {
  const waveform = useRef<HTMLDivElement>(null);
  const timeline = useRef<HTMLDivElement>(null);
  const instance = useRef<WaveSurfer | null>(null);
  const regions = useRef<RegionsPlugin | null>(null);
  const [zoom, setZoom] = useState(80);

  useEffect(() => {
    if (!waveform.current || !timeline.current || !media || !sourceUrl) return;
    const regionPlugin = RegionsPlugin.create();
    regions.current = regionPlugin;
    const ws = WaveSurfer.create({
      container: waveform.current,
      media,
      url: sourceUrl,
      height: 78,
      minPxPerSec: zoom,
      waveColor: '#2dd4bf',
      progressColor: '#0e7490',
      cursorColor: '#fbbf24',
      cursorWidth: 2,
      barWidth: 2,
      barGap: 1,
      barRadius: 2,
      autoScroll: true,
      autoCenter: false,
      plugins: [regionPlugin, TimelinePlugin.create({ container: timeline.current }), MinimapPlugin.create({ height: 28, waveColor: '#475569', progressColor: '#64748b' })],
    });
    instance.current = ws;
    regionPlugin.enableDragSelection({ color: 'rgba(45, 212, 191, 0.28)', minLength: 0.03 });
    regionPlugin.on('region-created', region => {
      if (events.some(event => event.event_id === region.id)) return;
      onCreate(Math.round(region.start * 1000), Math.round(region.end * 1000));
      region.remove();
    });
    regionPlugin.on('region-clicked', region => onSelect(region.id));
    regionPlugin.on('region-updated', region => {
      if (events.some(event => event.event_id === region.id)) onBoundsChange(region.id, Math.round(region.start * 1000), Math.round(region.end * 1000));
    });
    ws.on('interaction', () => onSeek(Math.round(ws.getCurrentTime() * 1000)));
    return () => { ws.destroy(); instance.current = null; regions.current = null; };
  // The media/source change is the only lifecycle reset; callback changes are
  // intentionally read through the current render because regions are rebuilt below.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [media, sourceUrl]);

  useEffect(() => { instance.current?.zoom(zoom); }, [zoom]);
  useEffect(() => { if (Math.abs((instance.current?.getCurrentTime() ?? 0) * 1000 - currentMs) > 80) instance.current?.setTime(currentMs / 1000); }, [currentMs]);
  useEffect(() => {
    const plugin = regions.current; if (!plugin) return;
    plugin.clearRegions();
    for (const event of events) {
      plugin.addRegion({ id: event.event_id, start: event.start_ms / 1000, end: event.end_ms / 1000, drag: true, resize: true, color: event.event_id === selectedId ? 'rgba(45, 212, 191, 0.78)' : 'rgba(45, 212, 191, 0.35)', content: event.kind, minLength: 0.001 });
    }
  }, [events, selectedId]);

  return <section className="rounded-xl border border-slate-800 bg-slate-900 p-3">
    <div className="mb-2 flex flex-wrap items-center justify-between gap-2"><div><p className="text-xs font-semibold uppercase tracking-widest text-slate-400">Waveform / canonical event regions</p><p className="text-xs text-slate-400">Drag to create; drag regions or their edges to edit. Regions never own persisted ground truth.</p></div><div className="flex gap-1"><button onClick={() => setZoom(value => Math.min(1200, value * 1.5))} className="rounded border border-slate-700 px-2 py-1 text-xs">Zoom in</button><button onClick={() => setZoom(value => Math.max(10, value / 1.5))} className="rounded border border-slate-700 px-2 py-1 text-xs">Zoom out</button><button onClick={() => setZoom(80)} className="rounded border border-slate-700 px-2 py-1 text-xs">Fit meeting</button></div></div>
    <div ref={waveform} className="min-h-[78px] rounded bg-slate-950" />
    <div ref={timeline} className="min-h-[22px] text-xs text-slate-400" />
  </section>;
}
