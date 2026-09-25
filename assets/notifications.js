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
