'use client';

import RegionsPlugin from 'wavesurfer.js/dist/plugins/regions.esm.js';
import MinimapPlugin from 'wavesurfer.js/dist/plugins/minimap.esm.js';
import TimelinePlugin from 'wavesurfer.js/dist/plugins/timeline.esm.js';
import WaveSurfer from 'wavesurfer.js';
import { useEffect, useRef, useState } from 'react';
import { useTheme } from 'next-themes';
import { shouldCreateRegion } from '@/lib/annotationIntegrity';
import { useUiTranslation } from '@/i18n/client';

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
  const { t } = useUiTranslation();
  const { resolvedTheme } = useTheme();
  const waveform = useRef<HTMLDivElement>(null);
  const timeline = useRef<HTMLDivElement>(null);
  const instance = useRef<WaveSurfer | null>(null);
  const regions = useRef<RegionsPlugin | null>(null);
  const dragSelectionCleanup = useRef<(() => void) | null>(null);
  const syncingRegionsRef = useRef(false);
  const eventsRef = useRef(events);
  const onCreateRef = useRef(onCreate);
  const onSelectRef = useRef(onSelect);
  const onBoundsChangeRef = useRef(onBoundsChange);
  const onSeekRef = useRef(onSeek);
  const [zoom, setZoom] = useState(80);
  const dark = resolvedTheme === 'dark';
  const colors = dark
    ? { wave: '#67e8f9', progress: '#99f6e4', cursor: '#fcd34d', region: 'rgba(34, 211, 238, 0.35)', selected: 'rgba(45, 212, 191, 0.78)' }
    : { wave: '#0d9488', progress: '#0f766e', cursor: '#d97706', region: 'rgba(8, 145, 178, 0.28)', selected: 'rgba(13, 148, 136, 0.68)' };

  eventsRef.current = events;
  onCreateRef.current = onCreate;
  onSelectRef.current = onSelect;
  onBoundsChangeRef.current = onBoundsChange;
  onSeekRef.current = onSeek;

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
      waveColor: colors.wave,
      progressColor: colors.progress,
      cursorColor: colors.cursor,
      cursorWidth: 2,
      barWidth: 2,
      barGap: 1,
      barRadius: 2,
      autoScroll: true,
      autoCenter: false,
      plugins: [regionPlugin, TimelinePlugin.create({ container: timeline.current }), MinimapPlugin.create({ height: 28, waveColor: '#64748b', progressColor: '#94a3b8' })],
    });
    instance.current = ws;
    dragSelectionCleanup.current = regionPlugin.enableDragSelection({ color: colors.region, minLength: 0.03 });
    regionPlugin.on('region-created', region => {
      if (!shouldCreateRegion(syncingRegionsRef.current, eventsRef.current.map(event => event.event_id), region.id)) return;
      onCreateRef.current(Math.round(region.start * 1000), Math.round(region.end * 1000));
      region.remove();
    });
    regionPlugin.on('region-clicked', region => onSelectRef.current(region.id));
    regionPlugin.on('region-updated', region => {
      if (!syncingRegionsRef.current && eventsRef.current.some(event => event.event_id === region.id)) onBoundsChangeRef.current(region.id, Math.round(region.start * 1000), Math.round(region.end * 1000));
    });
    ws.on('interaction', () => onSeekRef.current(Math.round(ws.getCurrentTime() * 1000)));
    return () => { dragSelectionCleanup.current?.(); dragSelectionCleanup.current = null; ws.destroy(); instance.current = null; regions.current = null; };
  // The media/source change is the only lifecycle reset; callback changes are
  // intentionally read through the current render because regions are rebuilt below.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [media, sourceUrl]);

  useEffect(() => {
    instance.current?.setOptions({ waveColor: colors.wave, progressColor: colors.progress, cursorColor: colors.cursor });
    if (regions.current) {
      dragSelectionCleanup.current?.();
      dragSelectionCleanup.current = regions.current.enableDragSelection({ color: colors.region, minLength: 0.03 });
    }
  }, [colors.cursor, colors.progress, colors.region, colors.wave]);
  useEffect(() => { instance.current?.zoom(zoom); }, [zoom]);
  useEffect(() => { if (Math.abs((instance.current?.getCurrentTime() ?? 0) * 1000 - currentMs) > 80) instance.current?.setTime(currentMs / 1000); }, [currentMs]);
  useEffect(() => {
    const plugin = regions.current; if (!plugin) return;
    syncingRegionsRef.current = true;
    try {
      plugin.clearRegions();
      for (const event of events) {
        plugin.addRegion({ id: event.event_id, start: event.start_ms / 1000, end: event.end_ms / 1000, drag: true, resize: true, color: event.event_id === selectedId ? colors.selected : colors.region, content: t(event.kind === 'short_speech' ? 'Short speech' : event.kind === 'backchannel' ? 'Backchannel' : event.kind === 'noise' ? 'Environmental noise' : event.kind === 'non_speech_vocalization' ? 'Non-speech vocalization' : 'Ordinary speech control'), minLength: 0.001 });
      }
    } finally {
      syncingRegionsRef.current = false;
    }
  }, [colors.region, colors.selected, events, selectedId, t]);

  return <section className="rounded-xl border border-border bg-card p-3">
    <div className="mb-2 flex flex-wrap items-center justify-between gap-2"><div><p className="text-xs font-semibold uppercase tracking-widest text-muted-foreground">{t('Waveform / canonical event regions')}</p><p className="text-xs text-muted-foreground">{t('Drag to create; drag regions or their edges to edit. Regions never own persisted Ground Truth.')}</p></div><div className="flex gap-1"><button onClick={() => setZoom(value => Math.min(1200, value * 1.5))} className="rounded border border-border bg-background px-2 py-1 text-xs hover:bg-accent">{t('Zoom in')}</button><button onClick={() => setZoom(value => Math.max(10, value / 1.5))} className="rounded border border-border bg-background px-2 py-1 text-xs hover:bg-accent">{t('Zoom out')}</button><button onClick={() => setZoom(80)} className="rounded border border-border bg-background px-2 py-1 text-xs hover:bg-accent">{t('Fit meeting')}</button></div></div>
    <div ref={waveform} className="min-h-[78px] rounded bg-muted/60" />
    <div ref={timeline} className="min-h-[22px] text-xs text-muted-foreground" />
  </section>;
}
