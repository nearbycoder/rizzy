let tagPickerOptionSequence = 0;
const tagPickerFilterTimers = new WeakMap();

function tagPickerItems(picker) {
  return [...picker.querySelectorAll('.popup__item:not([hidden])')];
}

function selectTagPickerItem(picker, item) {
  picker.querySelectorAll('.popup__item[aria-selected]').forEach(option => option.removeAttribute('aria-selected'));
  const input = picker.querySelector('#tag-search');
  if (item) {
    item.setAttribute('aria-selected', 'true');
    item.id ||= `tag-picker-option-${++tagPickerOptionSequence}`;
    input?.setAttribute('aria-activedescendant', item.id);
    item.scrollIntoView({ block: 'nearest' });
  } else {
    input?.removeAttribute('aria-activedescendant');
  }
}

function resetTagPickerSelection(picker) {
  selectTagPickerItem(picker, tagPickerItems(picker)[0]);
}

function filterTagPicker(picker) {
  const normalize = text => text.toLowerCase().normalize('NFD').replace(/[\u0300-\u036f]/g, '');
  const value = normalize(picker.querySelector('#tag-search')?.value || '');
  picker.querySelectorAll('.popup__item[data-filter-target]').forEach(item => {
    item.hidden = !normalize(item.textContent).includes(value);
  });
  resetTagPickerSelection(picker);
}

function scheduleTagPickerFilter(picker) {
  clearTimeout(tagPickerFilterTimers.get(picker));
  tagPickerFilterTimers.set(picker, setTimeout(() => filterTagPicker(picker), 100));
}

function openTagPicker(picker) {
  const input = picker?.querySelector('#tag-search');
  if (!input) return;
  clearTimeout(tagPickerFilterTimers.get(picker));
  input.value = '';
  filterTagPicker(picker);
  input.focus();
}

function navigateTagPicker(event) {
  const picker = event.target.closest('.tag-picker');
  if (!picker || event.isComposing) return;
  const items = tagPickerItems(picker);
  const current = items.findIndex(item => item.getAttribute('aria-selected') === 'true');
  if ((event.key === 'ArrowDown' || event.key === 'ArrowUp') && !event.shiftKey && !event.metaKey && !event.ctrlKey) {
    const next = event.key === 'ArrowDown' ? Math.min(current + 1, items.length - 1) : Math.max(current - 1, 0);
    if (items[next]) selectTagPickerItem(picker, items[next]);
    event.preventDefault();
  } else if (event.key === 'Enter' && items[current]) {
    event.preventDefault();
    const button = items[current].querySelector('button');
    if (button && (button.form !== picker.querySelector('form[id^="tags_form_card_"]') || picker.querySelector('#tag-search').value.trim())) {
      button.click();
    }
  }
}
