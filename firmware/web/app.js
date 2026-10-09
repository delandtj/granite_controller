// Granite controller setup page. Plain ES2020, no framework, no build step,
// no external resource: the board is on an isolated network and this file is
// served gzipped from the firmware image.
'use strict';

const $ = (id) => document.getElementById(id);
const show = (el, on) => { el.hidden = !on; };
const text = (v) => (v === null || v === undefined ? '-' : String(v));

let ident = {};         // last /id payload
let status = null;      // last /api/v1/status payload
let config = null;      // last /api/v1/config document
let poll = null;        // status poll timer
let confirmTimer = null;

// --- transport --------------------------------------------------------

async function api(method, path, body, opts) {
  const o = opts || {};
  const init = { method, headers: {}, credentials: 'same-origin' };
  if (body !== undefined && body !== null) {
    init.body = typeof body === 'string' ? body : JSON.stringify(body);
    init.headers['Content-Type'] = 'application/json';
  }
  const res = await fetch(path, init);
  const raw = await res.text();
  let data = null;
  if (raw) {
    try { data = JSON.parse(raw); } catch (e) { data = { text: raw }; }
  }
  if (res.status === 401 && !o.quiet) { toLogin('Session expired, log in again.'); }
  if (!res.ok && !o.quiet) {
    banner((data && data.error) || `${method} ${path} failed with ${res.status}`, 'err');
  }
  return { ok: res.ok, status: res.status, data, raw };
}

function banner(message, kind, action) {
  const el = $('banner');
  el.textContent = message;
  el.className = 'banner' + (kind ? ' ' + kind : '');
  if (action) {
    const b = document.createElement('button');
    b.textContent = action.label;
    b.onclick = action.run;
    el.appendChild(b);
  }
  show(el, true);
  if (kind === 'ok') { setTimeout(() => show(el, false), 4000); }
}

function clearBanner() { show($('banner'), false); }

// --- boot -------------------------------------------------------------

async function boot() {
  const r = await api('GET', '/id', null, { quiet: true });
  if (!r.ok) { banner('Cannot reach the controller.', 'err'); return; }
  ident = r.data;
  $('id-device').textContent = text(ident.device);
  $('id-mac').textContent = text(ident.mac);
  $('id-fw').textContent = 'fw ' + text(ident.fw);
  document.title = 'Granite ' + text(ident.device);
  if (!ident.password_set) { toSetup(); return; }
  const s = await api('GET', '/api/v1/session', null, { quiet: true });
  if (s.ok && s.data && s.data.authenticated) { toApp(); } else { toLogin(); }
}

function toSetup() {
  show($('setup'), true); show($('login'), false); show($('app'), false);
  show($('logout'), false);
  stopPoll();
}

function toLogin(message) {
  show($('setup'), false); show($('login'), true); show($('app'), false);
  show($('logout'), false);
  stopPoll();
  if (message) { banner(message, 'err'); }
  $('login-pw').focus();
}

async function toApp() {
  show($('setup'), false); show($('login'), false); show($('app'), true);
  show($('logout'), true);
  clearBanner();
  await loadConfig();
  await refreshStatus();
  startPoll();
}

function startPoll() {
  stopPoll();
  poll = setInterval(() => { if (!document.hidden) { refreshStatus(); } }, 3000);
}
function stopPoll() { if (poll) { clearInterval(poll); poll = null; } }

// --- first setup ------------------------------------------------------

$('setup-form').onsubmit = async (e) => {
  e.preventDefault();
  const pw = $('setup-pw').value, pw2 = $('setup-pw2').value;
  if (pw !== pw2) { banner('The two passwords differ.', 'err'); return; }
  const r = await api('POST', '/api/v1/security/password', { password: pw });
  if (!r.ok) { return; }
  clearBanner();
  if (r.data && r.data.recovery_token) {
    $('token-value').textContent = r.data.recovery_token;
    show($('token-card'), true);
    show($('setup-form'), false);
  } else {
    toLogin('Password set. Log in.');
  }
};

$('token-ack').onclick = () => {
  show($('token-card'), false);
  toLogin('Password set. Log in.');
};

$('login-form').onsubmit = async (e) => {
  e.preventDefault();
  const r = await api('POST', '/api/v1/session', { password: $('login-pw').value },
    { quiet: true });
  $('login-pw').value = '';
  if (r.ok) { toApp(); return; }
  banner((r.data && r.data.error) || 'Login failed.', 'err');
};

$('logout').onclick = async () => {
  await api('DELETE', '/api/v1/session', null, { quiet: true });
  toLogin('Logged out.');
};

// --- tabs -------------------------------------------------------------

for (const b of $('tabs').querySelectorAll('button')) {
  b.onclick = () => {
    for (const other of $('tabs').querySelectorAll('button')) {
      other.classList.toggle('active', other === b);
    }
    for (const tab of document.querySelectorAll('.tab')) {
      show(tab, tab.id === 'tab-' + b.dataset.tab);
    }
    if (b.dataset.tab === 'security') { loadTokens(); }
    if (b.dataset.tab === 'firmware') { loadFirmware(); }
    if (b.dataset.tab === 'maint') { loadLog(); }
  };
}

// --- status -----------------------------------------------------------

function row(table, label, value, cls) {
  const tr = document.createElement('tr');
  const th = document.createElement('th');
  th.textContent = label;
  const td = document.createElement('td');
  td.textContent = text(value);
  if (cls) { td.className = cls; }
  tr.append(th, td);
  table.appendChild(tr);
}

async function refreshStatus() {
  const r = await api('GET', '/api/v1/status', null, { quiet: true });
  if (r.status === 401) { toLogin('Session expired, log in again.'); return; }
  if (!r.ok) { return; }
  status = r.data;
  renderNodes();
  renderSensors();
  renderNet();
  renderMqtt();
  renderFwSummary();
  renderStaged();
}

const ACTIONS = [
  ['on', 'on'], ['off', 'off'], ['force_off', 'force off'],
  ['reset', 'reset'], ['cycle', 'cycle'],
];

function renderNodes() {
  const body = $('node-table').querySelector('tbody');
  body.textContent = '';
  for (const n of status.state.nodes) {
    const tr = document.createElement('tr');
    const cells = [n.node, n.name];
    for (const c of cells) {
      const td = document.createElement('td');
      td.textContent = text(c);
      tr.appendChild(td);
    }
    const st = document.createElement('td');
    st.textContent = n.busy_action ? n.state + ' (' + n.busy_action + ')' : n.state;
    st.className = 'state-' + n.state;
    tr.appendChild(st);
    const led = document.createElement('td');
    led.textContent = n.led === null || n.led === undefined ? '?' : (n.led ? 'lit' : 'dark');
    tr.appendChild(led);
    const last = document.createElement('td');
    last.textContent = n.last_action
      ? n.last_action.action + ' ' + (n.last_action.result || '')
      : '-';
    tr.appendChild(last);
    const act = document.createElement('td');
    for (const [action, label] of ACTIONS) {
      const b = document.createElement('button');
      b.className = 'small';
      b.textContent = label;
      b.onclick = () => nodeAction(n.node, action);
      act.appendChild(b);
    }
    tr.appendChild(act);
    body.appendChild(tr);
  }
}

async function nodeAction(node, action) {
  const r = await api('POST', `/api/v1/nodes/${node}/${action}`, {});
  if (r.ok) {
    banner(`node ${node}: ${action} -> ${(r.data && r.data.result) || 'ok'}`, 'ok');
    refreshStatus();
  }
}

$('on-all').onclick = async () => {
  const r = await api('POST', '/api/v1/nodes/all/on_all', {});
  if (r.ok) { banner('staggered power-on queued', 'ok'); refreshStatus(); }
};

function renderSensors() {
  const t = $('sensor-table').querySelector('tbody');
  t.textContent = '';
  const s = status.state;
  row(t, 'board', s.board_temp_c === null ? '-' : s.board_temp_c.toFixed(2) + ' C');
  row(t, 'VIN', s.vin_v === null ? '-' : s.vin_v.toFixed(2) + ' V');
  for (const p of s.probes) {
    row(t, 'probe ' + p.slot + (p.name ? ' ' + p.name : ''),
      p.temp_c === null ? 'missing' : p.temp_c.toFixed(2) + ' C');
  }
  if (s.dry_in) {
    s.dry_in.forEach((v, i) => row(t, 'dry in ' + (i + 1), v ? 'closed' : 'open'));
  }
  row(t, 'uptime', s.uptime_s + ' s');
  row(t, 'free heap', status.free_heap + ' B');
  row(t, 'boot reason', status.boot_reason);
}

function renderNet() {
  const t = $('net-table').querySelector('tbody');
  t.textContent = '';
  const n = status.net;
  row(t, 'link', n.link_up ? 'up' : 'down', n.link_up ? 'state-on' : 'state-unknown');
  row(t, 'mode', n.ip_mode + (n.dhcp_fallback ? ' + dhcp fallback' : ''));
  row(t, 'address', n.ip);
  row(t, 'netmask', n.netmask);
  row(t, 'gateway', n.gateway);
  row(t, 'dns', (n.dns || []).join(', '));
  row(t, 'hostname', n.hostname);
  row(t, 'clock', n.sntp_synced ? 'sntp' : 'build time');
  row(t, 'modbus', status.modbus && status.modbus.enabled
    ? 'on, port ' + status.modbus.port : 'off');
}

function renderMqtt() {
  const t = $('mqtt-table').querySelector('tbody');
  t.textContent = '';
  const m = status.mqtt;
  row(t, 'enabled', m.enabled ? 'yes' : 'no');
  row(t, 'connected', m.connected ? 'yes' : 'no',
    m.connected ? 'state-on' : 'state-off');
  row(t, 'broker', m.broker);
  if (m.last_error) { row(t, 'last error', m.last_error, 'state-unknown'); }
}

function renderFwSummary() {
  const t = $('fw-table').querySelector('tbody');
  t.textContent = '';
  const o = status.ota;
  row(t, 'running', o.running);
  row(t, 'state', o.state, o.pending_verify ? 'state-busy' : 'state-on');
  if (o.validate_left_s !== null && o.validate_left_s !== undefined) {
    row(t, 'validate in', o.validate_left_s + ' s');
  }
  row(t, 'version', ident.fw);
  row(t, 'key id', o.key_id);
  row(t, 'cert sha256', ident.cert_sha256);
}

function renderStaged() {
  if (!status.staged) {
    if (confirmTimer) { clearInterval(confirmTimer); confirmTimer = null; clearBanner(); }
    return;
  }
  const left = status.staged.seconds_left;
  banner(
    `A configuration change (${status.staged.sections.join(', ')}) reverts in ` +
    `${left} s unless you confirm it.`,
    '',
    { label: 'Confirm', run: confirmStaged });
}

async function confirmStaged() {
  const r = await api('POST', '/api/v1/config/confirm', {});
  if (r.ok) { banner('Configuration confirmed.', 'ok'); refreshStatus(); }
}

// --- configuration forms ---------------------------------------------

async function loadConfig() {
  const r = await api('GET', '/api/v1/config');
  if (!r.ok) { return; }
  config = r.data;
  fillNet();
  fillMqtt();
  fillNodes();
  fillRules();
  fillSecurity();
}

function val(id, v) {
  const el = $(id);
  if (el.type === 'checkbox') { el.checked = !!v; } else { el.value = v === null || v === undefined ? '' : v; }
}
function num(id) {
  const v = $(id).value.trim();
  return v === '' ? 0 : Number(v);
}
function list(id) {
  return $(id).value.split(',').map((s) => s.trim()).filter((s) => s.length);
}

function fillNet() {
  const n = config.net;
  for (const k of ['ip_mode', 'address', 'gateway', 'hostname', 'sntp',
    't_confirm_s', 't_deadman_s']) { val('net-' + k, n[k]); }
  val('net-dns', (n.dns || []).join(', '));
  val('net-vlan', n.vlan);
  val('net-mdns', n.mdns);
}

$('net-form').onsubmit = async (e) => {
  e.preventDefault();
  const vlan = $('net-vlan').value.trim();
  const section = {
    ip_mode: $('net-ip_mode').value,
    address: $('net-address').value.trim(),
    gateway: $('net-gateway').value.trim(),
    dns: list('net-dns'),
    hostname: $('net-hostname').value.trim(),
    mdns: $('net-mdns').checked,
    vlan: vlan === '' ? null : Number(vlan),
    sntp: $('net-sntp').value.trim(),
    t_confirm_s: num('net-t_confirm_s'),
    t_deadman_s: num('net-t_deadman_s'),
  };
  const r = await api('PUT', '/api/v1/config/net', section);
  if (r.ok) {
    banner(`Staged. Reconnect at the new address and confirm within ` +
      `${r.data.confirm_s} s or it reverts.`, '', { label: 'Confirm', run: confirmStaged });
    loadConfig();
  }
};

function fillMqtt() {
  const m = config.mqtt;
  for (const k of ['enabled', 'host', 'port', 'tls', 'username', 'client_id', 'site',
    'topic_root', 'qos', 'keepalive_s', 't_state_s', 'auto_confirm_on_connect',
    'skip_time_check', 'ca_pem']) { val('mqtt-' + k, m[k]); }
  const base = `${m.topic_root}/${m.site}/${ident.device}`;
  $('mqtt-topics').textContent =
    `Topics: ${base}/{status,state,event,log}, commands on ${base}/cmd, ` +
    `acks on ${base}/ack/<id>.`;
}

$('mqtt-form').onsubmit = async (e) => {
  e.preventDefault();
  const section = {
    enabled: $('mqtt-enabled').checked,
    host: $('mqtt-host').value.trim(),
    port: num('mqtt-port'),
    tls: $('mqtt-tls').checked,
    ca_pem: $('mqtt-ca_pem').value,
    username: $('mqtt-username').value.trim(),
    client_id: $('mqtt-client_id').value.trim(),
    site: $('mqtt-site').value.trim(),
    topic_root: $('mqtt-topic_root').value.trim(),
    qos: num('mqtt-qos'),
    keepalive_s: num('mqtt-keepalive_s'),
    t_state_s: num('mqtt-t_state_s'),
    auto_confirm_on_connect: $('mqtt-auto_confirm_on_connect').checked,
    skip_time_check: $('mqtt-skip_time_check').checked,
  };
  const r = await api('PUT', '/api/v1/config/mqtt', section);
  if (r.ok) { banner('Broker settings staged. Confirm once it connects.', '',
    { label: 'Confirm', run: confirmStaged }); loadConfig(); }
};

const POLICIES = ['leave', 'on', 'off'];
const SENSE = ['enabled', 'ignore'];

function select(options, value) {
  const s = document.createElement('select');
  for (const o of options) {
    const opt = document.createElement('option');
    opt.value = o; opt.textContent = o; opt.selected = o === value;
    s.appendChild(opt);
  }
  return s;
}

function fillNodes() {
  const body = $('nodecfg-table').querySelector('tbody');
  body.textContent = '';
  config.nodes.nodes.forEach((n, i) => {
    const tr = document.createElement('tr');
    const id = document.createElement('td'); id.textContent = i + 1;
    const name = document.createElement('td');
    const input = document.createElement('input'); input.value = n.name;
    input.dataset.node = i; input.className = 'node-name';
    name.appendChild(input);
    const pol = document.createElement('td');
    const polSel = select(POLICIES, n.boot_policy);
    polSel.className = 'node-policy'; pol.appendChild(polSel);
    const sen = document.createElement('td');
    const senSel = select(SENSE, n.sense);
    senSel.className = 'node-sense'; sen.appendChild(senSel);
    tr.append(id, name, pol, sen);
    body.appendChild(tr);
  });
  val('nodes-order', config.nodes.order.join(', '));
  val('nodes-t_probe_s', config.nodes.t_probe_s);
  val('nodes-vin_trim', config.nodes.vin_trim);
  const t = $('timings');
  t.textContent = '';
  for (const [k, v] of Object.entries(config.nodes.timings)) {
    const label = document.createElement('label');
    label.textContent = k;
    const input = document.createElement('input');
    input.type = 'number'; input.value = v; input.dataset.timing = k;
    label.appendChild(input);
    t.appendChild(label);
  }
  renderProbes();
}

function renderProbes() {
  const body = $('probe-table').querySelector('tbody');
  body.textContent = '';
  const live = status ? status.state.probes : [];
  const rows = config.nodes.probes.length ? config.nodes.probes : live;
  rows.forEach((p, i) => {
    const tr = document.createElement('tr');
    const slot = document.createElement('td'); slot.textContent = i + 1;
    const rom = document.createElement('td');
    const romIn = document.createElement('input');
    romIn.value = p.rom || ''; romIn.className = 'probe-rom';
    rom.appendChild(romIn);
    const name = document.createElement('td');
    const nameIn = document.createElement('input');
    nameIn.value = p.name || ''; nameIn.className = 'probe-name';
    name.appendChild(nameIn);
    const reading = document.createElement('td');
    const l = live[i];
    reading.textContent = l && l.temp_c !== null && l.temp_c !== undefined
      ? l.temp_c.toFixed(2) + ' C' : '-';
    tr.append(slot, rom, name, reading);
    body.appendChild(tr);
  });
}

$('probe-scan').onclick = async () => {
  const r = await api('POST', '/api/v1/probes/scan', {});
  if (r.ok) { banner('Scan queued.', 'ok'); setTimeout(refreshStatus, 2000); }
};

$('nodes-save').onclick = async () => {
  const names = [...document.querySelectorAll('.node-name')].map((i) => i.value);
  const policies = [...document.querySelectorAll('.node-policy')].map((i) => i.value);
  const senses = [...document.querySelectorAll('.node-sense')].map((i) => i.value);
  const timings = {};
  for (const i of document.querySelectorAll('[data-timing]')) {
    timings[i.dataset.timing] = Number(i.value);
  }
  const roms = [...document.querySelectorAll('.probe-rom')].map((i) => i.value.trim());
  const pnames = [...document.querySelectorAll('.probe-name')].map((i) => i.value.trim());
  const section = {
    nodes: names.map((name, i) => ({
      name, boot_policy: policies[i], sense: senses[i],
    })),
    order: list('nodes-order').map(Number),
    timings,
    probes: roms.map((rom, i) => ({ rom, name: pnames[i] })).filter((p) => p.rom),
    t_probe_s: num('nodes-t_probe_s'),
    vin_trim: Number($('nodes-vin_trim').value),
  };
  const r = await api('PUT', '/api/v1/config/nodes', section);
  if (r.ok) { banner('Nodes saved.', 'ok'); loadConfig(); }
};

// --- rules ------------------------------------------------------------

const SOURCES = ['probe', 'probe_max', 'board_temp', 'vin', 'dry_in', 'node_on',
  'mqtt_connected', 'link_up', 'uptime_s'];
const OPS = ['>', '<', '==', 'changed'];
const RULE_ACTIONS = ['event', 'on', 'off', 'force_off', 'reset', 'cycle', 'on_all'];
const REARM = ['auto', 'manual'];

function ruleRow(rule) {
  const tr = document.createElement('tr');
  const cell = (el) => { const td = document.createElement('td'); td.appendChild(el); tr.appendChild(td); return el; };
  const enabled = document.createElement('input');
  enabled.type = 'checkbox'; enabled.checked = rule.enabled; enabled.className = 'r-enabled';
  cell(enabled);
  const id = document.createElement('input');
  id.type = 'number'; id.min = 1; id.max = 255; id.value = rule.id; id.className = 'r-id';
  cell(id);
  const name = document.createElement('input');
  name.value = rule.name || ''; name.className = 'r-name';
  cell(name);
  const src = select(SOURCES, rule.source); src.className = 'r-source'; cell(src);
  const n = document.createElement('input');
  n.type = 'number'; n.min = 1; n.max = 8; n.value = rule.n || ''; n.className = 'r-n';
  cell(n);
  const op = select(OPS, rule.op); op.className = 'r-op'; cell(op);
  for (const [k, cls] of [['threshold', 'r-threshold'], ['hysteresis', 'r-hysteresis'],
    ['hold_s', 'r-hold']]) {
    const i = document.createElement('input');
    i.type = 'number'; i.value = rule[k] || 0; i.className = cls;
    cell(i);
  }
  const action = select(RULE_ACTIONS,
    rule.action === 'act' ? rule.kind : rule.action);
  action.className = 'r-action'; cell(action);
  const target = document.createElement('input');
  target.value = rule.target === null || rule.target === undefined ? 'all' : rule.target;
  target.className = 'r-target';
  cell(target);
  const rearm = select(REARM, rule.rearm); rearm.className = 'r-rearm'; cell(rearm);
  const tools = document.createElement('td');
  const ack = document.createElement('button');
  ack.className = 'small'; ack.textContent = 'ack';
  ack.onclick = async () => {
    const r = await api('POST', `/api/v1/rules/${rule.id}/ack`, {});
    if (r.ok) { banner('Rule acknowledged.', 'ok'); }
  };
  const del = document.createElement('button');
  del.className = 'small danger'; del.textContent = 'x';
  del.onclick = () => tr.remove();
  tools.append(ack, del);
  tr.appendChild(tools);
  return tr;
}

function fillRules() {
  const body = $('rule-table').querySelector('tbody');
  body.textContent = '';
  for (const rule of config.rules.rules) { body.appendChild(ruleRow(rule)); }
}

$('rule-add').onclick = () => {
  const used = [...document.querySelectorAll('.r-id')].map((i) => Number(i.value));
  let id = 1;
  while (used.includes(id)) { id += 1; }
  $('rule-table').querySelector('tbody').appendChild(ruleRow({
    id, enabled: false, name: '', source: 'probe_max', op: '>', threshold: 7000,
    hysteresis: 500, hold_s: 30, action: 'event', target: 'all', rearm: 'auto',
  }));
};

$('rules-save').onclick = async () => {
  const rules = [...$('rule-table').querySelectorAll('tbody tr')].map((tr) => {
    const g = (cls) => tr.querySelector('.' + cls);
    const source = g('r-source').value;
    const action = g('r-action').value;
    const rule = {
      id: Number(g('r-id').value),
      enabled: g('r-enabled').checked,
      name: g('r-name').value,
      source,
      op: g('r-op').value,
      threshold: Number(g('r-threshold').value),
      hysteresis: Number(g('r-hysteresis').value),
      hold_s: Number(g('r-hold').value),
      target: g('r-target').value.trim() || 'all',
      rearm: g('r-rearm').value,
    };
    if (source === 'probe' || source === 'dry_in' || source === 'node_on') {
      rule.n = Number(g('r-n').value || 1);
    }
    if (action === 'event') { rule.action = 'event'; }
    else { rule.action = 'act'; rule.kind = action; }
    return rule;
  });
  const r = await api('PUT', '/api/v1/rules', rules);
  if (r.ok) { banner('Rules saved.', 'ok'); loadConfig(); }
};

// --- security ---------------------------------------------------------

function fillSecurity() {
  $('cert-sha').textContent = text(ident.cert_sha256);
  val('fleet-pem', config.sec.fleet_recovery_pubkey_pem);
  const m = config.sec.modbus;
  val('modbus-enabled', m.enabled);
  val('modbus-port', m.port);
  val('modbus-unit_id', m.unit_id);
  val('modbus-max_conn', m.max_conn);
  val('modbus-allow', (m.allow || []).join(', '));
}

$('pw-form').onsubmit = async (e) => {
  e.preventDefault();
  const body = { password: $('pw-new').value };
  if ($('pw-current').value) { body.current = $('pw-current').value; }
  const r = await api('POST', '/api/v1/security/password', body);
  $('pw-current').value = ''; $('pw-new').value = '';
  if (r.ok) { toLogin('Password changed. Log in again.'); }
};

async function loadTokens() {
  const r = await api('GET', '/api/v1/security/tokens');
  if (!r.ok) { return; }
  const body = $('token-table').querySelector('tbody');
  body.textContent = '';
  for (const t of r.data.tokens) {
    const tr = document.createElement('tr');
    const name = document.createElement('td'); name.textContent = t.name;
    const created = document.createElement('td'); created.textContent = t.created_s;
    const tools = document.createElement('td');
    const del = document.createElement('button');
    del.className = 'small danger'; del.textContent = 'revoke';
    del.onclick = async () => {
      const d = await api('DELETE', '/api/v1/security/tokens/' + encodeURIComponent(t.name));
      if (d.ok) { loadTokens(); }
    };
    tools.appendChild(del);
    tr.append(name, created, tools);
    body.appendChild(tr);
  }
}

$('token-form').onsubmit = async (e) => {
  e.preventDefault();
  const r = await api('POST', '/api/v1/security/tokens', { name: $('token-name').value });
  if (!r.ok) { return; }
  $('token-new').textContent =
    'Authorization: Bearer ' + r.data.token + '\n(shown once)';
  show($('token-new'), true);
  $('token-name').value = '';
  loadTokens();
};

$('cert-form').onsubmit = async (e) => {
  e.preventDefault();
  const r = await api('PUT', '/api/v1/security/cert', {
    cert_pem: $('cert-pem').value, key_pem: $('cert-key').value,
  });
  if (r.ok) {
    banner('Certificate replaced; reconnect over HTTPS. Fingerprint ' +
      r.data.cert_sha256, 'ok');
  }
};

$('fleet-form').onsubmit = async (e) => {
  e.preventDefault();
  const r = await api('PUT', '/api/v1/security/fleet-key', { pem: $('fleet-pem').value });
  if (r.ok) { banner('Fleet recovery key set.', 'ok'); }
};

$('modbus-form').onsubmit = async (e) => {
  e.preventDefault();
  const r = await api('PUT', '/api/v1/security/modbus-allowlist', {
    enabled: $('modbus-enabled').checked,
    port: num('modbus-port'),
    unit_id: num('modbus-unit_id'),
    max_conn: num('modbus-max_conn'),
    allow: list('modbus-allow'),
  });
  if (r.ok) { banner('Modbus settings saved.', 'ok'); loadConfig(); }
};

// --- firmware ---------------------------------------------------------

async function loadFirmware() {
  const r = await api('GET', '/api/v1/firmware');
  if (!r.ok) { return; }
  const fw = r.data;
  const body = $('slot-table').querySelector('tbody');
  body.textContent = '';
  for (const s of fw.slots) {
    const tr = document.createElement('tr');
    for (const v of [s.label, s.state, s.version, (s.size / 1024).toFixed(0) + ' KiB']) {
      const td = document.createElement('td'); td.textContent = text(v); tr.appendChild(td);
    }
    body.appendChild(tr);
  }
  $('fw-key').textContent = text(fw.key_id);
  show($('fw-pending'), !!fw.pending_verify);
  $('fw-rollback').disabled = !fw.rollback_available;
  $('fw-mark-valid').disabled = !fw.pending_verify;
}

$('upload-form').onsubmit = (e) => {
  e.preventDefault();
  const file = $('upload-file').files[0];
  if (!file) { banner('Pick an image first.', 'err'); return; }
  const bar = $('upload-progress');
  bar.value = 0; show(bar, true);
  const xhr = new XMLHttpRequest();
  xhr.open('POST', '/api/v1/firmware/upload');
  xhr.setRequestHeader('Content-Type', 'application/octet-stream');
  xhr.upload.onprogress = (ev) => {
    if (ev.lengthComputable) { bar.value = Math.round((ev.loaded / ev.total) * 100); }
  };
  xhr.onload = () => {
    show(bar, false);
    let data = null;
    try { data = JSON.parse(xhr.responseText); } catch (err) { data = null; }
    if (xhr.status >= 200 && xhr.status < 300) {
      banner('Image accepted into ' + ((data && data.slot) || 'the spare slot') +
        '. Reboot to run it, then mark it valid.', 'ok');
      loadFirmware();
    } else {
      banner((data && data.error) || 'Upload failed with ' + xhr.status, 'err');
    }
  };
  xhr.onerror = () => { show(bar, false); banner('Upload failed.', 'err'); };
  xhr.send(file);
};

$('fw-rollback').onclick = async () => {
  if (!window.confirm('Boot the previous image?')) { return; }
  const r = await api('POST', '/api/v1/firmware/rollback', {});
  if (r.ok) { banner('Rollback scheduled.', 'ok'); loadFirmware(); }
};

$('fw-mark-valid').onclick = async () => {
  const r = await api('POST', '/api/v1/firmware/mark-valid', {});
  if (r.ok) { banner('Running image marked valid.', 'ok'); loadFirmware(); }
};

// --- maintenance ------------------------------------------------------

$('cfg-export').onclick = async () => {
  const r = await api('GET', '/api/v1/config/export');
  if (!r.ok) { return; }
  const blob = new Blob([r.raw], { type: 'application/json' });
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = (ident.device || 'granite') + '-config.json';
  a.click();
  URL.revokeObjectURL(a.href);
};

$('cfg-import').onchange = async (e) => {
  const file = e.target.files[0];
  if (!file) { return; }
  const body = await file.text();
  const r = await api('POST', '/api/v1/config/import', body);
  e.target.value = '';
  if (r.ok) {
    const staged = (r.data.staged || []).join(', ');
    banner('Imported.' + (staged ? ' Staged: ' + staged + '. Confirm to keep it.' : ''),
      staged ? '' : 'ok',
      staged ? { label: 'Confirm', run: confirmStaged } : null);
    loadConfig();
  }
};

async function loadLog() {
  const lines = Number($('log-lines').value) || 100;
  const r = await api('GET', '/api/v1/log/tail?lines=' + lines);
  if (!r.ok) { return; }
  $('log-view').textContent = r.raw || '(empty)';
}

$('log-refresh').onclick = loadLog;

$('reboot').onclick = async () => {
  if (!window.confirm('Reboot the controller? Node power is not affected.')) { return; }
  const r = await api('POST', '/api/v1/reboot', {});
  if (r.ok) { banner('Rebooting. This page reconnects by itself.', 'ok'); }
};

$('reset-form').onsubmit = async (e) => {
  e.preventDefault();
  const confirmText = $('reset-confirm').value.trim();
  if (confirmText !== ident.device) {
    banner('Type the device id exactly: ' + ident.device, 'err');
    return;
  }
  if (!window.confirm('Erase the configuration and the admin password?')) { return; }
  const r = await api('POST', '/api/v1/factory-reset', { confirm: confirmText });
  if (r.ok) { banner('Factory reset. The controller reboots into first setup.', 'ok'); }
};

boot();
