const activity = document.getElementById('activity');
document.addEventListener('click', event => {
  const link = event.target.closest('a[href]');
  if (!link) return;
  const destination = new URL(link.href);
  if (destination.origin !== location.origin || !new RegExp(`^/${document.body.dataset.account}/cards/\\d+$`).test(destination.pathname)) return;
  sessionStorage.setItem('rizzy-card-referrer', JSON.stringify({ url: location.href, label: document.title.replace(/ \| Rizzy$/, '') }));
});
const eventTime = new Intl.DateTimeFormat('en-US', { timeStyle: 'short' });
const eventTimeTitle = new Intl.DateTimeFormat('en-US', { timeStyle: 'short', dateStyle: 'short' });
function localizeEventTimes(root) {
  for (const element of root.querySelectorAll('.event__timestamp[datetime]')) {
    const date = new Date(element.getAttribute('datetime'));
    if (Number.isNaN(date.getTime())) continue;
    element.textContent = eventTime.format(date);
    element.title = eventTimeTitle.format(date);
  }
}
if (activity) localizeEventTimes(activity);
if (activity && 'IntersectionObserver' in window) {
  const observer = new IntersectionObserver(async entries => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      const link = entry.target;
      observer.unobserve(link);
      try {
        const response = await fetch(link.href, { headers: { Accept: 'text/html' } });
        if (!response.ok) throw new Error('Could not load activity');
        const fragment = document.createElement('div');
        fragment.innerHTML = await response.text();
        const frame = fragment.querySelector(`turbo-frame[id="${link.dataset.frame}"]`);
        if (!frame) throw new Error('Activity page was unavailable');
        link.closest('turbo-frame').after(frame);
        localizeEventTimes(frame);
        link.remove();
        const next = frame.querySelector('.day-timeline-pagination-link');
        if (next) observer.observe(next);
      } catch { link.textContent = 'Load more…'; }
    }
  }, { rootMargin: '350px' });
  const first = activity.querySelector('.day-timeline-pagination-link');
  if (first) observer.observe(first);
}

const timelineFilters = document.querySelector('.events__filters');
timelineFilters?.addEventListener('change', event => {
  if (event.target.matches('input[type="checkbox"]')) timelineFilters.requestSubmit();
});
timelineFilters?.addEventListener('input', event => {
  if (!event.target.matches('.quick-filter__search')) return;
  const query = event.target.value.trim().toLocaleLowerCase();
  for (const option of event.target.closest('.quick-filter').querySelectorAll('.quick-filter__option')) {
    option.hidden = !option.textContent.toLocaleLowerCase().includes(query);
  }
});
timelineFilters?.addEventListener('toggle', event => {
  if (event.target.open) event.target.querySelector('.quick-filter__search')?.focus();
}, true);
timelineFilters?.addEventListener('keydown', event => {
  if (event.key === 'Escape') event.target.closest('.quick-filter')?.removeAttribute('open');
});

document.addEventListener('keydown', event => {
  if (event.repeat || event.altKey || event.ctrlKey || event.metaKey ||
      event.target.closest('input, textarea, select, [contenteditable="true"]')) return;
  if (event.key.toLowerCase() === 'c') {
    const form = document.getElementById('add-card');
    if (form) { event.preventDefault(); form.requestSubmit(); }
  } else if (event.key.toLowerCase() === 'b') {
    const link = document.getElementById('add-board');
    if (link) { event.preventDefault(); link.click(); }
  }
});

const menuTrigger = document.querySelector('[data-open-menu]');
const menuDialog = document.getElementById('menu-dialog');
if (menuTrigger && menuDialog) {
  const menuFrame = menuDialog.querySelector('#my_menu');
  let menuLoad = 0;
  function closeMenu() {
    if (menuDialog.open) menuDialog.close();
    menuTrigger.setAttribute('aria-expanded', 'false');
    menuTrigger.focus();
  }
  async function openMenu() {
    for (const other of document.querySelectorAll('dialog[open]')) if (other !== menuDialog) other.close();
    document.querySelector('.search__reset')?.click();
    menuDialog.show();
    menuTrigger.setAttribute('aria-expanded', 'true');
    const loadId = ++menuLoad;
    menuFrame.textContent = 'Loading menu…';
    try {
      const response = await fetch(menuTrigger.href, { headers: { Accept: 'text/html' } });
      if (!response.ok) throw new Error('Could not load menu');
      const page = new DOMParser().parseFromString(await response.text(), 'text/html');
      const loaded = page.getElementById('my_menu');
      if (!loaded) throw new Error('Menu frame is unavailable');
      if (loadId !== menuLoad || !menuDialog.open) return;
      menuFrame.replaceChildren(...loaded.cloneNode(true).childNodes);
      menuDialog.querySelector('#menu-search')?.focus();
    } catch {
      if (loadId === menuLoad) menuFrame.innerHTML = `<a href="${menuTrigger.href}">Open menu</a>`;
    }
  }
  menuTrigger.addEventListener('click', event => {
    event.preventDefault();
    if (menuDialog.open) closeMenu(); else openMenu();
  });
  menuDialog.addEventListener('close', () => {
    if (!menuDialog.open) menuTrigger.setAttribute('aria-expanded', 'false');
  });
  menuDialog.addEventListener('click', event => {
    if (event.target.closest('[data-menu-close]')) closeMenu();
  });
  menuDialog.addEventListener('input', event => {
    if (event.target.id !== 'menu-search') return;
    const query = event.target.value.trim().toLocaleLowerCase();
    for (const item of menuFrame.querySelectorAll('[data-filter-target="item"]')) {
      item.hidden = !item.textContent.toLocaleLowerCase().includes(query);
    }
    for (const section of menuFrame.querySelectorAll('.nav__section')) {
      section.hidden = !!query && !section.querySelector('[data-filter-target="item"]:not([hidden])');
      if (section.classList.contains('nav__section--secret')) {
        if (query) section.setAttribute('data-is-filtering', ''); else section.removeAttribute('data-is-filtering');
      }
    }
  });
  menuDialog.addEventListener('submit', async event => {
    const form = event.target.closest('form[action="/session"]');
    if (!form) return;
    event.preventDefault();
    const response = await fetch('/session', { method: 'DELETE', headers: { Accept: 'application/json' } });
    if (response.ok) window.location.assign('/session/new');
  });
  menuDialog.addEventListener('keydown', event => {
    if (event.key === 'Escape') { event.preventDefault(); closeMenu(); return; }
    const items = [...menuFrame.querySelectorAll('.popup__item:not([hidden]) a, .popup__item:not([hidden]) button')]
      .filter(item => item.getClientRects().length);
    if (event.key === 'Enter' && event.target.id === 'menu-search' && items.length) {
      event.preventDefault(); items[0].click(); return;
    }
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    event.preventDefault();
    const current = items.indexOf(document.activeElement);
    const direction = event.key === 'ArrowDown' ? 1 : -1;
    items[(current + direction + items.length) % items.length]?.focus();
  });
  document.addEventListener('click', event => {
    if (menuDialog.open && !menuDialog.contains(event.target) && !menuTrigger.contains(event.target)) closeMenu();
  });
  document.addEventListener('keydown', event => {
    if (event.repeat || event.altKey || event.target.closest('input, textarea, select, [contenteditable="true"]')) return;
    if (event.key.toLowerCase() !== 'j') return;
    event.preventDefault();
    if (menuDialog.open) closeMenu(); else openMenu();
  });
}

const footer = document.getElementById('footer');
if (footer) {
  const account = document.body.dataset.account;
  const trays = {
    pins: document.getElementById('pin-tray'),
    notifications: document.getElementById('notification-tray')
  };
  const toggles = Object.fromEntries(Object.keys(trays).map(kind => [kind, footer.querySelector(`[data-open-tray="${kind}"]`)]));
  const searchButton = footer.querySelector('[data-open-search]');
  const searchPlaceholder = footer.querySelector('.bar__placeholder');
  const searchInputBar = footer.querySelector('.bar__input');
  const searchInput = document.getElementById('bar-search');
  const searchDialog = document.getElementById('bar-modal');
  const searchFrame = document.getElementById('bar_content');
  let searchTimer;
  const trayLoads = { pins: 0, notifications: 0 };

  function closeTray(kind) {
    trays[kind].close();
    toggles[kind].setAttribute('aria-expanded', 'false');
  }
  function resetSearch() {
    clearTimeout(searchTimer);
    if (searchDialog.open) searchDialog.close();
    searchInputBar.hidden = true;
    searchPlaceholder.hidden = false;
    searchFrame.replaceChildren();
  }
  async function loadTray(kind) {
    const loadId = ++trayLoads[kind];
    const frame = trays[kind].querySelector('turbo-frame');
    frame.textContent = 'Loading…';
    const path = kind === 'pins' ? `/${account}/my/pins` : `/${account}/notifications/tray`;
    try {
      const response = await fetch(path, { headers: { Accept: 'text/html' } });
      if (!response.ok) throw new Error('Could not load');
      const page = new DOMParser().parseFromString(await response.text(), 'text/html');
      const loaded = page.getElementById(frame.id);
      if (!loaded) throw new Error('Tray content was unavailable');
      if (loadId !== trayLoads[kind]) return;
      frame.replaceChildren(...loaded.cloneNode(true).childNodes);
      if (kind === 'pins' && !frame.querySelector('.tray__item')) frame.textContent = 'No pinned cards yet';
      if (kind === 'notifications' && !frame.querySelector('.tray__item')) frame.textContent = 'Nothing new for you';
      if (kind === 'notifications') toggles.notifications.classList.toggle('unread', !!frame.querySelector('.unread'));
    } catch { if (loadId === trayLoads[kind]) frame.textContent = 'Could not load items.'; }
  }
  function toggleTray(kind) {
    if (trays[kind].open) { closeTray(kind); return; }
    for (const other of Object.keys(trays)) if (trays[other].open) closeTray(other);
    resetSearch();
    trays[kind].show();
    toggles[kind].setAttribute('aria-expanded', 'true');
    loadTray(kind);
  }
  for (const kind of Object.keys(trays)) {
    toggles[kind].addEventListener('click', () => toggleTray(kind));
    trays[kind].addEventListener('close', () => toggles[kind].setAttribute('aria-expanded', 'false'));
  }
  document.addEventListener('click', event => {
    for (const kind of Object.keys(trays)) {
      if (trays[kind].open && !trays[kind].contains(event.target) && !toggles[kind].contains(event.target)) closeTray(kind);
    }
  });
  trays.pins.addEventListener('submit', async event => {
    const form = event.target.closest('.tray__item--pin form');
    if (!form) return;
    event.preventDefault();
    const response = await fetch(form.action, { method: 'DELETE', headers: { Accept: 'application/json' } });
    if (response.ok) form.closest('.tray__item--pin').remove();
    else loadTray('pins');
  });
  trays.notifications.addEventListener('submit', async event => {
    const form = event.target.closest('form');
    if (!form) return;
    event.preventDefault();
    const read = form.querySelector('input[name="_method"]')?.value !== 'delete';
    const method = form.hasAttribute('data-notification-toggle') ? (read ? 'POST' : 'DELETE') : 'POST';
    const response = await fetch(form.action, { method, headers: { Accept: 'application/json' } });
    if (response.ok) {
      if (!form.hasAttribute('data-notification-toggle')) closeTray('notifications');
      loadTray('notifications');
    }
  });
  async function showSearchResults() {
    const query = searchInput.value.trim();
    if (!query) { if (searchDialog.open) searchDialog.close(); searchFrame.replaceChildren(); return; }
    searchFrame.textContent = 'Searching…';
    if (!searchDialog.open) searchDialog.show();
    try {
      const response = await fetch(`/${account}/search?q=${encodeURIComponent(query)}`, { headers: { Accept: 'text/html' } });
      if (!response.ok) throw new Error('Could not search');
      const page = new DOMParser().parseFromString(await response.text(), 'text/html');
      const loaded = page.getElementById('bar_content');
      if (!loaded) throw new Error('Search results were unavailable');
      if (searchInput.value.trim() !== query) return;
      const cardRedirect = loaded.querySelector('form[data-controller="auto-submit"]');
      if (cardRedirect) { window.location.assign(new URL(cardRedirect.getAttribute('action'), window.location.origin)); return; }
      searchFrame.replaceChildren(...loaded.cloneNode(true).childNodes);
    } catch { if (searchInput.value.trim() === query) searchFrame.textContent = 'Could not search.'; }
  }
  searchButton.addEventListener('click', () => {
    for (const kind of Object.keys(trays)) if (trays[kind].open) closeTray(kind);
    searchPlaceholder.hidden = true;
    searchInputBar.hidden = false;
    searchInput.focus();
  });
  footer.querySelector('.search__reset').addEventListener('click', resetSearch);
  footer.querySelector('.search__form').addEventListener('submit', event => { event.preventDefault(); clearTimeout(searchTimer); showSearchResults(); });
  searchInput.addEventListener('input', () => { clearTimeout(searchTimer); searchTimer = setTimeout(showSearchResults, 300); });
  document.addEventListener('keydown', event => {
    if (event.key === 'Escape') {
      if (!searchInputBar.hidden) resetSearch();
      for (const kind of Object.keys(trays)) if (trays[kind].open) closeTray(kind);
      return;
    }
    if (event.repeat || event.altKey || event.ctrlKey || event.metaKey || event.target.closest('input, textarea, select, [contenteditable="true"]')) return;
    const kind = event.key.toLowerCase();
    if (kind === 'p' || kind === 'n') { event.preventDefault(); toggleTray(kind === 'p' ? 'pins' : 'notifications'); }
    if (kind === 'k') { event.preventDefault(); searchButton.click(); }
  });
  loadTray('notifications');
  window.addEventListener('focus', () => loadTray('notifications'));
}
