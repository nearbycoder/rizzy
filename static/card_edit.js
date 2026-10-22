const cardEditForm = document.querySelector('form[id^="edit_form_card_"]');
if (cardEditForm) void restoreCardEditDraft(cardEditForm);
async function cancelDirectCardEdit(form) {
  const frame = form.closest('turbo-frame');
  try {
    const response = await fetch(form.action, { headers: { Accept: 'text/html', 'Turbo-Frame': frame.id } });
    if (!response.ok) throw Error(`Could not close editor (${response.status})`);
    const page = new DOMParser().parseFromString(await response.text(), 'text/html');
    const replacement = page.getElementById(frame.id);
    if (!replacement) throw Error('Card display frame missing');
    frame.replaceWith(document.importNode(replacement, true));
  } catch (cause) {
    form.querySelector('#edit-error').textContent = cause.message;
  }
}
cardEditForm?.addEventListener('keydown', event => {
  if (event.isComposing) return;
  if (event.key === 'Escape') {
    event.preventDefault();
    void cancelDirectCardEdit(cardEditForm);
  } else if (event.key === 'Enter' && (event.ctrlKey || event.metaKey || event.target.matches('textarea[name="card[title]"]'))) {
    event.preventDefault();
    if (cardEditForm.checkValidity()) cardEditForm.requestSubmit();
  }
}, true);
cardEditForm?.querySelector('a[href]')?.addEventListener('click', event => {
  event.preventDefault();
  void cancelDirectCardEdit(cardEditForm);
});
cardEditForm?.addEventListener('submit', async event => {
  event.preventDefault();
  const form = event.currentTarget;
  const button = event.submitter;
  const error = form.querySelector('#edit-error');
  if (button) button.disabled = true;
  error.textContent = '';
  try {
    const response = await fetch(form.action, {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json', 'Accept': 'text/vnd.turbo-stream.html' },
      body: JSON.stringify({ card: {
        title: form.querySelector('[name="card[title]"]').value,
        description: form.querySelector('[name="card[description]"]').value
      } })
    });
    if (!response.ok) throw Error(`Could not save changes (${response.status})`);
    const page = new DOMParser().parseFromString(await response.text(), 'text/html');
    const frame = form.closest('turbo-frame');
    const stream = [...page.querySelectorAll('turbo-stream[action="update"]')]
      .find(item => item.getAttribute('target') === frame?.id);
    const template = stream?.querySelector('template');
    if (!template) throw Error('Card update frame missing');
    clearCardEditDraft(form);
    frame.replaceChildren(document.importNode(template.content, true));
  } catch (cause) {
    error.textContent = cause.message;
    if (button) button.disabled = false;
  }
});
