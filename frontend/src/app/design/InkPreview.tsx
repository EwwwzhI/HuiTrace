'use client';

import { useState } from 'react';
import { useTheme } from 'next-themes';
import { Mic, Pause, Square, Play, FileText, Settings2, Moon, Sun, PanelRightClose, PanelRightOpen } from 'lucide-react';
import { HomeOverview } from '@/components/HomeOverview';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Switch } from '@/components/ui/switch';
import { Dialog, DialogTrigger, DialogContent, DialogTitle, DialogDescription } from '@/components/ui/dialog';
import paneStyles from '@/components/MeetingDetails/CollapsibleSummaryPane.module.css';

const meetings = [
  { id: 'sample-1', title: '产品设计周会 · 让每一次交谈都有迹可循', date: '2026年9月12日 · 10:30' },
  { id: 'sample-2', title: '会迹品牌与体验评审', date: '2026年9月11日 · 15:00' },
  { id: 'sample-3', title: '与设计团队的一次长谈 / A conversation about thoughtful software', date: null },
];
const actions = [{ id: 'a', meeting_id: 'sample-1', meeting_title: meetings[0].title, text: '整理新版会议回顾页的设计反馈，并核对原文来源。', status: 'approved' }, { id: 'b', meeting_id: 'sample-2', meeting_title: meetings[1].title, text: '确认下一次体验评审的时间。', status: 'draft' }];

export default function InkPreview() {
  const { setTheme } = useTheme();
  const [screen, setScreen] = useState('home');
  const [tab, setTab] = useState('transcript');
  const [collapsed, setCollapsed] = useState(false);
  const [paused, setPaused] = useState(false);
  const [stopped, setStopped] = useState(false);
  const [category, setCategory] = useState('通用');
  return <div className="h-screen overflow-y-auto bg-background text-foreground">
    <header className="flex flex-wrap items-center justify-between gap-3 border-b border-border px-6 py-4">
      <div><h1 className="font-heading text-2xl">会迹 · Bento 第二版</h1><p className="mt-1 text-xs text-muted-foreground">设计验证页 · 以下会议内容均为演示数据</p></div>
      <div className="flex flex-wrap gap-2"><Button variant="outline" onClick={() => setTheme('light')}><Sun />宣纸</Button><Button variant="outline" onClick={() => setTheme('dark')}><Moon />夜墨</Button><Dialog><DialogTrigger asChild><Button variant="outline">检查弹层</Button></DialogTrigger><DialogContent><DialogTitle>保留交谈的温度</DialogTitle><DialogDescription>检查主题、对比度、键盘焦点和 Escape 关闭行为。</DialogDescription><Input aria-label="演示会议名称" placeholder="会议名称" /></DialogContent></Dialog></div>
    </header>
    <nav className="flex flex-wrap gap-2 border-b border-border p-3" aria-label="设计场景">
      {[['home','首页'],['recording','录音中'],['meeting','会议详情'],['settings','设置']].map(([key,label]) => <Button key={key} variant={screen===key?'default':'ghost'} aria-pressed={screen===key} onClick={()=>setScreen(key)}>{label}</Button>)}
    </nav>
    <div className="min-h-[520px]">
      {screen === 'home' && <HomeOverview meetings={meetings} total={3} actions={actions} onOpen={() => setScreen('meeting')} onRecord={() => setScreen('recording')} onImport={() => setScreen('recording')} onActions={() => setScreen('meeting')} onSettings={() => setScreen('settings')} />}
      {screen === 'recording' && <div className="flex min-h-[520px] flex-col">
        <header className="flex flex-wrap items-center justify-between gap-3 border-b border-border p-6"><h2 className="font-heading text-2xl">产品设计周会</h2><span className="inline-flex items-center gap-2 text-sm"><span className={`h-2 w-2 rounded-full ${paused?'bg-warning':'bg-seal'}`} />{stopped?'已保存':paused?'已暂停':'正在录音'}<span className="tabular-nums">12:48</span></span></header>
        <div className="mx-auto w-full max-w-3xl flex-1 space-y-7 p-8"><p className="text-xs text-muted-foreground">实时转写 · 演示</p>{['让界面安静一些，把注意力还给交谈本身。','原文、纪要与行动项之间，需要保留清楚的来源联系。','宣纸和墨色可以成为底色，重要状态要足够清楚。'].map((text,i)=><div key={text}><span className="text-xs tabular-nums text-muted-foreground">{`0${i+1}:24`} · 发言人 {i+1}</span><p className="mt-2 text-base leading-7">{text}</p></div>)}</div>
        <footer className="flex items-center justify-center gap-3 border-t border-border bg-card p-4"><Button variant="outline" disabled={stopped} onClick={()=>setPaused(!paused)}>{paused?<Play/>:<Pause/>}{paused?'继续':'暂停'}</Button><Button className="bg-seal text-seal-foreground hover:bg-seal/90" disabled={stopped} onClick={()=>setStopped(true)}><Square/>结束并保存</Button>{stopped&&<Button variant="ghost" onClick={()=>{setStopped(false);setPaused(false)}}>重置演示</Button>}</footer>
      </div>}
      <div hidden={screen !== 'meeting'} className={`ink-meeting ${paneStyles.root}`}>
        <header className="border-b border-border p-6"><p className="mb-2 text-xs text-seal">会议回顾</p><h2 className="font-heading text-2xl">产品设计周会 · 让每一次交谈都有迹可循</h2><p className="mt-2 text-xs text-muted-foreground">2026年9月12日 · 32 分钟 · 演示数据</p></header>
        <div className="flex items-center gap-4 border-b border-border px-6 py-3"><Play size={16}/><span className="text-xs tabular-nums">02:24 / 32:08</span><div className="h-1 flex-1 rounded bg-accent"><div className="h-full w-1/4 bg-primary"/></div></div>
        <div className={`${paneStyles.tabs} gap-2 border-b border-border p-3`}><Button aria-pressed={tab==='transcript'} variant={tab==='transcript'?'default':'ghost'} onClick={()=>setTab('transcript')}>原文</Button><Button aria-pressed={tab==='summary'} variant={tab==='summary'?'default':'ghost'} onClick={()=>setTab('summary')}>纪要</Button></div>
        <div className={`flex min-h-[320px] ${paneStyles.layout}`}>
          <section data-active={tab==='transcript'} className={`${paneStyles.transcript} flex-col p-6`}><h3 className="mb-6 flex items-center gap-2 text-sm font-semibold"><FileText size={16}/>原文</h3><p className="text-xs text-muted-foreground">02:24 · 发言人 1</p><p className="mt-3 text-base leading-7">原文、纪要与行动项之间，需要保留清楚的来源联系。点击引用时，应当能直接回到说出这句话的时刻。</p></section>
          <section data-active={tab==='summary'} data-collapsed={collapsed} className={paneStyles.pane}><div className={`${paneStyles.content} flex-col bg-card p-6`}><div className="mb-6 flex items-center justify-between"><h3 className="text-sm font-semibold">纪要</h3><Button className="ink-summary-collapse" variant="ghost" size="icon" aria-label="折叠纪要" onClick={()=>setCollapsed(true)}><PanelRightClose/></Button></div><p className="mb-2 text-xs text-warning">待审核 · AI 草稿</p><h4 className="font-heading text-xl">保留清晰的来源</h4><textarea aria-label="演示纪要草稿" className="mt-4 min-h-28 w-full resize-y rounded-lg border border-input bg-card p-3 text-base leading-7" defaultValue="在原文与纪要之间建立直接引用，让会议结论可以回到交谈现场。"/><button onClick={()=>setTab('transcript')} className="mt-4 border-l-2 border-primary bg-reference p-3 text-left text-sm text-reference-foreground">引用原文 · 02:24</button></div><div className={`${paneStyles.rail} bg-card`}><Button size="icon" variant="ghost" aria-label="展开纪要" onClick={()=>setCollapsed(false)}><PanelRightOpen/></Button></div></section>
        </div>
      </div>
      <div hidden={screen !== 'settings'} className="ink-settings p-6"><h2 className="mb-6 font-heading text-2xl">设置</h2><div className="ink-settings-grid"><nav className="ink-settings-nav flex gap-2" aria-label="演示设置分类">{['通用','录音','转写','摘要'].map(name=><Button key={name} variant={category===name?'default':'ghost'} onClick={()=>setCategory(name)}><Settings2/>{name}</Button>)}</nav><section className="ink-settings-panel rounded-xl border border-border bg-card p-6"><h3 className="text-base font-semibold">{category}</h3><div className="mt-6 flex items-center justify-between border-b border-border pb-5"><label htmlFor="demo-switch" className="text-sm">跟随系统外观</label><Switch id="demo-switch" defaultChecked /></div><label htmlFor="demo-draft" className="mt-5 block text-sm">会议默认名称</label><Input id="demo-draft" className="mt-2" defaultValue="我的会议"/><p className="mt-3 text-xs text-muted-foreground">切换分类或调整窗口后，输入内容仍保留。</p></section></div></div>
    </div>
    <section className="border-t border-border p-6"><h2 className="mb-4 text-sm font-semibold">组件与语义色</h2><div className="grid grid-cols-2 gap-3 md:grid-cols-4">{['bg-background','bg-card','bg-sidebar','bg-accent'].map(name=><div key={name} className={`${name} rounded-xl border border-border p-5 text-xs`}>{name.slice(3)}</div>)}</div><div className="mt-5 flex flex-wrap items-center gap-3"><Button><Mic/>主操作</Button><Button variant="outline">次操作</Button><Button disabled>不可用</Button><span className="text-success">✓ 已完成</span><span className="text-warning">△ 待审核</span><span className="text-destructive">! 操作失败</span></div></section>
  </div>;
}
