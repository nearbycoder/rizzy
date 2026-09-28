(() => {
  const input = document.querySelector('.search-perma .search__input');
  const reset = document.querySelector('.search-perma .search__reset');
  const timeStyle = new Intl.DateTimeFormat('en-US', { timeStyle: 'short' });
  const shortDate = new Intl.DateTimeFormat('en-US', { month: 'short', day: 'numeric' });
  const dateTime = new Intl.DateTimeFormat('en-US', { dateStyle: 'short', timeStyle: 'short' });
  function formatTimes(scope) {
    const today = new Date().toDateString();
    for (const time of scope.querySelectorAll('time[data-local-time-target="timeordate"]')) {
      const date = new Date(Number(time.getAttribute('datetime')) * 1000);
      if (Number.isNaN(date.valueOf())) continue;
      time.textContent = date.toDateString() === today ? timeStyle.format(date) : shortDate.format(date);
      time.title = dateTime.format(date);
    }
  }
  formatTimes(document);
  reset?.addEventListener('click', () => {
    if (input.value) {
      input.value = '';
      input.focus();
    } else {
      location.assign(document.querySelector('.header__back').href);
    }
  });

  let loading = false;
  const observer = 'IntersectionObserver' in window
    ? new IntersectionObserver(entries => {
        for (const entry of entries) if (entry.isIntersecting) load(entry.target);
      }, { rootMargin: '300px' })
    : null;
  const observe = link => { if (link && observer) observer.observe(link); };
  async function load(link) {
    if (loading || !link.isConnected) return;
    loading = true;
    observer?.unobserve(link);
    link.setAttribute('aria-busy', 'true');
    let nextLink = null;
    try {
      const response = await fetch(link.href, { headers: { Accept: 'text/html' }, credentials: 'same-origin' });
      if (!response.ok) throw new Error(`Search page ${response.status}`);
      const documentPage = new DOMParser().parseFromString(await response.text(), 'text/html');
      const page = documentPage.getElementById(link.dataset.frame);
      if (!page) throw new Error('Search page frame missing');
      const frame = document.createElement('turbo-frame');
      frame.id = page.id;
      frame.setAttribute('role', 'presentation');
      frame.innerHTML = page.innerHTML;
      link.replaceWith(frame);
      formatTimes(frame);
      nextLink = frame.querySelector('.pagination-link');
    } catch (error) {
      link.removeAttribute('aria-busy');
      console.error(error);
    } finally {
      loading = false;
      observe(nextLink);
    }
  }
  document.addEventListener('click', event => {
    const link = event.target.closest('.search-perma .pagination-link');
    if (!link) return;
    event.preventDefault();
    load(link);
  });
  observe(document.querySelector('.search-perma .pagination-link'));
})();
