const cardEditDraftTimers = new WeakMap();

function cardEditDraftKey(form) {
  return form?.dataset.localSaveKeyValue;
}

async function restoreCardEditDraft(form) {
  const key = cardEditDraftKey(form);
  if (!key) return;
  await customElements.whenDefined('lexxy-editor');
  await new Promise(resolve => requestAnimationFrame(resolve));
  if (!form.isConnected) return;
  const saved = localStorage.getItem(key);
  if (!saved) return;
  const editor = form.querySelector('lexxy-editor[name="card[description]"]');
  if (!editor) return;
  const content = `<div>${saved}</div>`;
  editor.value = content;
  editor.dispatchEvent(new CustomEvent('lexxy:change', {
    bubbles: true,
    detail: { previousContent: '', newContent: content }
  }));
}

function clearCardEditDraft(form) {
  clearTimeout(cardEditDraftTimers.get(form));
  const key = cardEditDraftKey(form);
  if (key) localStorage.removeItem(key);
}

document.addEventListener('lexxy:change', event => {
  const editor = event.target.closest?.('form[id^="edit_form_card_"] lexxy-editor[name="card[description]"]');
  if (!editor) return;
  const form = editor.closest('form');
  clearTimeout(cardEditDraftTimers.get(form));
  cardEditDraftTimers.set(form, setTimeout(() => {
    const key = cardEditDraftKey(form);
    if (!key) return;
    const content = editor.value;
    if (content) localStorage.setItem(key, content);
    else localStorage.removeItem(key);
  }, 300));
});
