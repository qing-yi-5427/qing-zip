// The folder labels stay on one line. Keep the scroll extent of the full
// tree while mounting only the rows near the viewport and the focused row.
class ArchiveTreeWindow {
  constructor(host, rows, renderRow, scrollTop) {
    this.host = host;
    this.viewport = host.closest('.oa-tree-section');
    this.rows = rows;
    this.renderRow = renderRow;
    this.nodes = new Map();
    this.frame = 0;
    this.onScroll = () => {
      if (!this.frame) this.frame = requestAnimationFrame(() => {
        this.frame = 0;
        this.paint();
      });
    };
    this.onKey = event => this.moveFocus(event);
    host.classList.add('oa-windowed-tree');
    host.replaceChildren();
    const first = this.node(0);
    host.append(first);
    this.height = first.getBoundingClientRect().height || 34;
    host.style.height = rows.length * this.height + 'px';
    this.viewport.scrollTop = scrollTop;
    this.resize = new ResizeObserver(this.onScroll);
    this.resize.observe(this.viewport);
    this.viewport.addEventListener('scroll', this.onScroll, {passive:true});
    host.addEventListener('keydown', this.onKey);
    this.paint();
  }

  node(index) {
    let row = this.nodes.get(index);
    if (!row) {
      const template = document.createElement('template');
      template.innerHTML = this.renderRow(this.rows[index]);
      row = template.content.firstElementChild;
      row.dataset.treeIndex = String(index);
      this.nodes.set(index, row);
    }
    return row;
  }

  top() {
    return this.viewport.getBoundingClientRect().top + this.viewport.clientTop
      - this.host.getBoundingClientRect().top;
  }

  paint() {
    const sample = this.host.firstElementChild;
    if (sample) this.height = sample.getBoundingClientRect().height || this.height;
    this.host.style.height = this.rows.length * this.height + 'px';
    const start = Math.max(0, Math.min(this.rows.length - 1, Math.floor(this.top() / this.height)) - 4);
    const end = Math.min(this.rows.length, start + Math.ceil(this.viewport.clientHeight / this.height) + 10);
    const wanted = new Set();
    for (let i = start; i < end; i++) wanted.add(i);
    const focused = document.activeElement?.closest('[data-tree-index]');
    if (focused && this.host.contains(focused)) wanted.add(Number(focused.dataset.treeIndex));
    for (const [i, node] of this.nodes) {
      if (!wanted.has(i)) {
        node.remove();
        this.nodes.delete(i);
      }
    }
    let cursor = this.host.firstElementChild;
    for (const i of [...wanted].sort((a, b) => a - b)) {
      const row = this.node(i);
      row.style.top = i * this.height + 'px';
      if (cursor !== row) this.host.insertBefore(row, cursor);
      cursor = row.nextElementSibling;
    }
  }

  moveFocus(event) {
    if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey) return;
    const row = event.target.closest('[data-tree-index]');
    if (!row) return;
    let index = Number(row.dataset.treeIndex), toggle = false;
    switch (event.key) {
      case 'Tab': {
        const atToggle = event.target.hasAttribute('data-tree-toggle');
        if ((!event.shiftKey && atToggle) || (event.shiftKey && !atToggle && this.rows[index].hasChildren)) return;
        index += event.shiftKey ? -1 : 1;
        toggle = !event.shiftKey && this.rows[index]?.hasChildren;
        break;
      }
      case 'ArrowDown': index++; break;
      case 'ArrowUp': index--; break;
      case 'Home': index = 0; break;
      case 'End': index = this.rows.length - 1; break;
      default: return;
    }
    if (index < 0 || index >= this.rows.length) return;
    event.preventDefault();
    event.stopPropagation();
    const top = this.top(), y = index * this.height;
    if (y < top) this.viewport.scrollTop += y - top;
    else if (y + this.height > top + this.viewport.clientHeight) {
      this.viewport.scrollTop += y + this.height - top - this.viewport.clientHeight;
    }
    this.paint();
    this.nodes.get(index)?.querySelector(toggle ? '[data-tree-toggle]' : '[data-tree]')?.focus({preventScroll:true});
  }

  stop() {
    cancelAnimationFrame(this.frame);
    this.resize.disconnect();
    this.viewport.removeEventListener('scroll', this.onScroll);
    this.host.removeEventListener('keydown', this.onKey);
    this.host.classList.remove('oa-windowed-tree');
    this.host.style.height = '';
    this.nodes.clear();
  }
}
