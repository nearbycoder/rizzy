document.querySelectorAll('[data-board-involvement]').forEach((button) => {
  button.addEventListener('click', async (event) => {
    event.preventDefault();
    const next = button.dataset.involvement === 'watching' ? 'access_only' : 'watching';
    const error = document.getElementById('board-involvement-error');
    button.disabled = true;
    error.textContent = '';
    try {
      const response = await fetch(button.dataset.url, {
        method: 'PUT',
        credentials: 'same-origin',
        headers: { 'Accept': 'application/json', 'Content-Type': 'application/json' },
        body: JSON.stringify({ involvement: next }),
      });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const watching = next === 'watching';
      const action = watching ? 'Stop watching' : 'Watch this';
      button.dataset.involvement = next;
      button.setAttribute('aria-label', action);
      button.setAttribute('title', action);
      button.setAttribute('aria-pressed', String(watching));
      button.classList.toggle('is-watching', watching);
      button.classList.toggle('btn--reversed', watching);
      button.form.elements.involvement.value = watching ? 'access_only' : 'watching';
      button.querySelector('.for-screen-reader').textContent = action;
    } catch (_) {
      error.textContent = 'Could not update board notifications. Try again.';
    } finally {
      button.disabled = false;
    }
  });
});
