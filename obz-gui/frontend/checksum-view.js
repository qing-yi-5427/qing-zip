(() => {
  'use strict';
  const basename = path => path.split(/[\\/]/).pop();
  const algorithmKey = name => name.replace(/-/g, '').toUpperCase();

  function hashes(file) {
    if (file.hashes) return Object.entries(file.hashes);
    return ['crc32', 'md5', 'sha1', 'sha256']
      .filter(key => typeof file[key] === 'string')
      .map(key => [key.toUpperCase(), file[key]]);
  }

  function compare(value, file) {
    const line = value.trim();
    if (!line) return {state: '', message: ''};
    const named = /^([\w-]+) \(([^\r\n]+)\) = ([\da-f]+)$/i.exec(line);
    const labeled = /^([\w-]+)\s*:\s*([\da-f]+)$/i.exec(line);
    const simple = /^([\da-f]+)(?:[ \t]+[*]?([^\r\n]+))?$/i.exec(line);
    const digest = named?.[3] ?? labeled?.[2] ?? simple?.[1];
    if (!digest || digest.length < 8 || digest.length % 2) {
      return {state: 'notice', message: '请粘贴一个完整校验值，或校验清单中的一行。'};
    }
    const label = named?.[1] ?? labeled?.[1];
    const candidates = hashes(file).filter(([name, actual]) => label
      ? algorithmKey(name) === algorithmKey(label) : actual.length === digest.length);
    if (!candidates.length) {
      return {state: 'notice', message: label ? '本次没有计算 ' + label + '。请选择该算法后重新计算。' : '本次结果没有这个长度的校验值，请确认所选算法。'};
    }
    const match = candidates.find(([,actual]) => actual.toLowerCase() === digest.toLowerCase());
    if (!match) return {state: 'different', message: '校验值不一致。'};
    const source = named?.[2] ?? simple?.[2];
    const differentName = source && basename(source).toLowerCase() !== basename(file.path).toLowerCase();
    return differentName
      ? {state: 'notice', message: match[0] + ' 校验值一致，清单中的文件名不同。'}
      : {state: 'match', message: match[0] + ' 校验值一致。'};
  }

  function report(files) {
    return files.map(file => [file.path, ...hashes(file).map(([name,value]) => name + ': ' + value)].join('\n')).join('\n\n');
  }

  function render(host, result) {
    const files = result.checksums || [result];
    const add = (parent, tag, text = '', cls = '') => {
      const element = host.ownerDocument.createElement(tag);
      element.textContent = text;
      if (cls) element.className = cls;
      parent.append(element);
      return element;
    };
    if (!files.length) { add(host, 'p', '文件夹中没有可校验的文件。'); return; }
    const feedback = add(host, 'p', '', 'hash-feedback');
    feedback.setAttribute('role', 'status');
    const copy = (parent, text, value, label) => {
      const button = add(parent, 'button', text, 'qa-button quiet');
      button.type = 'button';
      button.setAttribute('aria-label', label);
      button.addEventListener('click', async () => {
        try {
          await navigator.clipboard.writeText(value);
          feedback.textContent = '已' + label + '。';
        } catch {
          feedback.textContent = '无法写入剪贴板，请选中结果后手动复制。';
        }
      });
    };
    if (files.length > 1) {
      const head = add(host, 'div', '', 'hash-head');
      add(head, 'h3', '已计算 ' + files.length + ' 个文件');
      const output = report(files);
      copy(head, '复制全部结果', output, '复制全部结果');
      add(host, 'pre', output, 'checksum-results');
      return;
    }
    const file = files[0];
    const label = add(host, 'label', '比对校验值（选填）', 'hash-label');
    const input = add(label, 'input');
    input.type = 'text';
    input.id = 'hash-reference';
    input.autocomplete = 'off';
    input.spellcheck = false;
    input.placeholder = '粘贴发布方的校验值或 .sha256 清单的一行';
    const verdict = add(host, 'p', '', 'hash-verdict');
    verdict.id = 'hash-verdict';
    verdict.setAttribute('role', 'status');
    input.setAttribute('aria-describedby', verdict.id);
    input.addEventListener('input', () => {
      const result = compare(input.value, file);
      verdict.dataset.state = result.state;
      verdict.textContent = result.message;
    });
    add(host, 'p', file.path, 'hash-source');
    const list = add(host, 'dl', '', 'hash-list');
    for (const [name, value] of hashes(file)) {
      const row = add(list, 'div');
      add(row, 'dt', name);
      const data = add(row, 'dd');
      add(data, 'code', value);
      copy(data, '复制', value, '复制 ' + name + ' 校验值');
    }
  }

  const view = {compare, report, render};
  if (typeof module !== 'undefined') module.exports = view;
  else window.QingChecksumView = view;
})();
