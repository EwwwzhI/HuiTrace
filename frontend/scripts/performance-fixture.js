// Served ONLY by performance-server.cjs, never bundled in the application.
(() => {
  const metrics = { fixture: '100 of 1000 synthetic segments; read latency 80ms', clicks: [], longTasks: [], errors: [], calls: [], ready: {} };
  window.__HUITRACE_PERF__ = metrics;
  localStorage.setItem('mityu.onboardingTourCompleted', 'true');
  const meeting = { id: 'perf-meeting', title: 'Performance fixture', created_at: '2026-09-11T00:00:00Z', updated_at: '2026-09-11T00:00:00Z', transcript_count: 1000, folder_path: null };
  const reads = {
    get_onboarding_status: { completed: true, current_step: 5, model_status: {} },
    check_first_launch: false,
    get_recording_state: { is_recording: false, is_paused: false },
    api_diarization_availability: new URLSearchParams(location.search).has('speakers') ? { status: 'done', diarized_at: '2026-09-11', turn_count: 11 } : { status: 'noAudio' },
    api_get_speaker_turns: Array.from({ length: 11 }, (_, i) => ({ start_ms: i * 2000, end_ms: (i + 1) * 2000, speaker_label: 'Speaker ' + (i + 1), confidence: null })),
    api_get_meeting_summary_language: 'auto',
    'plugin:store|load': 1,
    'plugin:store|get': [null, false],
    api_list_templates: [],
    api_get_pending_recording_post_processing: [],
    api_get_current_workspace_id: 'perf-workspace',
    api_get_meetings: [meeting],
    api_get_meeting_metadata: meeting,
    api_get_summary: { status: 'completed', data: { markdown: '## Summary\n\nSynthetic performance fixture summary.' } },
    api_get_summary_draft: { draft: null, action_items: [], status: 'draft', model: null, template_id: null },
    api_get_model_config: { provider: 'builtin', model: 'fixture', apiKey: null },
    api_get_transcript_config: { provider: 'parakeet', model: 'fixture', language: 'en' },
    get_licensing_status: { state: 'licensed', configured: false, daysLeft: null },
    api_get_open_action_items: [],
    get_recording_preferences: { auto_summary: false },
    'plugin:app|version': '1.1.0',
  };
  let callback = 0;
  window.__TAURI_INTERNALS__ = {
    transformCallback: () => ++callback,
    unregisterCallback: () => {},
    convertFileSrc: () => '',
    invoke: async (cmd, args = {}) => {
      const start = performance.now();
      metrics.calls.push({ cmd, start });
      if (cmd.startsWith('plugin:event|')) return ++callback;
      if (cmd === 'api_get_meeting_transcripts') {
        await new Promise(resolve => setTimeout(resolve, 80));
        return { transcripts: Array.from({ length: args.limit || 100 }, (_, i) => ({ id: `perf-${i + (args.offset || 0)}`, text: `Synthetic transcript segment ${i + (args.offset || 0)}. Review the project milestones and assign the next action.`, audio_start_time: i * 5, audio_end_time: i * 5 + 4, timestamp: '00:00:00' })), total_count: 1000, has_more: true };
      }
      if (cmd in reads) { await new Promise(resolve => setTimeout(resolve, 80)); return reads[cmd]; }
      // Reject unmodeled commands, especially mutations: no native IPC exists.
      throw new Error(`Unmodeled fixture command: ${cmd}`);
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
  setInterval(() => {
    if (!document.body) return;
    let result = document.getElementById('__perf-result');
    if (!result) { result = document.createElement('pre'); result.id = '__perf-result'; result.hidden = true; document.body.append(result); }
    const resources = performance.getEntriesByType('resource').filter(e => /\.js(\?|$)/.test(e.name));
    const output = JSON.stringify({ ...metrics, jsBytes: resources.reduce((sum, e) => sum + e.decodedBodySize, 0), jsCount: resources.length, paints: performance.getEntriesByType('paint').map(e => ({name:e.name,time:e.startTime})) });
    if (result.textContent !== output) result.textContent = output;
    if (metrics.ready.summaryText) result.setAttribute('data-complete', 'true');
  }, 250);
  new PerformanceObserver(list => metrics.longTasks.push(...list.getEntries().map(e => ({ start: e.startTime, duration: e.duration })))).observe({ type: 'longtask', buffered: true });
  window.addEventListener('error', e => metrics.errors.push(e.message));
  document.addEventListener('click', e => {
    const button = e.target.closest('button,a,[role=tab]');
    if (button) metrics.clicks.push({ label: button.getAttribute('aria-label') || button.textContent.trim(), start: performance.now() });
  }, true);
  let queued = false;
  const inspect = () => {
    queued = false;
    const visible = selector => [...document.querySelectorAll(selector)].some(el => el.getBoundingClientRect().height > 0);
    for (const [key, selector] of Object.entries({ transcript: '[data-tour="transcript-panel"]', summary: '[aria-label="Legacy AI-generated summary is unverified"]', editor: '[contenteditable="true"]', settings: '[role="tablist"]', shell: 'button[aria-label="Settings"]' })) {
      if (!metrics.ready[key] && visible(selector)) metrics.ready[key] = performance.now();
    }
    if (!metrics.ready.summaryText && document.body?.textContent.includes('Synthetic performance fixture summary.')) metrics.ready.summaryText = performance.now();
  };
  new MutationObserver(() => { if (!queued) { queued = true; requestAnimationFrame(inspect); } }).observe(document, { childList: true, subtree: true });
})();
