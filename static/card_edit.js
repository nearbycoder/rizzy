const cardEditForm = document.querySelector('form[id^="edit_form_card_"]');
if (cardEditForm) void restoreCardEditDraft(cardEditForm);
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
      headers: { 'Content-Type': 'application/json', 'Accept': 'application/json' },
      body: JSON.stringify({ card: {
        title: form.querySelector('[name="card[title]"]').value,
        description: form.querySelector('[name="card[description]"]').value
      } })
    });
    if (!response.ok) throw Error(`Could not save changes (${response.status})`);
    clearCardEditDraft(form);
    location.href = form.action;
  } catch (cause) {
    error.textContent = cause.message;
    if (button) button.disabled = false;
  }
});
