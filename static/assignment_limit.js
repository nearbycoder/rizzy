(() => {
  function update(root) {
    const panel = root.querySelector('[data-assignment-limit-count-value]');
    if (!panel) return;
    const count = Number(panel.dataset.assignmentLimitCountValue);
    const limit = Number(panel.dataset.assignmentLimitLimitValue);
    const atLimit = count >= limit;
    const query = panel.querySelector('#user-search')?.value.trim().toLocaleLowerCase() || '';
    for (const item of panel.querySelectorAll('.popup__item')) {
      const unassigned = item.matches('[data-assignment-limit-target="unassigned"]');
      item.hidden = (atLimit && unassigned) ||
        (query !== '' && !item.textContent.toLocaleLowerCase().includes(query));
    }
    const message = panel.querySelector('[data-assignment-limit-target="limitMessage"]');
    if (message) message.hidden = !atLimit;
  }

  function start() {
    const root = document.getElementById('assignment-dialog-content') || document.querySelector('main');
    if (!root) return;
    update(root);
    new MutationObserver(() => update(root)).observe(root, { childList: true, subtree: true });
    document.addEventListener('input', event => {
      if (event.target.id === 'user-search') update(root);
    });
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start, { once: true });
  } else {
    start();
  }
})();
