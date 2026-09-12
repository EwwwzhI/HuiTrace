'use client';

import { useState } from 'react';
import { ArrowRight, ArrowUpRight, AudioLines, FileText, ListChecks, Mic, Settings2, Upload, Clock3 } from 'lucide-react';
import { translateUI } from '@/i18n';
import { useUiTranslation } from '@/i18n/client';
import { ThemeToggle } from './ThemeToggle';
import styles from './HomeOverview.module.css';

type MeetingRow = { id: string; title: string; date: string | null };
type ActionRow = { id: string; meeting_id: string; meeting_title: string; text: string; status: string; due?: string | null };

export function HomeOverview({ meetings, total, actions, onOpen, onRecord, onImport, onActions, onSettings, recordingDisabled = false }: {
  meetings: MeetingRow[]; total: number; actions: ActionRow[];
  onOpen: (id: string, title: string) => void;
  onRecord?: () => void; onImport?: () => void; onActions?: () => void; onSettings?: () => void;
  recordingDisabled?: boolean;
}) {
  useUiTranslation();
  const [category, setCategory] = useState('all');
  const latest = meetings[0];
  const tools = [
    { id: 'import', category: 'capture', icon: Upload, title: 'Import audio', description: 'Turn an existing recording into a clear meeting record.', action: onImport },
    { id: 'actions', category: 'organize', icon: ListChecks, title: 'Action Items', description: 'Review approved follow-ups with their source.', action: onActions },
    { id: 'settings', category: 'organize', icon: Settings2, title: 'Settings', description: 'Your devices, models and preferences. Just as you like them.', action: onSettings },
  ];

  return <div className={`${styles.scroll} custom-scrollbar`}>
    <div className={styles.page}>
      <header className={styles.header}>
        <div><p className={styles.eyebrow}>{translateUI('Your conversation workspace')}</p><h1>{translateUI('Welcome back')}</h1></div>
        <ThemeToggle compact />
      </header>
      <div className={styles.heroGrid}>
        <section className={`${styles.card} ${styles.hero}`}>
          <div className={styles.heroCopy}>
            <span className={styles.kicker}><AudioLines size={16} aria-hidden />{translateUI('A little more present.')}</span>
            <h2>{translateUI('Good conversations. Lasting ideas.')}</h2>
            <p>{translateUI('Stay in the conversation. Let HuiTrace keep the details.')}</p>
            <button type="button" className={styles.primary} onClick={onRecord} disabled={recordingDisabled || !onRecord}><Mic size={17} aria-hidden />{translateUI('Start recording')}<ArrowUpRight size={16} aria-hidden /></button>
          </div>
          <div className={styles.heroMark} aria-hidden="true"><div className={styles.orbit} /><img src="/huitrace-icon-yin-wave.svg" alt="" width={108} height={108} /><div className={styles.soundLine}>{[8, 14, 22, 12, 28, 18, 10, 24, 32, 16, 26, 12, 20, 8, 16].map((height, i) => <i key={i} style={{ height }} />)}</div></div>
        </section>
        <section className={`${styles.card} ${styles.continueCard}`}>
          <div className={styles.cardTop}><span className={styles.iconTile}><Clock3 size={19} aria-hidden /></span><span className={styles.eyebrow}>{translateUI('Pick up where you left off')}</span></div>
          <div className={styles.continueCopy}><h2>{latest?.title ?? translateUI('Room for your next idea.')}</h2><p>{latest?.date ?? translateUI('Every conversation has something worth keeping.')}</p></div>
          {latest ? <button className={styles.textButton} onClick={() => onOpen(latest.id, latest.title)}>{translateUI('Open meeting')}<ArrowRight size={16} aria-hidden /></button> : <span className={styles.emptyHint}>{translateUI('Your meetings will appear here.')}</span>}
        </section>
      </div>
      <section className={styles.toolSection} aria-labelledby="workspace-tools">
        <div className={styles.sectionHead}><h2 id="workspace-tools">{translateUI('Made for your workflow')}</h2><div className={styles.filters} role="group" aria-label={translateUI('Tool categories')}>
          {[['all', 'All tools'], ['capture', 'Capture'], ['organize', 'Organize']].map(([id, label]) => <button key={id} type="button" aria-pressed={category === id} onClick={() => setCategory(id)}>{translateUI(label)}</button>)}
        </div></div>
        <div className={styles.tools} key={category}>
          {tools.filter(tool => category === 'all' || tool.category === category).map(({ id, icon: Icon, title, description, action }) => <button key={id} type="button" className={`${styles.card} ${styles.tool}`} onClick={action} disabled={!action}>
            <span className={styles.toolTop}><span className={styles.iconTile}><Icon size={20} aria-hidden /></span><ArrowUpRight size={17} className={styles.arrow} aria-hidden /></span>
            <span className={styles.toolTitle}>{translateUI(title)}</span><span className={styles.toolDescription}>{translateUI(description)}</span>
          </button>)}
        </div>
      </section>
      <div className={styles.libraryGrid}>
        <section className={`${styles.card} ${styles.library}`} aria-labelledby="recent-meetings">
          <div className={styles.sectionHead}><div><p className={styles.eyebrow}>{translateUI('Your library')}</p><h2 id="recent-meetings">{translateUI('Recent meetings')}</h2></div><span className={styles.count}>{total}</span></div>
          {meetings.length ? <div className={styles.meetings}>{meetings.map(meeting => <button key={meeting.id} type="button" onClick={() => onOpen(meeting.id, meeting.title)} className={styles.meeting}>
            <span className={styles.documentIcon}><FileText size={18} aria-hidden /></span><span className={styles.meetingCopy}><strong>{meeting.title}</strong><small>{meeting.date ?? translateUI('Meeting report')}</small></span><ArrowUpRight size={16} className={styles.arrow} aria-hidden />
          </button>)}</div> : <div className={styles.empty}><FileText size={28} aria-hidden /><h3>{translateUI('Your first conversation starts here')}</h3><p>{translateUI('Start with the recording controls below, or import audio from the sidebar.')}</p></div>}
        </section>
        <section className={`${styles.card} ${styles.actionCard}`} aria-labelledby="open-actions">
          <div className={styles.sectionHead}><div><p className={styles.eyebrow}>{translateUI('From conversation to action')}</p><h2 id="open-actions">{translateUI('Open action items')}</h2></div><ListChecks size={20} className="text-muted-foreground" aria-hidden /></div>
          {actions.length ? <ul className={styles.actions}>{actions.slice(0, 4).map(item => <li key={item.id}><button type="button" onClick={() => onOpen(item.meeting_id, item.meeting_title)}><span className={styles.actionStatus} data-approved={item.status === 'approved'}>{translateUI(item.status === 'approved' ? 'Approved' : 'Awaiting review')}</span><strong>{item.text}</strong><small>{item.meeting_title}{item.due ? ` · ${item.due}` : ''}</small></button></li>)}</ul> : <div className={styles.empty}><ListChecks size={28} aria-hidden /><p>{translateUI('No action items to show. Action items from your meetings appear here.')}</p></div>}
          {onActions && <button type="button" className={styles.textButton} onClick={onActions}>{translateUI('View Action Items')}<ArrowRight size={16} aria-hidden /></button>}
        </section>
      </div>
    </div>
  </div>;
}
