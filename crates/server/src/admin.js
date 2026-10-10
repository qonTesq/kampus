'use strict';
let state;
const byId = id => document.getElementById(id);
async function load() {
  const response = await fetch('/state', { cache: 'no-store' });
  if (!response.ok) throw new Error('Cannot load policy or audit log.');
  state = await response.json();
  byId('version').textContent = state.version;
  byId('sensitive').checked = state.sensitive_enabled;
  byId('personal').checked = state.allow_self_registered_sensitive;
  byId('clients').replaceChildren();
  for (const client of state.clients) {
    const label = document.createElement('label');
    const input = document.createElement('input');
    input.type = 'checkbox'; input.checked = client.approved;
    input.dataset.clientId = client.client_id;
    label.append(input, document.createTextNode(`${client.workspace} (${client.client_id})`));
    byId('clients').append(label);
  }
  byId('controls').disabled = !state.writable;
  byId('audit').textContent = state.audit.map(row => JSON.stringify(row)).join('\n') || 'No calls recorded.';
}
byId('switches').addEventListener('submit', async event => {
  event.preventDefault();
  const approvals = Object.fromEntries([...byId('clients').querySelectorAll('input')].map(input => [input.dataset.clientId, input.checked]));
  byId('controls').disabled = true;
  try {
    const response = await fetch('/state', { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Kampus-CSRF': state.csrf }, body: JSON.stringify({ version: state.version, sensitive_enabled: byId('sensitive').checked, allow_self_registered_sensitive: byId('personal').checked, approvals }) });
    const result = await response.json();
    if (!response.ok) throw new Error(result.error || 'Save failed. Reload the page before retrying.');
    await load();
    byId('status').textContent = `Saved policy version ${result.version}. New calls use this policy.`;
  } catch (error) {
    byId('status').textContent = error.message;
    byId('controls').disabled = !state.writable;
  }
});
load().then(() => { byId('status').textContent = state.writable ? 'Ready. Tab to a switch, press Space to toggle, then Save policy.' : 'Read-only. Start with a policy file to enable saving.'; }).catch(error => { byId('status').textContent = error.message; });
