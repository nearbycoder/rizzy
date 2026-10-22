(() => {
  let currentPanel;

  function normalizeFilteredText(value) {
    return value.toLowerCase().normalize('NFD').replace(/[\u0300-\u036f]/g, '');
  }

  function visibleItems(panel) {
    return [...panel.querySelectorAll('.popup__item')].filter(item =>
      !item.hidden && item.checkVisibility());
  }

  function select(panel, item) {
    if (!item) return;
    for (const previous of panel.querySelectorAll('.popup__item[aria-selected]')) {
      previous.removeAttribute('aria-selected');
    }
    item.setAttribute('aria-selected', 'true');
    item.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }

  function selectFirst(panel) {
    select(panel, visibleItems(panel)[0]);
  }

  function update(root) {
    const panel = root.querySelector('[data-assignment-limit-count-value]');
    if (!panel) return;
    const newPanel = panel !== currentPanel;
    currentPanel = panel;
    const count = Number(panel.dataset.assignmentLimitCountValue);
    const limit = Number(panel.dataset.assignmentLimitLimitValue);
    const atLimit = count >= limit;
    const query = normalizeFilteredText(panel.querySelector('#user-search')?.value || '');
    for (const item of panel.querySelectorAll('.popup__item')) {
      const unassigned = item.matches('[data-assignment-limit-target="unassigned"]');
      item.hidden = (atLimit && unassigned) ||
        (query !== '' && !normalizeFilteredText(item.textContent).includes(query));
    }
    const message = panel.querySelector('[data-assignment-limit-target="limitMessage"]');
    if (message) message.hidden = !atLimit;
    if (newPanel) selectFirst(panel);
  }

  function start() {
    const root = document.getElementById('assignment-dialog-content') || document.querySelector('main');
    if (!root) return;
    update(root);
    new MutationObserver(() => update(root)).observe(root, { childList: true, subtree: true });
    document.addEventListener('input', event => {
      if (event.target.id === 'user-search' && root.contains(event.target)) {
        update(root);
        selectFirst(currentPanel);
      }
    });
    document.addEventListener('keydown', event => {
      const panel = event.target.closest?.('[data-assignment-limit-count-value]');
      if (!panel || !root.contains(panel)) return;
      const items = visibleItems(panel);
      const selected = panel.querySelector('.popup__item[aria-selected="true"]');
      const index = items.indexOf(selected);
      if (['ArrowDown', 'ArrowRight', 'ArrowUp', 'ArrowLeft'].includes(event.key)) {
        if (event.shiftKey || event.metaKey || event.ctrlKey) return;
        const offset = event.key === 'ArrowDown' || event.key === 'ArrowRight' ? 1 : -1;
        select(panel, items[index + offset]);
      } else if (event.key === 'Enter' && !event.isComposing && !event.shiftKey &&
                 selected && items.includes(selected)) {
        selected.querySelector('a,button')?.click();
        event.preventDefault();
      }
    });
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start, { once: true });
  } else {
    start();
  }
})();
