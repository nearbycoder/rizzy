const activity = document.getElementById('activity');
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
