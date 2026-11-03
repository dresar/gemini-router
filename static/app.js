/**
 * Gemini Key Router - Frontend Application
 * Vanilla JS Client for Admin REST API & Server-Sent Events (SSE)
 */

(() => {
  'use strict';

  // --- Constants & Config ---
  const STORAGE_KEY = 'gemini_router_token';
  const SSE_RECONNECT_MS = 3000;
  const GEMINI_KEY_REGEX = /AIza[0-9A-Za-z-_]{35}/g;

  // --- Application State ---
  const state = {
    token: localStorage.getItem(STORAGE_KEY) || '',
    keys: [],
    stats: {
      total_keys: 0,
      active_keys: 0,
      rate_limited_keys: 0,
      quota_exhausted_keys: 0,
      invalid_keys: 0,
      disabled_keys: 0,
      total_requests: 0,
      successful_requests: 0,
      failed_requests: 0,
      success_rate: 100,
      average_latency_ms: 0,
    },
    rotation: {
      strategy: 'round_robin',
      total_rotations: 0,
      failovers: 0,
      in_flight: 0,
      last_used_key: null,
      last_used_at: null,
    },
    settings: {
      strategy: 'round_robin',
      concurrency: 10,
      request_timeout_seconds: 30,
      retry_budget: 3,
      rate_limit_cooldown_seconds: 60,
      quota_cooldown_hours: 24,
    },
    filter: {
      search: '',
      status: 'ALL',
    },
    sort: {
      field: 'alias',
      direction: 'asc',
    },
    testProgress: {
      running: false,
      total: 0,
      completed: 0,
      passed: 0,
      failed: 0,
    },
    sse: {
      eventSource: null,
      reconnectTimer: null,
      isConnected: false,
    },
  };

  // --- DOM Elements ---
  const el = {
    // Header
    sseIndicator: document.getElementById('sse-indicator'),
    sseText: document.getElementById('sse-text'),
    strategyBadge: document.getElementById('strategy-badge'),
    activeStrategyName: document.getElementById('active-strategy-name'),
    openSettingsBtn: document.getElementById('open-settings-btn'),
    authBtn: document.getElementById('auth-btn'),
    authBtnLabel: document.getElementById('auth-btn-label'),

    // Stats
    statTotalKeys: document.getElementById('stat-total-keys'),
    statActiveKeys: document.getElementById('stat-active-keys'),
    statRateLimited: document.getElementById('stat-rate-limited'),
    statQuotaExhausted: document.getElementById('stat-quota-exhausted'),
    statInvalidKeys: document.getElementById('stat-invalid-keys'),
    statTotalRequests: document.getElementById('stat-total-requests'),
    statReqBreakdown: document.getElementById('stat-req-breakdown'),
    statSuccessRate: document.getElementById('stat-success-rate'),
    statAvgLatency: document.getElementById('stat-avg-latency'),

    // Test Progress
    testProgressSection: document.getElementById('test-progress-section'),
    testProgressTitle: document.getElementById('test-progress-title'),
    testProgressDetails: document.getElementById('test-progress-details'),
    testProgressPassed: document.getElementById('test-progress-passed'),
    testProgressFailed: document.getElementById('test-progress-failed'),
    testProgressBar: document.getElementById('test-progress-bar'),
    cancelTestBtn: document.getElementById('cancel-test-btn'),

    // Rotation Monitor
    monitorStrategyTag: document.getElementById('monitor-strategy-tag'),
    rotationCount: document.getElementById('rotation-count'),
    failoverCount: document.getElementById('failover-count'),
    inFlightCount: document.getElementById('in-flight-count'),
    lastUsedKeyDisplay: document.getElementById('last-used-key-display'),

    // Toolbar
    searchInput: document.getElementById('search-input'),
    clearSearchBtn: document.getElementById('clear-search-btn'),
    statusFilter: document.getElementById('status-filter'),
    openImportBtn: document.getElementById('open-import-btn'),
    openAddBtn: document.getElementById('open-add-btn'),
    testAllBtn: document.getElementById('test-all-btn'),
    testAllLabel: document.getElementById('test-all-label'),
    refreshKeysBtn: document.getElementById('refresh-keys-btn'),

    // Table
    keysTableBody: document.getElementById('keys-table-body'),
    tableCountLabel: document.getElementById('table-count-label'),

    // Modals
    loginModal: document.getElementById('login-modal'),
    loginForm: document.getElementById('login-form'),
    adminTokenInput: document.getElementById('admin-token-input'),
    toggleTokenVisibilityBtn: document.getElementById('toggle-token-visibility-btn'),
    loginErrorAlert: document.getElementById('login-error-alert'),

    importModal: document.getElementById('import-modal'),
    importForm: document.getElementById('import-form'),
    dropZone: document.getElementById('drop-zone'),
    fileUploadInput: document.getElementById('file-upload-input'),
    bulkKeysInput: document.getElementById('bulk-keys-input'),
    detectedKeysCount: document.getElementById('detected-keys-count'),
    importFeedback: document.getElementById('import-feedback'),
    submitImportBtn: document.getElementById('submit-import-btn'),

    addModal: document.getElementById('add-modal'),
    addKeyForm: document.getElementById('add-key-form'),
    newApiKey: document.getElementById('new-api-key'),
    newKeyAlias: document.getElementById('new-key-alias'),
    addFeedback: document.getElementById('add-feedback'),
    submitAddBtn: document.getElementById('submit-add-btn'),

    settingsModal: document.getElementById('settings-modal'),
    settingsForm: document.getElementById('settings-form'),
    settingStrategy: document.getElementById('setting-strategy'),
    settingConcurrency: document.getElementById('setting-concurrency'),
    settingTimeout: document.getElementById('setting-timeout'),
    settingRetryBudget: document.getElementById('setting-retry-budget'),
    settingCooldownRatelimit: document.getElementById('setting-cooldown-ratelimit'),
    settingCooldownQuota: document.getElementById('setting-cooldown-quota'),
    settingsFeedback: document.getElementById('settings-feedback'),
    saveSettingsBtn: document.getElementById('save-settings-btn'),

    // Toasts
    toastContainer: document.getElementById('toast-container'),
  };

  // --- API Client Helpers ---

  /**
   * Universal fetch wrapper with Bearer token and JSON handling
   */
  async function apiFetch(path, options = {}) {
    const headers = {
      'Content-Type': 'application/json',
      Accept: 'application/json',
      ...options.headers,
    };

    if (state.token) {
      headers['Authorization'] = `Bearer ${state.token}`;
      headers['x-admin-key'] = state.token;
    }

    try {
      const response = await fetch(path, { ...options, headers });

      if (response.status === 401 || response.status === 403) {
        showLoginModal('Session invalid or unauthorized. Please re-enter your admin secret.');
        throw new Error('Unauthorized');
      }

      if (!response.ok) {
        const errData = await response.json().catch(() => ({}));
        const msg = errData.error || errData.message || `HTTP ${response.status}: ${response.statusText}`;
        throw new Error(msg);
      }

      if (response.status === 204) {
        return null;
      }

      return await response.json();
    } catch (err) {
      if (err.message !== 'Unauthorized') {
        console.error(`API Error [${path}]:`, err);
      }
      throw err;
    }
  }

  // --- Toast Notification System ---
  function showToast(message, type = 'info', duration = 3500) {
    if (!el.toastContainer) return;

    const toast = document.createElement('div');
    toast.className = `toast toast-${type}`;

    let iconSvg = '';
    if (type === 'emerald') {
      iconSvg = '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="20 6 9 17 4 12"></polyline></svg>';
    } else if (type === 'danger') {
      iconSvg = '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"></circle><line x1="12" y1="8" x2="12" y2="12"></line><line x1="12" y1="16" x2="12.01" y2="16"></line></svg>';
    } else if (type === 'warning') {
      iconSvg = '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z"></path><line x1="12" y1="9" x2="12" y2="13"></line><line x1="12" y1="17" x2="12.01" y2="17"></line></svg>';
    } else {
      iconSvg = '<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"></circle><line x1="12" y1="16" x2="12" y2="12"></line><line x1="12" y1="8" x2="12.01" y2="8"></line></svg>';
    }

    toast.innerHTML = `
      <span class="metric-icon ${type}">${iconSvg}</span>
      <span class="toast-msg">${escapeHtml(message)}</span>
      <button class="toast-close" title="Dismiss">&times;</button>
    `;

    toast.querySelector('.toast-close').addEventListener('click', () => {
      toast.remove();
    });

    el.toastContainer.appendChild(toast);

    setTimeout(() => {
      toast.style.opacity = '0';
      setTimeout(() => toast.remove(), 250);
    }, duration);
  }

  // --- SSE Real-Time Connection ---
  function initSSE() {
    if (!state.token) {
      updateSSEStatus(false, 'Waiting for login');
      return;
    }

    if (state.sse.eventSource) {
      state.sse.eventSource.close();
      state.sse.eventSource = null;
    }

    if (state.sse.reconnectTimer) {
      clearTimeout(state.sse.reconnectTimer);
      state.sse.reconnectTimer = null;
    }

    try {
      const url = `/admin/events?token=${encodeURIComponent(state.token)}`;
      const es = new EventSource(url);
      state.sse.eventSource = es;

      es.onopen = () => {
        updateSSEStatus(true, 'SSE: Connected');
      };

      es.onerror = () => {
        updateSSEStatus(false, 'SSE: Reconnecting...');
        es.close();
        state.sse.eventSource = null;
        state.sse.reconnectTimer = setTimeout(initSSE, SSE_RECONNECT_MS);
      };

      // General message event handler
      es.onmessage = (event) => {
        if (!event.data) return;
        try {
          const payload = JSON.parse(event.data);
          handleSSEMessage(payload);
        } catch (err) {
          console.warn('Failed to parse SSE payload:', event.data, err);
        }
      };

      // Custom event types
      es.addEventListener('test_progress', (e) => {
        try { handleSSEMessage(JSON.parse(e.data)); } catch (_) {}
      });
      es.addEventListener('test_complete', (e) => {
        try { handleSSEMessage(JSON.parse(e.data)); } catch (_) {}
      });
      es.addEventListener('key_update', (e) => {
        try { handleSSEMessage(JSON.parse(e.data)); } catch (_) {}
      });
      es.addEventListener('rotation', (e) => {
        try { handleSSEMessage(JSON.parse(e.data)); } catch (_) {}
      });
      es.addEventListener('stats', (e) => {
        try { handleSSEMessage(JSON.parse(e.data)); } catch (_) {}
      });
    } catch (err) {
      console.error('SSE initialization failed:', err);
      updateSSEStatus(false, 'SSE: Disconnected');
      state.sse.reconnectTimer = setTimeout(initSSE, SSE_RECONNECT_MS);
    }
  }

  function updateSSEStatus(connected, text) {
    state.sse.isConnected = connected;
    if (el.sseIndicator) {
      el.sseIndicator.className = `sse-badge ${connected ? 'connected' : 'disconnected'}`;
    }
    if (el.sseText) {
      el.sseText.textContent = text;
    }
  }

  function handleSSEMessage(payload) {
    if (!payload) return;

    const type = payload.type || payload.event;

    switch (type) {
      case 'test_progress': {
        const { completed, total, passed, failed, status, key_id, latency_ms } = payload;
        updateTestProgress({
          running: true,
          total: total !== undefined ? total : state.testProgress.total,
          completed: completed !== undefined ? completed : state.testProgress.completed + 1,
          passed: passed !== undefined ? passed : state.testProgress.passed,
          failed: failed !== undefined ? failed : state.testProgress.failed,
        });

        // If specific key was tested, update it in local state table
        if (key_id && status) {
          updateKeyInState({ id: key_id, status, average_latency_ms: latency_ms });
        }
        break;
      }

      case 'test_complete': {
        updateTestProgress({
          running: false,
          total: payload.total || state.testProgress.total,
          completed: payload.total || state.testProgress.completed,
          passed: payload.passed || state.testProgress.passed,
          failed: payload.failed || state.testProgress.failed,
        });
        showToast(
          `Fast Test completed: ${payload.passed || 0} passed, ${payload.failed || 0} failed.`,
          payload.failed > 0 ? 'warning' : 'emerald'
        );
        fetchStats();
        fetchKeys();
        break;
      }

      case 'key_update': {
        if (payload.key) {
          updateKeyInState(payload.key);
        }
        break;
      }

      case 'key_added': {
        if (payload.key) {
          state.keys.unshift(payload.key);
          renderKeysTable();
          fetchStats();
        }
        break;
      }

      case 'key_deleted': {
        const id = payload.id || (payload.key && payload.key.id);
        if (id) {
          state.keys = state.keys.filter((k) => k.id !== id);
          renderKeysTable();
          fetchStats();
        }
        break;
      }

      case 'rotation': {
        if (payload.strategy) state.rotation.strategy = payload.strategy;
        if (payload.rotations !== undefined) state.rotation.total_rotations = payload.rotations;
        if (payload.total_rotations !== undefined) state.rotation.total_rotations = payload.total_rotations;
        if (payload.failovers !== undefined) state.rotation.failovers = payload.failovers;
        if (payload.in_flight !== undefined) state.rotation.in_flight = payload.in_flight;
        if (payload.key) state.rotation.last_used_key = payload.key;
        if (payload.masked_key) state.rotation.last_used_key = payload.masked_key;
        renderRotationMonitor();
        break;
      }

      case 'stats': {
        Object.assign(state.stats, payload);
        renderStats();
        break;
      }

      default:
        // Generic event object handler
        if (payload.total_keys !== undefined) {
          Object.assign(state.stats, payload);
          renderStats();
        }
        break;
    }
  }

  function updateKeyInState(updatedKey) {
    const idx = state.keys.findIndex((k) => k.id === updatedKey.id);
    if (idx !== -1) {
      state.keys[idx] = { ...state.keys[idx], ...updatedKey };
      renderKeysTable();
    }
  }

  // --- API Actions ---

  async function fetchStats() {
    try {
      const data = await apiFetch('/admin/stats');
      if (data) {
        state.stats = { ...state.stats, ...data };
        renderStats();
      }
    } catch (_) {
      // Handled in apiFetch
    }
  }

  async function fetchKeys() {
    try {
      const data = await apiFetch('/admin/keys');
      state.keys = Array.isArray(data) ? data : data.keys || [];
      renderKeysTable();
    } catch (_) {
      el.keysTableBody.innerHTML = `
        <tr>
          <td colspan="7" class="table-empty">
            Failed to load keys. Check authorization or connection.
          </td>
        </tr>
      `;
    }
  }

  async function fetchRotation() {
    try {
      const data = await apiFetch('/admin/rotation');
      if (data) {
        state.rotation = { ...state.rotation, ...data };
        renderRotationMonitor();
      }
    } catch (_) {
      // Rotation metrics might not be initialized yet
    }
  }

  async function fetchSettings() {
    try {
      const data = await apiFetch('/admin/settings');
      if (data) {
        state.settings = { ...state.settings, ...data };
        populateSettingsForm();
        updateStrategyBadge(state.settings.strategy);
      }
    } catch (_) {
      // Handled in apiFetch
    }
  }

  async function submitBulkImport(keysArray) {
    el.submitImportBtn.disabled = true;
    el.submitImportBtn.innerHTML = '<span class="spinner-sm"></span> Importing...';
    el.importFeedback.style.display = 'none';

    try {
      const res = await apiFetch('/admin/keys/import', {
        method: 'POST',
        body: JSON.stringify({ keys: keysArray }),
      });

      const imported = res.imported !== undefined ? res.imported : keysArray.length;
      const skipped = res.skipped !== undefined ? res.skipped : 0;

      showToast(`Successfully imported ${imported} keys (${skipped} skipped).`, 'emerald');
      closeModal('import-modal');
      el.bulkKeysInput.value = '';
      updateDetectedCount();
      await fetchKeys();
      await fetchStats();
    } catch (err) {
      el.importFeedback.className = 'alert alert-danger';
      el.importFeedback.textContent = `Import failed: ${err.message}`;
      el.importFeedback.style.display = 'block';
    } finally {
      el.submitImportBtn.disabled = false;
      el.submitImportBtn.innerHTML = '<span>Import Detected Keys</span>';
    }
  }

  async function submitAddKey(apiKey, alias) {
    el.submitAddBtn.disabled = true;
    el.submitAddBtn.innerHTML = '<span class="spinner-sm"></span> Adding...';
    el.addFeedback.style.display = 'none';

    try {
      await apiFetch('/admin/keys', {
        method: 'POST',
        body: JSON.stringify({
          key: apiKey.trim(),
          alias: (alias || '').trim(),
        }),
      });

      showToast('API Key added successfully.', 'emerald');
      closeModal('add-modal');
      el.addKeyForm.reset();
      await fetchKeys();
      await fetchStats();
    } catch (err) {
      el.addFeedback.className = 'alert alert-danger';
      el.addFeedback.textContent = `Failed to add key: ${err.message}`;
      el.addFeedback.style.display = 'block';
    } finally {
      el.submitAddBtn.disabled = false;
      el.submitAddBtn.innerHTML = '<span>Add Key</span>';
    }
  }

  async function deleteKey(id, aliasOrMasked) {
    if (!confirm(`Are you sure you want to delete key "${aliasOrMasked}"?\nThis action cannot be undone.`)) {
      return;
    }

    try {
      await apiFetch(`/admin/keys/${encodeURIComponent(id)}`, {
        method: 'DELETE',
      });

      showToast(`Key deleted successfully.`, 'emerald');
      state.keys = state.keys.filter((k) => k.id !== id);
      renderKeysTable();
      await fetchStats();
    } catch (err) {
      showToast(`Delete failed: ${err.message}`, 'danger');
    }
  }

  async function toggleKeyStatus(id, currentStatus) {
    const newStatus = currentStatus === 'ACTIVE' ? 'DISABLED' : 'ACTIVE';

    try {
      await apiFetch(`/admin/keys/${encodeURIComponent(id)}`, {
        method: 'PATCH',
        body: JSON.stringify({ status: newStatus }),
      });

      showToast(`Key status changed to ${newStatus}.`, 'emerald');
      updateKeyInState({ id, status: newStatus });
      await fetchStats();
    } catch (err) {
      showToast(`Status update failed: ${err.message}`, 'danger');
    }
  }

  async function testSingleKey(id) {
    try {
      showToast('Testing single key upstream...', 'info');
      const res = await apiFetch('/admin/keys/test', {
        method: 'POST',
        body: JSON.stringify({ id }),
      });

      const latency = res.latency_ms || res.average_latency_ms || 0;
      const status = res.status || 'ACTIVE';

      if (status === 'ACTIVE') {
        showToast(`Key test PASSED (${latency}ms)`, 'emerald');
      } else {
        showToast(`Key test FAILED: ${res.error || status}`, 'danger');
      }

      updateKeyInState({ id, status, average_latency_ms: latency });
      await fetchStats();
    } catch (err) {
      showToast(`Test failed: ${err.message}`, 'danger');
    }
  }

  async function testAllKeys() {
    if (state.testProgress.running) {
      showToast('A test batch is already running.', 'warning');
      return;
    }

    el.testAllBtn.disabled = true;
    el.testAllLabel.textContent = 'Triggering...';

    try {
      const res = await apiFetch('/admin/keys/test-all', {
        method: 'POST',
      });

      const total = res.total || state.keys.length;
      updateTestProgress({
        running: true,
        total: total,
        completed: 0,
        passed: 0,
        failed: 0,
      });

      showToast(`Triggered parallel fast test on ${total} keys.`, 'emerald');
    } catch (err) {
      showToast(`Failed to trigger parallel test: ${err.message}`, 'danger');
      updateTestProgress({ running: false });
    } finally {
      el.testAllBtn.disabled = false;
      el.testAllLabel.textContent = 'Test All Keys';
    }
  }

  async function saveSettings(payload) {
    el.saveSettingsBtn.disabled = true;
    el.saveSettingsBtn.innerHTML = '<span class="spinner-sm"></span> Saving...';
    el.settingsFeedback.style.display = 'none';

    try {
      const updated = await apiFetch('/admin/settings', {
        method: 'PATCH',
        body: JSON.stringify(payload),
      });

      state.settings = { ...state.settings, ...payload, ...(updated || {}) };
      updateStrategyBadge(state.settings.strategy);
      showToast('Settings updated successfully.', 'emerald');
      closeModal('settings-modal');
      fetchRotation();
    } catch (err) {
      el.settingsFeedback.className = 'alert alert-danger';
      el.settingsFeedback.textContent = `Save failed: ${err.message}`;
      el.settingsFeedback.style.display = 'block';
    } finally {
      el.saveSettingsBtn.disabled = false;
      el.saveSettingsBtn.innerHTML = '<span>Save Settings</span>';
    }
  }

  // --- Rendering UI ---

  function renderStats() {
    const s = state.stats;
    if (el.statTotalKeys) el.statTotalKeys.textContent = formatNum(s.total_keys);
    if (el.statActiveKeys) el.statActiveKeys.textContent = formatNum(s.active_keys);
    if (el.statRateLimited) el.statRateLimited.textContent = formatNum(s.rate_limited_keys);
    if (el.statQuotaExhausted) el.statQuotaExhausted.textContent = formatNum(s.quota_exhausted_keys);
    if (el.statInvalidKeys) el.statInvalidKeys.textContent = formatNum(s.invalid_keys);
    if (el.statTotalRequests) el.statTotalRequests.textContent = formatNum(s.total_requests);

    if (el.statReqBreakdown) {
      el.statReqBreakdown.textContent = `${formatNum(s.successful_requests)} ok / ${formatNum(s.failed_requests)} err`;
    }

    if (el.statSuccessRate) {
      let rate = 100;
      if (s.total_requests > 0) {
        rate = ((s.successful_requests / s.total_requests) * 100).toFixed(1);
      } else if (s.success_rate !== undefined) {
        rate = Number(s.success_rate).toFixed(1);
      }
      el.statSuccessRate.textContent = `${rate}%`;
    }

    if (el.statAvgLatency) {
      const lat = s.average_latency_ms || 0;
      el.statAvgLatency.textContent = `${Math.round(lat)} ms`;
    }
  }

  function renderRotationMonitor() {
    const r = state.rotation;
    const stratName = formatStrategyName(r.strategy || state.settings.strategy);

    if (el.monitorStrategyTag) el.monitorStrategyTag.textContent = stratName;
    if (el.rotationCount) el.rotationCount.textContent = formatNum(r.total_rotations);
    if (el.failoverCount) el.failoverCount.textContent = formatNum(r.failovers);
    if (el.inFlightCount) el.inFlightCount.textContent = formatNum(r.in_flight);

    if (el.lastUsedKeyDisplay) {
      if (r.last_used_key) {
        el.lastUsedKeyDisplay.textContent = r.last_used_key;
        el.lastUsedKeyDisplay.title = r.last_used_at ? `Last used: ${new Date(r.last_used_at).toLocaleTimeString()}` : '';
      } else {
        el.lastUsedKeyDisplay.textContent = 'None';
      }
    }
  }

  function updateStrategyBadge(strat) {
    const name = formatStrategyName(strat);
    if (el.activeStrategyName) {
      el.activeStrategyName.textContent = `Strategy: ${name}`;
    }
    if (el.monitorStrategyTag) {
      el.monitorStrategyTag.textContent = name;
    }
  }

  function updateTestProgress(progress) {
    Object.assign(state.testProgress, progress);
    const p = state.testProgress;

    if (!p.running && p.completed === 0) {
      el.testProgressSection.style.display = 'none';
      return;
    }

    el.testProgressSection.style.display = 'block';

    const pct = p.total > 0 ? Math.min(100, Math.round((p.completed / p.total) * 100)) : 0;
    el.testProgressBar.style.width = `${pct}%`;
    el.testProgressDetails.textContent = `(${p.completed} / ${p.total} keys tested - ${pct}%)`;
    el.testProgressPassed.textContent = `Passed: ${p.passed}`;
    el.testProgressFailed.textContent = `Failed: ${p.failed}`;

    if (!p.running) {
      el.testProgressTitle.textContent = 'Fast Test Batch Complete';
      el.testProgressBar.style.backgroundColor = p.failed > 0 ? 'var(--accent-warning)' : 'var(--accent-emerald)';
    } else {
      el.testProgressTitle.textContent = 'Parallel Fast Tester Running...';
      el.testProgressBar.style.backgroundColor = 'var(--accent-emerald)';
    }
  }

  function renderKeysTable() {
    let filtered = state.keys.filter((key) => {
      // Search Filter (alias or masked_key)
      if (state.filter.search) {
        const query = state.filter.search.toLowerCase();
        const aliasMatch = (key.alias || '').toLowerCase().includes(query);
        const keyMatch = (key.masked_key || '').toLowerCase().includes(query);
        const idMatch = (key.id || '').toLowerCase().includes(query);
        if (!aliasMatch && !keyMatch && !idMatch) return false;
      }

      // Status Filter
      if (state.filter.status !== 'ALL') {
        if (key.status !== state.filter.status) return false;
      }

      return true;
    });

    // Sort
    filtered.sort((a, b) => {
      let valA = a[state.sort.field];
      let valB = b[state.sort.field];

      if (state.sort.field === 'requests') {
        valA = a.total_requests || 0;
        valB = b.total_requests || 0;
      } else if (state.sort.field === 'latency') {
        valA = a.average_latency_ms || 0;
        valB = b.average_latency_ms || 0;
      } else if (state.sort.field === 'cooldown') {
        valA = a.cooldown_expires_at ? new Date(a.cooldown_expires_at).getTime() : 0;
        valB = b.cooldown_expires_at ? new Date(b.cooldown_expires_at).getTime() : 0;
      } else {
        valA = (valA || '').toString().toLowerCase();
        valB = (valB || '').toString().toLowerCase();
      }

      if (valA < valB) return state.sort.direction === 'asc' ? -1 : 1;
      if (valA > valB) return state.sort.direction === 'asc' ? 1 : -1;
      return 0;
    });

    // Update table count footer
    if (el.tableCountLabel) {
      el.tableCountLabel.textContent = `Showing ${filtered.length} of ${state.keys.length} keys`;
    }

    if (filtered.length === 0) {
      el.keysTableBody.innerHTML = `
        <tr>
          <td colspan="7" class="table-empty">
            No API keys found matching the current criteria.
          </td>
        </tr>
      `;
      return;
    }

    el.keysTableBody.innerHTML = filtered
      .map((key) => {
        const statusClass = `status-${(key.status || 'active').toLowerCase().replace(/_/g, '-')}`;
        const total = formatNum(key.total_requests || 0);
        const succ = formatNum(key.successful_requests || 0);
        const fail = formatNum(key.failed_requests || 0);
        const lat = key.average_latency_ms ? `${Math.round(key.average_latency_ms)} ms` : '&mdash;';
        const cooldown = formatCooldown(key.cooldown_expires_at);

        return `
          <tr data-key-id="${escapeHtml(key.id)}">
            <td>
              <div class="key-alias-wrap">
                <span class="key-alias">${escapeHtml(key.alias || 'Unnamed Key')}</span>
                <span class="key-id">${escapeHtml(key.id)}</span>
              </div>
            </td>
            <td>
              <div class="masked-key-wrap" title="Click icon to copy full/masked key">
                <span>${escapeHtml(key.masked_key || 'AIzaSy...')}</span>
                <button class="copy-key-btn" data-copy-key="${escapeHtml(key.masked_key)}" title="Copy key">
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <rect x="9" y="9" width="13" height="13" rx="2" ry="2"></rect>
                    <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"></path>
                  </svg>
                </button>
              </div>
            </td>
            <td>
              <span class="badge ${statusClass}">${escapeHtml(key.status || 'UNKNOWN')}</span>
            </td>
            <td>
              <span class="font-mono">${total}</span>
              <span class="text-muted font-mono" style="font-size: 0.75rem;">(${succ} ok / ${fail} err)</span>
            </td>
            <td>
              <span class="font-mono">${lat}</span>
            </td>
            <td>
              <span class="font-mono ${cooldown.isExpired ? 'text-muted' : 'text-warning'}">${cooldown.text}</span>
            </td>
            <td class="td-actions">
              <div class="table-action-group">
                <button class="btn btn-secondary btn-sm" data-action="test" data-id="${escapeHtml(key.id)}" title="Test key with upstream probe">
                  Test
                </button>
                <button class="btn btn-secondary btn-sm" data-action="toggle" data-id="${escapeHtml(key.id)}" data-status="${escapeHtml(key.status)}" title="Toggle active status">
                  ${key.status === 'ACTIVE' ? 'Disable' : 'Enable'}
                </button>
                <button class="btn btn-danger btn-sm" data-action="delete" data-id="${escapeHtml(key.id)}" data-alias="${escapeHtml(key.alias || key.masked_key)}" title="Delete key">
                  Delete
                </button>
              </div>
            </td>
          </tr>
        `;
      })
      .join('');
  }

  // --- Modal Helpers ---

  function openModal(modalId) {
    const modal = document.getElementById(modalId);
    if (modal) {
      modal.style.display = 'flex';
    }
  }

  function closeModal(modalId) {
    const modal = document.getElementById(modalId);
    if (modal) {
      modal.style.display = 'none';
    }
  }

  function showLoginModal(errorMessage = '') {
    if (el.loginErrorAlert) {
      if (errorMessage) {
        el.loginErrorAlert.textContent = errorMessage;
        el.loginErrorAlert.style.display = 'block';
      } else {
        el.loginErrorAlert.style.display = 'none';
      }
    }
    openModal('login-modal');
  }

  function populateSettingsForm() {
    const s = state.settings;
    if (el.settingStrategy) el.settingStrategy.value = s.strategy || 'round_robin';
    if (el.settingConcurrency) el.settingConcurrency.value = s.concurrency || 10;
    if (el.settingTimeout) el.settingTimeout.value = s.request_timeout_seconds || 30;
    if (el.settingRetryBudget) el.settingRetryBudget.value = s.retry_budget || 3;
    if (el.settingCooldownRatelimit) el.settingCooldownRatelimit.value = s.rate_limit_cooldown_seconds || 60;
    if (el.settingCooldownQuota) el.settingCooldownQuota.value = s.quota_cooldown_hours || 24;
  }

  // --- Key Extraction Helper ---
  function parseKeysInput(rawText) {
    if (!rawText) return [];

    // Match all standard Gemini API keys
    const regexMatches = rawText.match(GEMINI_KEY_REGEX);
    if (regexMatches && regexMatches.length > 0) {
      return [...new Set(regexMatches.map((k) => k.trim()))];
    }

    // Fallback: split by lines, commas, or spaces
    const tokens = rawText
      .split(/[\r\n,;\s]+/)
      .map((t) => t.trim())
      .filter((t) => t.length > 10);

    return [...new Set(tokens)];
  }

  function updateDetectedCount() {
    const raw = el.bulkKeysInput.value;
    const detected = parseKeysInput(raw);
    const count = detected.length;

    el.detectedKeysCount.textContent = `${count} ${count === 1 ? 'key' : 'keys'} detected`;
    el.submitImportBtn.disabled = count === 0;
  }

  // --- Utility Functions ---

  function formatNum(n) {
    if (n === undefined || n === null) return '0';
    return Number(n).toLocaleString();
  }

  function formatStrategyName(strat) {
    switch (strat) {
      case 'round_robin': return 'Round Robin';
      case 'least_recently_used': return 'Least Recently Used';
      case 'lowest_failure_rate': return 'Lowest Failure Rate';
      case 'lowest_latency': return 'Lowest Latency';
      case 'weighted': return 'Weighted';
      default: return strat || 'Round Robin';
    }
  }

  function formatCooldown(expiryIso) {
    if (!expiryIso) return { text: 'None', isExpired: true };
    const expDate = new Date(expiryIso);
    const now = new Date();
    const diffMs = expDate.getTime() - now.getTime();

    if (diffMs <= 0) {
      return { text: 'Expired', isExpired: true };
    }

    const diffSec = Math.ceil(diffMs / 1000);
    if (diffSec < 60) {
      return { text: `${diffSec}s remaining`, isExpired: false };
    }
    const diffMin = Math.ceil(diffSec / 60);
    if (diffMin < 60) {
      return { text: `${diffMin}m remaining`, isExpired: false };
    }
    const diffHours = (diffSec / 3600).toFixed(1);
    return { text: `${diffHours}h remaining`, isExpired: false };
  }

  function escapeHtml(str) {
    if (!str) return '';
    return String(str)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#039;');
  }

  // --- Event Listeners Setup ---

  function setupEventListeners() {
    // Auth & Login
    el.authBtn.addEventListener('click', () => {
      if (state.token) {
        if (confirm('Do you want to log out of the admin session?')) {
          state.token = '';
          localStorage.removeItem(STORAGE_KEY);
          el.authBtnLabel.textContent = 'Login';
          if (state.sse.eventSource) state.sse.eventSource.close();
          updateSSEStatus(false, 'Logged out');
          showLoginModal();
        }
      } else {
        showLoginModal();
      }
    });

    el.loginForm.addEventListener('submit', async (e) => {
      e.preventDefault();
      const token = el.adminTokenInput.value.trim();
      if (!token) return;

      state.token = token;
      localStorage.setItem(STORAGE_KEY, token);
      el.authBtnLabel.textContent = 'Logout';
      closeModal('login-modal');
      el.loginErrorAlert.style.display = 'none';

      showToast('Authenticated successfully.', 'emerald');
      initSSE();
      await fetchStats();
      await fetchKeys();
      await fetchSettings();
      await fetchRotation();
    });

    el.toggleTokenVisibilityBtn.addEventListener('click', () => {
      const type = el.adminTokenInput.type === 'password' ? 'text' : 'password';
      el.adminTokenInput.type = type;
    });

    // Modals Close via Buttons
    document.querySelectorAll('[data-close-modal]').forEach((btn) => {
      btn.addEventListener('click', () => {
        const targetId = btn.getAttribute('data-close-modal');
        closeModal(targetId);
      });
    });

    // Close Modals with ESC or Backdrop Click
    window.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        document.querySelectorAll('.modal-overlay').forEach((modal) => {
          if (modal.id !== 'login-modal' || state.token) {
            modal.style.display = 'none';
          }
        });
      }
    });

    document.querySelectorAll('.modal-overlay').forEach((overlay) => {
      overlay.addEventListener('click', (e) => {
        if (e.target === overlay) {
          if (overlay.id !== 'login-modal' || state.token) {
            overlay.style.display = 'none';
          }
        }
      });
    });

    // Header buttons
    el.openSettingsBtn.addEventListener('click', () => {
      populateSettingsForm();
      el.settingsFeedback.style.display = 'none';
      openModal('settings-modal');
    });

    el.strategyBadge.addEventListener('click', () => {
      populateSettingsForm();
      openModal('settings-modal');
    });

    // Toolbar buttons
    el.openImportBtn.addEventListener('click', () => {
      el.bulkKeysInput.value = '';
      updateDetectedCount();
      el.importFeedback.style.display = 'none';
      openModal('import-modal');
    });

    el.openAddBtn.addEventListener('click', () => {
      el.addKeyForm.reset();
      el.addFeedback.style.display = 'none';
      openModal('add-modal');
    });

    el.testAllBtn.addEventListener('click', () => {
      testAllKeys();
    });

    el.refreshKeysBtn.addEventListener('click', async () => {
      showToast('Refreshing data...', 'info', 1500);
      await fetchStats();
      await fetchKeys();
      await fetchRotation();
    });

    el.cancelTestBtn.addEventListener('click', () => {
      el.testProgressSection.style.display = 'none';
      state.testProgress.running = false;
    });

    // Search & Filter
    el.searchInput.addEventListener('input', (e) => {
      state.filter.search = e.target.value.trim();
      el.clearSearchBtn.style.display = state.filter.search ? 'block' : 'none';
      renderKeysTable();
    });

    el.clearSearchBtn.addEventListener('click', () => {
      el.searchInput.value = '';
      state.filter.search = '';
      el.clearSearchBtn.style.display = 'none';
      renderKeysTable();
    });

    el.statusFilter.addEventListener('change', (e) => {
      state.filter.status = e.target.value;
      renderKeysTable();
    });

    // Sorting Headers
    document.querySelectorAll('.th-sortable').forEach((th) => {
      th.addEventListener('click', () => {
        const field = th.getAttribute('data-sort');
        if (state.sort.field === field) {
          state.sort.direction = state.sort.direction === 'asc' ? 'desc' : 'asc';
        } else {
          state.sort.field = field;
          state.sort.direction = 'asc';
        }
        renderKeysTable();
      });
    });

    // Table Actions (Event Delegation)
    el.keysTableBody.addEventListener('click', (e) => {
      const copyBtn = e.target.closest('[data-copy-key]');
      if (copyBtn) {
        const textToCopy = copyBtn.getAttribute('data-copy-key');
        if (navigator.clipboard && textToCopy) {
          navigator.clipboard.writeText(textToCopy).then(() => {
            copyBtn.classList.add('copied');
            showToast('API key copied to clipboard.', 'emerald', 1800);
            setTimeout(() => copyBtn.classList.remove('copied'), 2000);
          });
        }
        return;
      }

      const actionBtn = e.target.closest('[data-action]');
      if (!actionBtn) return;

      const action = actionBtn.getAttribute('data-action');
      const id = actionBtn.getAttribute('data-id');

      if (action === 'test') {
        testSingleKey(id);
      } else if (action === 'toggle') {
        const currentStatus = actionBtn.getAttribute('data-status');
        toggleKeyStatus(id, currentStatus);
      } else if (action === 'delete') {
        const alias = actionBtn.getAttribute('data-alias');
        deleteKey(id, alias);
      }
    });

    // Bulk Import Live Counter
    el.bulkKeysInput.addEventListener('input', updateDetectedCount);

    // Bulk Import File Drag & Drop
    const handleFile = (file) => {
      if (!file) return;
      const reader = new FileReader();
      reader.onload = (e) => {
        el.bulkKeysInput.value = e.target.result;
        updateDetectedCount();
      };
      reader.readAsText(file);
    };

    el.dropZone.addEventListener('dragover', (e) => {
      e.preventDefault();
      el.dropZone.classList.add('dragover');
    });

    el.dropZone.addEventListener('dragleave', () => {
      el.dropZone.classList.remove('dragover');
    });

    el.dropZone.addEventListener('drop', (e) => {
      e.preventDefault();
      el.dropZone.classList.remove('dragover');
      if (e.dataTransfer.files.length > 0) {
        handleFile(e.dataTransfer.files[0]);
      }
    });

    el.fileUploadInput.addEventListener('change', (e) => {
      if (e.target.files.length > 0) {
        handleFile(e.target.files[0]);
      }
    });

    el.importForm.addEventListener('submit', (e) => {
      e.preventDefault();
      const keys = parseKeysInput(el.bulkKeysInput.value);
      if (keys.length === 0) {
        showToast('No valid keys detected.', 'warning');
        return;
      }
      submitBulkImport(keys);
    });

    // Add Key Form
    el.addKeyForm.addEventListener('submit', (e) => {
      e.preventDefault();
      submitAddKey(el.newApiKey.value, el.newKeyAlias.value);
    });

    // Settings Form
    el.settingsForm.addEventListener('submit', (e) => {
      e.preventDefault();
      const payload = {
        strategy: el.settingStrategy.value,
        concurrency: parseInt(el.settingConcurrency.value, 10),
        request_timeout_seconds: parseInt(el.settingTimeout.value, 10),
        retry_budget: parseInt(el.settingRetryBudget.value, 10),
        rate_limit_cooldown_seconds: parseInt(el.settingCooldownRatelimit.value, 10),
        quota_cooldown_hours: parseInt(el.settingCooldownQuota.value, 10),
      };
      saveSettings(payload);
    });
  }

  // --- Initial Boot ---
  function init() {
    setupEventListeners();

    if (state.token) {
      el.authBtnLabel.textContent = 'Logout';
      initSSE();
      fetchStats();
      fetchKeys();
      fetchSettings();
      fetchRotation();
    } else {
      el.authBtnLabel.textContent = 'Login';
      showLoginModal();
    }

    // Interval to refresh cooldown countdowns every 5s
    setInterval(() => {
      if (state.keys.some((k) => k.cooldown_expires_at)) {
        renderKeysTable();
      }
    }, 5000);
  }

  // Run on DOM ready
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', init);
  } else {
    init();
  }
})();
