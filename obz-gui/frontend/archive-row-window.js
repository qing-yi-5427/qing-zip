// Prefix sums include measured corrections for wrapped names and compact rows.
class ArchiveRowHeights {
  constructor(length, estimate) {
    this.length = length;
    this.estimate = estimate;
    this.values = new Map();
    this.deltas = new Float64Array(length + 1);
  }
  before(index) {
    let height = index * this.estimate;
    for (let i = index; i > 0; i -= i & -i) height += this.deltas[i];
    return height;
  }
  at(offset) {
    let index = 0, height = 0;
    for (let step = 2 ** Math.floor(Math.log2(this.length)); step >= 1; step /= 2) {
      const next = index + step;
      if (next > this.length) continue;
      const total = height + step * this.estimate + this.deltas[next];
      if (total <= offset) { index = next; height = total; }
    }
    return Math.min(index, this.length - 1);
  }
  measure(index, height) {
    const delta = height - (this.values.get(index) ?? this.estimate);
    if (!(height > 0) || Math.abs(delta) < 0.05) return false;
    if (height === this.estimate) this.values.delete(index); else this.values.set(index, height);
    for (let i = index + 1; i <= this.length; i += i & -i) this.deltas[i] += delta;
    return true;
  }
}

// A tab restores the previous scroll offset against the same measured listing.
const measuredArchiveLists = new WeakMap();
class ArchiveRowWindow {
  constructor(body, viewport, list, renderRow, cursorIndex) {
    this.body = body;
    this.viewport = viewport;
    this.list = list;
    this.renderRow = renderRow;
    this.cursorIndex = cursorIndex;
    this.nodes = new Map();
    this.gaps = [];
    this.frame = 0;
    this.schedule = () => {
      if (!this.frame) this.frame = requestAnimationFrame(() => { this.frame = 0; this.paint(); });
    };
    this.resize = new ResizeObserver(() => { this.measureAgain = true; this.schedule(); });
    this.resize.observe(viewport);
    viewport.addEventListener('scroll', this.schedule, {passive:true});
    body.replaceChildren();
    this.paint();
  }
  stop() {
    cancelAnimationFrame(this.frame);
    this.resize.disconnect();
    this.viewport.removeEventListener('scroll', this.schedule);
  }
  visibleTop() {
    return this.viewport.getBoundingClientRect().top - this.body.previousElementSibling.getBoundingClientRect().bottom;
  }
  gap(slot) {
    if (!this.gaps[slot]) {
      const row = this.body.ownerDocument.createElement('tr');
      row.className = 'oa-row-gap';
      row.setAttribute('aria-hidden', 'true');
      row.setAttribute('role', 'presentation');
      row.insertCell().colSpan = 6;
      this.gaps[slot] = row;
    }
    return this.gaps[slot];
  }
  paint() {
    if (!this.body.isConnected) return;
    const width = this.viewport.clientWidth;
    const estimate = parseFloat(getComputedStyle(this.body).getPropertyValue('--qa-row')) || 56;
    let top = this.visibleTop();
    if (width !== this.width || this.heights?.estimate !== estimate) {
      const old = this.heights;
      this.measureAgain = true;
      this.width = width;
      const cache = measuredArchiveLists.get(this.list);
      this.heights = cache?.width === width && cache.heights.estimate === estimate
        ? cache.heights : new ArchiveRowHeights(this.list.length, estimate);
      measuredArchiveLists.set(this.list, {width, heights:this.heights});
      if (old && top > 0) {
        const index = old.at(top);
        top += this.heights.before(index) - old.before(index);
      }
    }
    const heights = this.heights, anchor = heights.at(Math.max(top, 0));
    const before = heights.before(anchor), indices = [];
    const start = Math.max(0, anchor - 8);
    const end = Math.min(this.list.length, heights.at(top + this.viewport.clientHeight) + 9);
    const cursor = this.cursorIndex();
    if (!this.measureAgain && start === this.start && end === this.end && cursor === this.cursor) return;
    this.start = start; this.end = end; this.cursor = cursor; this.measureAgain = false;
    for (let i = start; i < end; i++) indices.push(i);
    if (cursor >= 0 && cursor < start) indices.unshift(cursor);
    if (cursor >= end && cursor < this.list.length) indices.push(cursor);
    const focused = this.body.ownerDocument.activeElement;
    const preserveFocus = focused && this.body.contains(focused);
    const wanted = new Set(indices);
    for (const [i, node] of this.nodes) {
      if (!wanted.has(i)) { this.resize.unobserve(node); node.remove(); this.nodes.delete(i); }
    }
    const missing = indices.filter(i => !this.nodes.has(i));
    if (missing.length) {
      this.body.insertAdjacentHTML('beforeend', missing.map(i => this.renderRow(this.list[i], i)).join(''));
      let row = this.body.lastElementChild;
      for (let j = missing.length - 1; j >= 0; j--) {
        this.nodes.set(missing[j], row);
        this.resize.observe(row);
        row = row.previousElementSibling;
      }
    }
    const order = [], spans = [];
    let first = 0;
    const addGap = endIndex => {
      if (first === endIndex) return;
      const node = this.gap(spans.length);
      spans.push({node, first, end:endIndex});
      order.push(node);
    };
    for (const i of indices) {
      addGap(i);
      order.push(this.nodes.get(i));
      first = i + 1;
    }
    addGap(this.list.length);
    this.gaps.slice(spans.length).forEach(node => node.remove());
    // Retain the actual focused button across wheel scrolling.
    let next = this.body.firstElementChild;
    for (const node of order) {
      if (node === next) next = next.nextElementSibling;
      else this.body.insertBefore(node, next);
    }
    if (preserveFocus && this.body.contains(focused) && this.body.ownerDocument.activeElement !== focused) focused.focus({preventScroll:true});
    const measured = indices.map(i => this.nodes.get(i).getBoundingClientRect().height);
    let resized = false;
    indices.forEach((i, j) => { if (heights.measure(i, measured[j])) resized = true; });
    for (const {node, first, end} of spans) {
      const height = (heights.before(end) - heights.before(first)) + 'px';
      if (node.firstElementChild.style.height !== height) node.firstElementChild.style.height = height;
    }
    if (top > 0) {
      const correction = top + heights.before(anchor) - before - this.visibleTop();
      if (Math.abs(correction) >= 0.05) this.viewport.scrollTop += correction;
    }
    if (resized) this.schedule();
  }
  reveal(index) {
    const top = this.visibleTop(), start = this.heights.before(index), end = this.heights.before(index + 1);
    const bounds = this.viewport.getBoundingClientRect();
    const pathBar = this.viewport.querySelector('.qa-path');
    const actions = this.viewport.querySelector('.oa-selection-actions:not([hidden])');
    const insetTop = pathBar ? Math.max(0, pathBar.getBoundingClientRect().bottom - bounds.top) : 0;
    const insetBottom = actions ? Math.max(0, bounds.bottom - actions.getBoundingClientRect().top) : 0;
    const visibleStart = top + insetTop, visibleEnd = top + this.viewport.clientHeight - insetBottom;
    if (start < visibleStart) this.viewport.scrollTop += start - visibleStart;
    else if (end > visibleEnd) this.viewport.scrollTop += end - visibleEnd;
    this.paint();
  }
}
