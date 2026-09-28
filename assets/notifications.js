document.addEventListener('submit', async (event) => {
  const form = event.target.closest('form[data-notification-toggle]');
  if (!form) return;
  event.preventDefault();
  const method = form.querySelector('[name="_method"]')?.value?.toUpperCase() || 'POST';
  const button = form.querySelector('button');
  button.disabled = true;
  try {
    const response = await fetch(form.action, {
      method,
      headers: { Accept: 'text/vnd.turbo-stream.html' },
      credentials: 'same-origin',
    });
    if (!response.ok) throw new Error('Could not update notification');
    const documentFragment = new DOMParser().parseFromString(await response.text(), 'text/html');
    for (const stream of documentFragment.querySelectorAll('turbo-stream')) {
      const targetId = stream.getAttribute('target');
      if (stream.getAttribute('action') === 'prepend' && !document.getElementById(targetId)) {
        if (targetId === 'notifications_list_read') {
          document.querySelector('main').insertAdjacentHTML('beforeend', '<section class="notifications-list notifications-list--read panel panel--wide" id="read_section"><h2>Previously seen</h2><div id="notifications_list_read" contents></div></section>');
        } else if (targetId === 'notifications_list') {
          document.querySelector('main').insertAdjacentHTML('afterbegin', '<section class="notifications-list panel panel--wide"><h2>New for you</h2><div id="notifications_list" contents></div></section>');
        }
      }
      const target = document.getElementById(targetId);
      if (!target) continue;
      if (stream.getAttribute('action') === 'remove') target.remove();
      if (stream.getAttribute('action') === 'prepend') {
        target.prepend(stream.querySelector('template').content.cloneNode(true));
      }
    }
    const unread = document.querySelectorAll('#notifications_list .tray__item').length;
    const blankSlate = document.querySelector('.notifications-list__blank-slate');
    if (blankSlate) blankSlate.hidden = unread > 0;
  } catch (error) {
    button.disabled = false;
    form.insertAdjacentHTML('afterend', '<span role="alert">Could not update notification</span>');
  }
});

let nextPageObserver;
let nextPageLoading = false;

function observeNextPage() {
  nextPageObserver?.disconnect();
  const marker = document.getElementById('next_page');
  if (!marker || marker.dataset.fetchFailed || nextPageLoading) return;
  if (!('IntersectionObserver' in window)) {
    showNextPageRetry(marker);
    return;
  }
  nextPageObserver = new IntersectionObserver((entries) => {
    if (entries.some((entry) => entry.isIntersecting)) {
      nextPageObserver.disconnect();
      loadNextPage(marker);
    }
  });
  nextPageObserver.observe(marker);
}

function showNextPageRetry(marker) {
  marker.dataset.fetchFailed = 'true';
  if (marker.querySelector('button')) return;
  const button = document.createElement('button');
  button.type = 'button';
  button.textContent = 'Load more';
  button.addEventListener('click', () => {
    delete marker.dataset.fetchFailed;
    button.remove();
    loadNextPage(marker);
  });
  marker.append(button);
}

async function loadNextPage(marker) {
  if (nextPageLoading || !marker.isConnected) return;
  nextPageLoading = true;
  marker.setAttribute('aria-busy', 'true');
  try {
    const response = await fetch(marker.dataset.fetchOnVisibleUrlValue, {
      headers: { Accept: 'text/vnd.turbo-stream.html' },
      credentials: 'same-origin',
    });
    if (!response.ok) throw new Error('Could not load notifications');
    const page = new DOMParser().parseFromString(await response.text(), 'text/html');
    const streams = page.querySelectorAll('turbo-stream');
    if (streams.length !== 2) throw new Error('Invalid notification page');
    const append = page.querySelector('turbo-stream[action="append"][target="notifications_list_read"]');
    const replace = page.querySelector('turbo-stream[action="replace"][target="next_page"]');
    const list = document.getElementById('notifications_list_read');
    if (!append || !replace || !list) throw new Error('Invalid notification page');
    list.append(append.querySelector('template').content.cloneNode(true));
    marker.replaceWith(replace.querySelector('template').content.cloneNode(true));
  } catch (error) {
    showNextPageRetry(marker);
  } finally {
    marker.removeAttribute('aria-busy');
    nextPageLoading = false;
    observeNextPage();
  }
}

observeNextPage();
