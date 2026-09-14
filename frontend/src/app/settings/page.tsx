'use client';

import { useEffect, useRef, useState } from 'react';
import dynamic from 'next/dynamic';
import { ArrowLeft, ArrowRight, Settings2, Mic, Database as DatabaseIcon, SparkleIcon, FlaskConical } from 'lucide-react';
import { useRouter } from 'next/navigation';

import { PreferenceSettings } from '@/components/PreferenceSettings';
import { useConfig } from '@/contexts/ConfigContext';
import { Tabs, TabsList, TabsTrigger, TabsContent } from '@/components/ui/tabs';
import { translateUI } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';
import { ThemeToggle } from '@/components/ThemeToggle';
import { Button } from '@/components/ui/button';
import { isShortTurnAnnotationEnabled } from '@/lib/shortTurnAnnotationFeature';


const loading = () => <p role="status" className="p-4 text-sm text-muted-foreground">{translateUI("Loading settings…")}</p>;
const TranscriptSettings = dynamic(() => import('@/components/TranscriptSettings').then(module => module.TranscriptSettings), { loading });
const RecordingSettings = dynamic(() => import('@/components/RecordingSettings').then(module => module.RecordingSettings), { loading });
const SummaryModelSettings = dynamic(() => import('@/components/SummaryModelSettings').then(module => module.SummaryModelSettings), { loading });

// Tabs configuration (constant)
const TABS = [
  { value: 'general', get label() { return translateUI("General"); }, icon: Settings2 },
  { value: 'recording', get label() { return translateUI("Recordings"); }, icon: Mic },
  { value: 'Transcriptionmodels', get label() { return translateUI("Transcription"); }, icon: DatabaseIcon },
  { value: 'summaryModels', get label() { return translateUI("Summary"); }, icon: SparkleIcon },
] as const;

export default function SettingsPage() {
  useUiTranslation();
  const router = useRouter();
  const { transcriptModelConfig, setTranscriptModelConfig } = useConfig();
  const annotationWorkspaceEnabled = isShortTurnAnnotationEnabled();
  const tabs = annotationWorkspaceEnabled
    ? [...TABS, { value: 'evaluation', label: translateUI('Evaluation Tools'), icon: FlaskConical }]
    : TABS;

  // Animation state for tabs
  const [activeTab, setActiveTab] = useState('general');
  const [visitedTabs, setVisitedTabs] = useState(() => new Set(['general']));
  const containerRef = useRef<HTMLDivElement>(null);
  const [verticalTabs, setVerticalTabs] = useState(false);
  useEffect(() => {
    const element = containerRef.current;
    if (!element || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(([entry]) => setVerticalTabs(entry.contentRect.width >= 900));
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const selectTab = (tab: string) => {
    setVisitedTabs(previous => new Set(previous).add(tab));
    setActiveTab(tab);
  };


  return (
    <div ref={containerRef} className="h-screen bg-background flex flex-col ink-settings">
      {/* Fixed Header */}
      <div className="v2-page-header sticky top-0 z-10">
        <div className="max-w-6xl mx-auto">
          <div className="flex flex-wrap items-center gap-4">
            <button
              onClick={() => router.back()}
              aria-label={translateUI('Back')}
              className="v2-back text-muted-foreground hover:text-foreground"
            >
              <ArrowLeft className="w-5 h-5" />
            </button>
            <div className="min-w-0 flex-1"><h1 className="font-heading text-[28px] font-semibold">{translateUI("Settings")}</h1><p className="v2-settings-intro">{translateUI('A workspace that feels like yours.')}</p></div>
            <ThemeToggle compact />
          </div>
        </div>
      </div>

      {/* Scrollable Content */}
      <div className="flex-1 overflow-y-auto">
        <div className="max-w-6xl mx-auto p-6">
          {/* Tabs */}
          <Tabs className="ink-settings-grid" orientation={verticalTabs ? 'vertical' : 'horizontal'} value={activeTab} onValueChange={selectTab}>
            <TabsList className="ink-settings-nav flex w-full justify-start overflow-x-auto bg-transparent rounded-none border-b border-border p-0 h-auto">
              {tabs.map((tab) => {
                const Icon = tab.icon;
                return (
                  <TabsTrigger
                    key={tab.value}
                    value={tab.value}
                    className="shrink-0 flex items-center justify-start gap-2 px-4 py-3 text-sm rounded-lg data-[state=active]:bg-accent data-[state=active]:text-accent-foreground data-[state=active]:shadow-none text-muted-foreground hover:text-foreground"
                  >
                    <Icon className="w-4 h-4" />
                    {tab.label}
                  </TabsTrigger>
                );
              })}
            </TabsList>

            <TabsContent className="ink-settings-panel" forceMount hidden={activeTab !== 'general'} value="general">
              {visitedTabs.has('general') && <PreferenceSettings />}
            </TabsContent>
            <TabsContent className="ink-settings-panel" forceMount hidden={activeTab !== 'recording'} value="recording">
              {visitedTabs.has('recording') && <RecordingSettings />}
            </TabsContent>
            <TabsContent className="ink-settings-panel v2-panel border bg-card p-6" forceMount hidden={activeTab !== 'Transcriptionmodels'} value="Transcriptionmodels">
              {visitedTabs.has('Transcriptionmodels') && <TranscriptSettings
                transcriptModelConfig={transcriptModelConfig}
                setTranscriptModelConfig={setTranscriptModelConfig}
              />}
            </TabsContent>
            <TabsContent className="ink-settings-panel" forceMount hidden={activeTab !== 'summaryModels'} value="summaryModels">
              {visitedTabs.has('summaryModels') && <SummaryModelSettings />}
            </TabsContent>
            {annotationWorkspaceEnabled && <TabsContent className="ink-settings-panel" forceMount hidden={activeTab !== 'evaluation'} value="evaluation">
              {visitedTabs.has('evaluation') && <section aria-labelledby="short-turn-workspace-title" className="v2-panel border bg-card p-6">
                <div className="flex flex-col gap-5 sm:flex-row sm:items-start sm:justify-between">
                  <div className="max-w-2xl">
                    <div className="mb-3 flex h-10 w-10 items-center justify-center rounded-xl bg-primary/10 text-primary"><FlaskConical aria-hidden="true" className="h-5 w-5" /></div>
                    <h2 id="short-turn-workspace-title" className="font-heading text-xl font-semibold">{translateUI('Short-Turn Annotation Workspace')}</h2>
                    <p className="mt-2 text-sm leading-6 text-muted-foreground">{translateUI('Build and review local Ground Truth for short-turn speaker diarization evaluation.')}</p>
                    <p className="mt-3 text-xs font-medium tracking-wide text-muted-foreground">{translateUI('Blind annotation · Review · Quality assurance · Benchmark Manifest')}</p>
                  </div>
                  <Button onClick={() => router.push('/dev/short-turn-annotation')} className="shrink-0 gap-2">
                    {translateUI('Open annotation workspace')}
                    <ArrowRight aria-hidden="true" className="h-4 w-4" />
                  </Button>
                </div>
              </section>}
            </TabsContent>}
          </Tabs>
        </div>
      </div>
    </div>
  );
};
