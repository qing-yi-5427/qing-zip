(() => {
 'use strict';

 window.QingPasswordBook = ({invoke, esc}) => {
  const mask = '••••••••';
  const pickerMarkup = id => `<div class="oa-password-picker">
   <button type="button" class="qa-button quiet" data-password-picker="${id}" aria-expanded="false" aria-controls="${id}-book">从密码本选择</button>
   <div id="${id}-book" class="oa-password-choices" hidden></div>
  </div>`;
  const pageMarkup = `<form class="oa-password-add" autocomplete="off" data-book-add>
   <div class="qa-fieldrow">
    <label class="qa-field">备注（选填）<input data-book-label type="text"></label>
    <label class="qa-field">密码<input data-book-password type="password" autocomplete="new-password" required></label>
   </div>
   <button type="submit" class="qa-button primary">添加密码</button>
   <p data-book-save-status role="status"></p>
  </form>
  <section class="qa-setting-group" aria-labelledby="oa-password-book-title">
   <h3 id="oa-password-book-title" tabindex="-1">已保存的密码</h3>
   <p class="qa-sub">成功解密并解压时使用的密码会自动保存。输入解压密码时可在密码本中选用。</p>
   <p data-book-status role="status"></p>
   <div data-book-entries></div>
  </section>`;

  function distinguishEntries(entries) {
   const totals = new Map();
   const occurrences = new Map();
   const named = entries.map(({id, label}) => ({id, label: label.trim() ? label : id}));
   for (const {label} of named) totals.set(label, (totals.get(label) ?? 0) + 1);
   return named.map(({id, label}) => {
    if (totals.get(label) === 1) return {id, label};
    const occurrence = (occurrences.get(label) ?? 0) + 1;
    occurrences.set(label, occurrence);
    return {id, label: `${label} · ${occurrence}`};
   });
  }

  function closePicker(input) {
   const picker = input.closest('label')?.nextElementSibling;
   const button = picker?.querySelector('[data-password-picker]');
   if (!button) return;
   const choices = picker.querySelector('.oa-password-choices');
   button.setAttribute('aria-expanded', 'false');
   choices.hidden = true;
   choices.replaceChildren();
  }

  function mountPickers(container) {
   for (const input of container.querySelectorAll('input[data-password-book]')) {
    const field = input.closest('label');
    field.insertAdjacentHTML('afterend', pickerMarkup(input.id));
    const picker = field.nextElementSibling;
    const button = picker.querySelector('[data-password-picker]');
    const choices = picker.querySelector('.oa-password-choices');
    button.onclick = async () => {
     if (input.disabled || button.disabled) return;
     if (!choices.hidden) {
      closePicker(input);
      return;
     }
     const status = document.createElement('p');
     status.setAttribute('role', 'status');
     status.textContent = '正在读取密码本…';
     choices.replaceChildren(status);
     choices.hidden = false;
     button.setAttribute('aria-expanded', 'true');
     const current = () => status.isConnected && !input.disabled && !choices.hidden;
     try {
      const entries = await invoke('password_book_list');
      if (!current()) return;
      status.textContent = entries.length ? '' : '密码本暂无记录。';
      for (const {id, label} of distinguishEntries(entries)) {
       const item = document.createElement('button');
       item.type = 'button';
       item.className = 'qa-button';
       item.textContent = label;
       item.title = label;
       item.onclick = async () => {
        const previous = input.value;
        choices.querySelectorAll('button').forEach(entry => { entry.disabled = true; });
        try {
         const password = await invoke('password_book_get', {id});
         if (!current() || input.value !== previous || button.disabled || input.readOnly) return;
         input.value = password;
         closePicker(input);
         input.focus();
        } catch {
         if (current()) status.textContent = '无法读取此密码，请重新打开密码本或手动输入。';
        } finally {
         if (current()) choices.querySelectorAll('button').forEach(entry => { entry.disabled = false; });
        }
       };
       choices.append(item);
      }
     } catch {
      if (current()) status.textContent = '无法读取密码本，可手动输入密码。';
     }
    };
   }
  }

  let pendingWrite = null;
  let listRequest = 0;
  function write(command, args) {
   const request = invoke(command, args);
   pendingWrite = request;
   return request.finally(() => { pendingWrite = null; });
  }

  function mountPage(container) {
   const form = container.querySelector('[data-book-add]');
   const label = form.querySelector('[data-book-label]');
   const password = form.querySelector('[data-book-password]');
   const submit = form.querySelector('button');
   const saveStatus = form.querySelector('[data-book-save-status]');
   form.onsubmit = async event => {
    event.preventDefault();
    if (submit.disabled || pendingWrite || !password.value) return;
    submit.disabled = label.disabled = password.disabled = true;
    saveStatus.textContent = '正在保存密码…';
    try {
     await write('password_book_save', {label: label.value, password: password.value});
     form.reset();
     if (form.isConnected) {
      saveStatus.textContent = '已保存密码。';
      await loadEntries(container);
     }
    } catch {
     if (form.isConnected) saveStatus.textContent = '无法保存密码，请重试。';
    } finally {
     submit.disabled = label.disabled = password.disabled = false;
     if (form.isConnected && (document.activeElement === document.body || form.contains(document.activeElement))) {
      password.focus({preventScroll: true});
     }
    }
   };
   loadEntries(container);
  }

  async function loadEntries(container) {
   const heading = container.querySelector('#oa-password-book-title');
   const status = container.querySelector('[data-book-status]');
   const list = container.querySelector('[data-book-entries]');
   const request = ++listRequest;
   const current = () => list.isConnected && request === listRequest;
   list.replaceChildren();
   status.textContent = '正在读取密码本…';
   try {
    if (pendingWrite) await pendingWrite.catch(() => {});
    if (!current()) return;
    const entries = await invoke('password_book_list');
    if (!current()) return;
    status.textContent = entries.length ? '' : '密码本暂无记录。';
    for (const {id, label} of distinguishEntries(entries)) {
     const row = document.createElement('div');
     row.className = 'qa-setting-row oa-password-entry';
     row.innerHTML = `<div><strong>${esc(label)}</strong>
      <input type="text" readonly autocomplete="off" aria-label="密码 · ${esc(label)}"></div>
      <div class="qa-actions"><button type="button" class="qa-button quiet" aria-expanded="false">显示密码</button>
      <button type="button" class="qa-button quiet">删除</button></div>`;
     const value = row.querySelector('input');
     const [reveal, remove] = row.querySelectorAll('button');
     value.value = mask;
     reveal.setAttribute('aria-label', '显示密码 · ' + label);
     remove.setAttribute('aria-label', '删除 · ' + label);
     const conceal = () => {
      value.value = mask;
      reveal.setAttribute('aria-expanded', 'false');
      reveal.setAttribute('aria-label', '显示密码 · ' + label);
      reveal.textContent = '显示密码';
     };
     reveal.onclick = async () => {
      if (reveal.getAttribute('aria-expanded') === 'true') {
       conceal();
       return;
      }
      reveal.disabled = true;
      status.textContent = '';
      try {
       const password = await invoke('password_book_get', {id});
       if (!row.isConnected || remove.disabled) return;
       value.value = password;
       reveal.setAttribute('aria-expanded', 'true');
       reveal.setAttribute('aria-label', '隐藏密码 · ' + label);
       reveal.textContent = '隐藏密码';
      } catch {
       if (row.isConnected) status.textContent = '无法读取此密码。';
      } finally {
       reveal.disabled = remove.disabled;
      }
     };
     remove.onclick = async () => {
      if (pendingWrite || !window.confirm('从密码本删除此密码？')) return;
      conceal();
      reveal.disabled = remove.disabled = true;
      status.textContent = '';
      try {
       await write('password_book_delete', {id});
       if (!row.isConnected) return;
       row.remove();
       if (!list.childElementCount) status.textContent = '密码本暂无记录。';
       heading.focus({preventScroll: true});
      } catch {
       if (row.isConnected) status.textContent = '无法删除此密码，请重试。';
       reveal.disabled = remove.disabled = false;
      }
     };
     list.append(row);
    }
   } catch {
    if (current()) status.textContent = '无法读取密码本。';
   }
  }

  return {pageMarkup, mountPickers, mountPage, closePicker};
 };
})();
