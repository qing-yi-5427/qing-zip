// The icon CSS grid supplies the actual column tracks and each tile has two
// reserved caption lines. Keep the scroll height while mounting visible bands.
class ArchiveIconWindow {
  constructor(body, viewport, list, renderRow, cursorIndex, watch, unwatch) {
    Object.assign(this, {body,viewport,list,renderRow,cursorIndex,watch,unwatch});
    this.iconGrid = true;
    this.nodes = new Map();
    this.frame = 0;
    this.schedule = () => {
      if (!this.frame) this.frame = requestAnimationFrame(() => { this.frame = 0; this.paint(); });
    };
    body.replaceChildren();
    this.mount(0);
    this.resize = new ResizeObserver(this.schedule);
    this.resize.observe(viewport);
    viewport.addEventListener('scroll', this.schedule, {passive:true});
    this.paint();
  }
  stop() {
    this.resize.disconnect();
    cancelAnimationFrame(this.frame);
    this.viewport.removeEventListener('scroll', this.schedule);
    for (const node of this.nodes.values()) this.unwatch(node);
    this.body.style.removeProperty('height');
  }
  mount(index) {
    this.body.insertAdjacentHTML('beforeend', this.renderRow(this.list[index], index));
    const node = this.body.lastElementChild;
    this.nodes.set(index, node);
    this.watch(node);
    return node;
  }
  visibleTop() {
    return this.viewport.getBoundingClientRect().top - this.body.getBoundingClientRect().top - this.padding;
  }
  paint() {
    if (!this.body.isConnected || !this.body.clientWidth) return;
    const css = getComputedStyle(this.body);
    const template = css.gridTemplateColumns;
    const widths = template.split(/\s+/).map(parseFloat);
    const columns = widths.length;
    const columnGap = parseFloat(css.columnGap) || 0;
    let left = parseFloat(css.paddingInlineStart) || 0;
    const offsets = widths.map(width => {
      const offset = left;
      left += width + columnGap;
      return offset;
    });
    const gap = parseFloat(css.rowGap) || 0;
    const tileHeight = this.nodes.values().next().value.getBoundingClientRect().height;
    const stride = tileHeight + gap;
    if (!tileHeight) return;
    this.padding = parseFloat(css.paddingTop) || 0;
    const before = this.visibleTop();
    const changed = template !== this.template || stride !== this.stride;
    if (changed) {
      const total = Math.ceil(this.list.length / columns);
      this.body.style.height = (this.padding + total * stride - gap + (parseFloat(css.paddingBottom) || 0)) + 'px';
      if (before > 0 && this.columns) {
        const index = Math.floor(before / this.stride) * this.columns;
        const intra = before % this.stride;
        this.viewport.scrollTop += Math.floor(index / columns) * stride + intra * stride / this.stride - before;
      }
      this.template = template;
      this.columns = columns;
      this.stride = stride;
    }
    const top = this.visibleTop();
    const start = Math.max(0, Math.floor(top / stride) - 2) * columns;
    const end = Math.min(this.list.length, (Math.ceil((top + this.viewport.clientHeight) / stride) + 2) * columns);
    const cursor = this.cursorIndex();
    if (!changed && start === this.start && end === this.end && cursor === this.cursor) return;
    this.start = start; this.end = end; this.cursor = cursor;
    const indices = [];
    if (cursor >= 0 && cursor < start) indices.push(cursor);
    for (let i = start; i < end; i++) indices.push(i);
    if (cursor >= end && cursor < this.list.length) indices.push(cursor);
    const wanted = new Set(indices);
    for (const [index,node] of this.nodes) {
      if (!wanted.has(index)) { this.unwatch(node); node.remove(); this.nodes.delete(index); }
    }
    let next = this.body.firstElementChild;
    const focused = this.body.ownerDocument.activeElement;
    for (const i of indices) {
      const node = this.nodes.get(i) || this.mount(i);
      const column = i % columns;
      node.style.insetInlineStart = offsets[column] + 'px';
      node.style.width = widths[column] + 'px';
      node.style.top = (this.padding + Math.floor(i / columns) * stride) + 'px';
      if (node === next) next = next.nextElementSibling;
      else if (node.nextElementSibling !== next) this.body.insertBefore(node,next);
    }
    if (this.body.contains(focused) && this.body.ownerDocument.activeElement !== focused) focused.focus({preventScroll:true});
  }
  reveal(index) {
    this.paint();
    if (!this.columns || index < 0 || index >= this.list.length) return;
    const top = this.visibleTop(), band = Math.floor(index / this.columns) * this.stride;
    const bounds = this.viewport.getBoundingClientRect();
    const path = this.viewport.querySelector('.qa-path'), actions = this.viewport.querySelector('.oa-selection-actions:not([hidden])');
    const start = top + (path ? Math.max(0, path.getBoundingClientRect().bottom - bounds.top) : 0);
    const end = top + (actions ? Math.min(bounds.bottom, actions.getBoundingClientRect().top) : bounds.bottom) - bounds.top;
    const bottom = band + this.nodes.values().next().value.getBoundingClientRect().height;
    if (band < start) this.viewport.scrollTop += band - start;
    else if (bottom > end) this.viewport.scrollTop += bottom - end;
    this.paint();
  }
}
