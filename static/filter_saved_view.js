(() => {
  const form = document.querySelector('.filter-form, .board-filter');
  if (!form) return;

  form.addEventListener('submit', () => sessionStorage.setItem('rizzy-filter-refresh', '1'));

  const viewFields = new Set([
    'filter_id', 'indexed_by', 'sorted_by', 'creation', 'closure', 'assignment_status',
    'board_ids[]', 'column_ids[]', 'tag_ids[]', 'assignee_ids[]', 'creator_ids[]',
    'closer_ids[]', 'card_ids[]', 'terms[]'
  ]);

  function viewPayload() {
    const source = new URLSearchParams(location.search);
    const fields = new URLSearchParams();
    if (source.has('filter_id')) {
      fields.set('filter_id', source.get('filter_id'));
      return fields;
    }
    for (const [key, value] of source) {
      if (!viewFields.has(key) || !value.trim() ||
          (key === 'indexed_by' && value === 'all') ||
          (key === 'sorted_by' && value === 'latest')) continue;
      fields.append(key, value);
    }
    return fields;
  }

  function applyToggle(html) {
    const stream = new DOMParser().parseFromString(html, 'text/html')
      .querySelector('turbo-stream[target="filter-settings-save-toggle"] template');
    const replacement = stream?.content.firstElementChild;
    if (!replacement) return;
    const current = document.getElementById('filter-settings-save-toggle');
    if (current) current.replaceWith(replacement);
    else form.after(replacement);
  }

  const csrf = document.querySelector('meta[name="csrf-token"]')?.content;
  const headers = {
    Accept: 'text/vnd.turbo-stream.html',
    'Content-Type': 'application/x-www-form-urlencoded',
    ...(csrf ? { 'X-CSRF-Token': csrf } : {})
  };

  async function refreshToggle() {
    const fields = viewPayload();
    if (![...fields].length) return;
    const response = await fetch(`/${document.body.dataset.account}/filters/settings_refresh`, {
      method: 'POST', headers, body: fields.toString()
    });
    if (response.ok) applyToggle(await response.text());
  }

  document.addEventListener('submit', async event => {
    const toggleForm = event.target.closest('#filter-settings-save-toggle form');
    if (!toggleForm) return;
    event.preventDefault();
    const deleting = toggleForm.querySelector('input[name="_method"]')?.value === 'delete';
    const response = await fetch(toggleForm.action, {
      method: deleting ? 'DELETE' : 'POST', headers,
      body: deleting ? '' : new URLSearchParams(new FormData(toggleForm)).toString()
    });
    if (response.ok) applyToggle(await response.text());
  });

  if (sessionStorage.getItem('rizzy-filter-refresh') === '1') {
    sessionStorage.removeItem('rizzy-filter-refresh');
    refreshToggle().catch(() => {});
  }
})();
