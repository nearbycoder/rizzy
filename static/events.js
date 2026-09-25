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
        const frame = fragment.querySelector('turbo-frame');
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
