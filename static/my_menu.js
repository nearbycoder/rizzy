const standaloneMenu = document.getElementById('my_menu');
const standaloneSearch = document.getElementById('menu-search');
standaloneSearch?.addEventListener('input', event => {
  const query = event.target.value.trim().toLocaleLowerCase();
  for (const item of standaloneMenu.querySelectorAll('[data-filter-target="item"]')) {
    item.hidden = !item.textContent.toLocaleLowerCase().includes(query);
  }
  for (const section of standaloneMenu.querySelectorAll('.nav__section')) {
    section.hidden = !!query && !section.querySelector('[data-filter-target="item"]:not([hidden])');
    if (section.classList.contains('nav__section--secret')) {
      if (query) section.setAttribute('data-is-filtering', '');
      else section.removeAttribute('data-is-filtering');
    }
  }
});
standaloneMenu.querySelector('[data-menu-close]')?.addEventListener('click', () => {
  location.href = location.pathname.split('/').slice(0, 2).join('/') + '/';
});
standaloneMenu.addEventListener('submit', async event => {
  if (!event.target.matches('form[action="/session"]')) return;
  event.preventDefault();
  const response = await fetch('/session', { method: 'DELETE' });
  if (response.ok) location.href = '/session/new';
});
