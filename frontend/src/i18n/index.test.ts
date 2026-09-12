import { describe, expect, it } from 'vitest';
import en from './en.json';
import zh from './zh-CN.json';
import { resolveUiLanguage, translateUI, uiI18n, validUiLanguage } from './index';

describe('UI language contract', () => {
  it('uses Simplified Chinese only for Chinese system locales', () => {
    expect(resolveUiLanguage('system', ['zh-CN', 'en-US'])).toBe('zh-CN');
    expect(resolveUiLanguage('system', ['en-US'])).toBe('en');
    expect(resolveUiLanguage('zh-CN', ['en-US'])).toBe('zh-CN');
  });

  it('rejects unknown persisted choices', () => {
    expect(validUiLanguage('fr')).toBe('system');
    expect(validUiLanguage('en')).toBe('en');
    expect(validUiLanguage('zh-CN')).toBe('zh-CN');
  });

  it('keeps the English fallback and the Chinese translation for core labels', async () => {
    expect(en['Interface language']).toBe('Interface language');
    expect(zh['Interface language']).toBe('界面语言');
    await uiI18n.changeLanguage('zh-CN');
    expect(translateUI('Settings')).toBe('设置');
    expect(translateUI('Daily Standup')).toBe('每日站会');
    expect(translateUI('Project Sync / Status Update')).toBe('项目同步 / 状态更新');
    expect(translateUI('Weekly or bi-weekly project status meeting focusing on milestones and risks.'))
      .toBe('聚焦里程碑与风险的每周或双周项目状态会议。');
    expect(translateUI('Retrospective (Agile)')).toBe('敏捷回顾');
    expect(translateUI('Sprint retrospective template for continuous improvement.'))
      .toBe('用于持续改进的迭代回顾模板。');
    expect(translateUI('Client / Sales Meeting')).toBe('客户 / 销售会议');
    expect(translateUI('Capture client goals, deliverables, and next steps.'))
      .toBe('记录客户目标、交付物与后续步骤。');
    expect(translateUI('Audio is unavailable for this meeting')).toBe('此会议的音频不可用');
    expect(translateUI('Reimport the original audio file to restore playback.'))
      .toBe('请重新导入原始音频文件以恢复播放。');
    expect(translateUI('Source: built-in template')).toBe('来源：内置模板');
    expect(translateUI('Using "{{name}}" template for summary generation', { name: '每日站会' }))
      .toBe('将使用“每日站会”模板生成摘要');
    await uiI18n.changeLanguage('en');
    expect(translateUI('Settings')).toBe('Settings');
  });
});
