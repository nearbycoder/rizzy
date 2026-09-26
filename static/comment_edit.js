const commentFrame = () => document.querySelector('turbo-frame[id^="container_comment_"]');

async function applyCommentStream(response) {
  if (!response.ok) throw new Error(`Could not update comment (${response.status})`);
  const page = new DOMParser().parseFromString(await response.text(), 'text/html');
  const stream = page.querySelector('turbo-stream');
  const frame = commentFrame();
  if (!stream || !frame || stream.getAttribute('target') !== frame.id) {
    throw new Error('Comment update missing');
  }
  if (stream.getAttribute('action') === 'remove') {
    frame.remove();
    return;
  }
  const replacement = stream.querySelector('template')?.content.firstElementChild;
  if (stream.getAttribute('action') !== 'replace' || !replacement) {
    throw new Error('Unexpected comment update');
  }
  frame.replaceWith(document.importNode(replacement, true));
}

document.addEventListener('submit', async event => {
  const form = event.target;
  const editing = form.id.startsWith('edit_comment_');
  const deleting = form.id.startsWith('delete_comment_');
  if (!editing && !deleting) return;
  event.preventDefault();
  if (deleting && !confirm('Are you sure you want to delete this comment?')) return;
  const button = event.submitter;
  if (button) button.disabled = true;
  try {
    const response = await fetch(form.action, editing ? {
      method: 'PATCH',
      headers: { Accept: 'text/vnd.turbo-stream.html', 'Content-Type': 'application/json' },
      body: JSON.stringify({ comment: { body: form.querySelector('lexxy-editor').value } }),
    } : { method: 'DELETE', headers: { Accept: 'text/vnd.turbo-stream.html' } });
    await applyCommentStream(response);
  } catch (error) {
    const message = commentFrame()?.querySelector('#comment_error');
    if (message) message.textContent = error.message;
    if (button) button.disabled = false;
  }
});

document.addEventListener('click', async event => {
  const cancel = event.target.closest('a[data-comment-cancel]');
  if (!cancel) return;
  event.preventDefault();
  try {
    const response = await fetch(cancel.href, { headers: { Accept: 'text/html' } });
    if (!response.ok) throw new Error(`Could not load comment (${response.status})`);
    const page = new DOMParser().parseFromString(await response.text(), 'text/html');
    const replacement = page.querySelector('turbo-frame[id^="container_comment_"]');
    if (!replacement || replacement.id !== commentFrame()?.id) {
      throw new Error('Comment frame missing');
    }
    commentFrame().replaceWith(document.importNode(replacement, true));
  } catch (error) {
    const message = commentFrame()?.querySelector('#comment_error');
    if (message) message.textContent = error.message;
  }
});

document.addEventListener('keydown', event => {
  if (event.isComposing) return;
  const form = event.target.closest('form[id^="edit_comment_"]');
  if (!form) return;
  if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
    event.preventDefault();
    if (form.checkValidity()) form.requestSubmit();
  } else if (event.key === 'Escape') {
    event.preventDefault();
    commentFrame()?.querySelector('a[data-comment-cancel]')?.click();
  }
});
