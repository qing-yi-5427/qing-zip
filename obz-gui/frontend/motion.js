// Motion for state changes. CSS handles colour and transform transitions on
// controls (motion.css); this module choreographs what CSS cannot: things that
// appear, disappear, move or change size. Everything here is purely visual —
// application state (hidden, content, focus) always changes synchronously, and
// exits play on inert copies, so an interrupted animation never leaves the UI
// in a half state. Durations and curves come from the CSS tokens in ui.css.
(() => {
  'use strict';
  const reduceQuery = matchMedia('(prefers-reduced-motion: reduce)');
  const forcedQuery = matchMedia('(forced-colors: active)');
  let tokens = null;
  function token() {
    if (tokens) return tokens;
    const style = getComputedStyle(document.documentElement);
    const ms = (name, fallback) => parseFloat(style.getPropertyValue(name)) || fallback;
    const curve = (name, fallback) => style.getPropertyValue(name).trim() || fallback;
    tokens = {
      quick: ms('--qa-dur-quick', 140), base: ms('--qa-dur-base', 200),
      sheet: ms('--qa-dur-sheet', 260), exit: ms('--qa-dur-exit', 140),
      out: curve('--qa-ease-out', 'ease-out'), in: curve('--qa-ease-in', 'ease-in'),
      move: curve('--qa-ease-move', 'ease-in-out'),
    };
    return tokens;
  }
  const enabled = () => !reduceQuery.matches && typeof Element.prototype.animate === 'function';

  // One running animation per element and channel; a new one replaces it.
  const running = new WeakMap();
  const entering = new Set();
  function play(el, frames, options, channel = 'main') {
    if (!enabled() || !el?.isConnected) return null;
    let channels = running.get(el);
    if (!channels) running.set(el, channels = new Map());
    channels.get(channel)?.cancel();
    const animation = el.animate(frames, { fill: 'none', ...options });
    channels.set(channel, animation);
    const done = () => { if (channels.get(channel) === animation) channels.delete(channel); };
    animation.finished.then(done, done);
    return animation;
  }
  function enter(el, frames, options) {
    const animation = play(el, frames, options);
    if (!animation) return null;
    entering.add(el);
    const done = () => entering.delete(el);
    animation.finished.then(done, done);
    return animation;
  }
  const insideEntrance = el => [...entering].some(host => host !== el && host.contains(el));

  // An inert copy that plays an exit while the real element has already gone.
  // Ids are dropped so lookups keep finding the live element. A copy that
  // must keep id-based styling (keepIds) is always placed after the live
  // element, which therefore stays first for getElementById.
  function ghost(el, className, keepIds = false) {
    const copy = el.cloneNode(true);
    if (!keepIds) {
      copy.removeAttribute('id');
      for (const node of copy.querySelectorAll('[id]')) node.removeAttribute('id');
    }
    const from = el.querySelectorAll('input,select,textarea');
    const to = copy.querySelectorAll('input,select,textarea');
    if (from.length <= 200) from.forEach((field, index) => {
      const target = to[index];
      if (!target) return;
      if (field.type === 'checkbox' || field.type === 'radio') target.checked = field.checked;
      else if (field.type !== 'password') target.value = field.value;
    });
    copy.hidden = false;
    copy.inert = true;
    copy.setAttribute('aria-hidden', 'true');
    copy.classList.add('qa-motion-ghost');
    if (className) copy.classList.add(className);
    return copy;
  }
  function removeWhenDone(node, animation) {
    const remove = () => node.remove();
    animation ? animation.finished.then(remove, remove) : remove();
  }

  // Dialogs: the sheet rises out of the dimmed backdrop; replacing an open
  // dialog only cross-fades the content. Closing plays on a copy.
  function dialogShown(overlay, wasOpen) {
    const sheet = overlay.firstElementChild;
    if (!sheet) return;
    const t = token();
    if (wasOpen) {
      play(sheet, [{ opacity: 0, translate: '0 4px' }, { opacity: 1, translate: '0 0' }], { duration: t.quick, easing: t.out });
      return;
    }
    enter(overlay, [{ opacity: 0 }, { opacity: 1 }], { duration: t.base, easing: t.out });
    play(sheet, [{ opacity: 0, translate: '0 12px', scale: '.98' }, { opacity: 1, translate: '0 0', scale: '1' }], { duration: t.sheet, easing: t.out });
  }
  function dialogClosing(overlay) {
    if (!enabled() || overlay.hidden || !overlay.firstElementChild) return;
    const t = token();
    const opacity = getComputedStyle(overlay).opacity;
    const copy = ghost(overlay, 'qa-overlay-ghost');
    overlay.after(copy);
    const fade = copy.animate([{ opacity }, { opacity: 0 }], { duration: t.exit, easing: t.in, fill: 'forwards' });
    copy.firstElementChild?.animate([{ translate: '0 0', scale: '1' }, { translate: '0 6px', scale: '.985' }], { duration: t.exit, easing: t.in, fill: 'forwards' });
    removeWhenDone(copy, fade);
  }

  // Menus grow out of the control that opened them. Moving between the menu
  // bar and a submenu slides in the direction of travel.
  function menuShown(box, anchorRect, wasOpen) {
    const t = token();
    const depth = box.querySelector('.oa-submenu-head') ? 1 : 0;
    const previous = Number(box.dataset.motionDepth ?? depth);
    box.dataset.motionDepth = String(depth);
    if (wasOpen) {
      const direction = Math.sign(depth - previous);
      play(box, [{ opacity: 0, translate: `${direction * 10}px 0` }, { opacity: 1, translate: '0 0' }], { duration: t.quick, easing: t.out });
      return;
    }
    const bounds = box.getBoundingClientRect();
    const originX = anchorRect ? Math.max(0, Math.min(bounds.width, anchorRect.left + (anchorRect.width || 0) / 2 - bounds.left)) : 0;
    const originY = anchorRect && anchorRect.top > bounds.top ? bounds.height : 0;
    box.style.transformOrigin = `${originX}px ${originY}px`;
    play(box, [{ opacity: 0, scale: '.96' }, { opacity: 1, scale: '1' }], { duration: t.quick, easing: t.out });
  }
  function menuClosing(box) {
    if (!enabled() || box.hidden) return;
    const t = token();
    const copy = ghost(box, 'oa-menu-ghost');
    copy.style.left = box.style.left;
    copy.style.top = box.style.top;
    copy.scrollTop = box.scrollTop;
    box.after(copy);
    removeWhenDone(copy, copy.animate([{ opacity: getComputedStyle(box).opacity }, { opacity: 0 }], { duration: t.exit * 0.8, easing: t.in, fill: 'forwards' }));
  }

  // Toasts rise in and fade out; a new message during the exit takes over.
  function toastShown(el) {
    const t = token();
    play(el, [{ opacity: 0, translate: '0 8px' }, { opacity: 1, translate: '0 0' }], { duration: t.base, easing: t.out });
  }
  function toastHide(el) {
    const t = token();
    const animation = play(el, [{ opacity: 1 }, { opacity: 0 }], { duration: t.exit, easing: t.in, fill: 'forwards' });
    if (!animation) { el.hidden = true; return; }
    animation.finished.then(() => { el.hidden = true; animation.cancel(); }, () => {});
  }

  // Pages: a new destination settles in; switching archive tabs is lateral
  // and only cross-fades.
  function pageShown(main, lateral) {
    const t = token();
    if (lateral) play(main, [{ opacity: 0.4 }, { opacity: 1 }], { duration: t.quick, easing: t.out });
    else play(main, [{ opacity: 0, translate: '0 6px' }, { opacity: 1, translate: '0 0' }], { duration: t.base, easing: t.out });
  }

  // Changed wording (not a changed number) cross-fades so the reader notices it.
  const lastText = new WeakMap();
  function text(el, value) {
    if (el.textContent === value) return;
    const shape = value.replace(/\d+/g, '#');
    const changed = lastText.has(el) && lastText.get(el) !== shape;
    lastText.set(el, shape);
    el.textContent = value;
    if (changed) play(el, [{ opacity: 0.25 }, { opacity: 1 }], { duration: token().base, easing: token().out }, 'text');
  }

  // Brief attention for a message that just appeared, such as a form error.
  function attention(el) {
    if (!el?.textContent) return;
    play(el, [{ opacity: 0, translate: '0 -3px' }, { opacity: 1, translate: '0 0' }], { duration: token().base, easing: token().out }, 'attention');
  }

  // Images that just finished loading fade in instead of popping.
  function fadeIn(el) {
    play(el, [{ opacity: 0 }, { opacity: 1 }], { duration: token().quick, easing: token().out });
  }

  // Reorder or remove items in a row: survivors glide from where they were.
  function flip(container, selector, key, mutate) {
    if (!enabled() || !container?.isConnected || container.hidden) { mutate(); return; }
    const before = new Map();
    for (const item of container.querySelectorAll(selector)) before.set(item.getAttribute(key), item.getBoundingClientRect());
    mutate();
    if (!before.size) return;
    const t = token();
    for (const item of container.querySelectorAll(selector)) {
      const old = before.get(item.getAttribute(key));
      if (!old) {
        play(item, [{ opacity: 0, scale: '.94' }, { opacity: 1, scale: '1' }], { duration: t.base, easing: t.out });
        continue;
      }
      const now = item.getBoundingClientRect();
      const dx = old.left - now.left;
      if (Math.abs(dx) >= 1) play(item, [{ translate: `${dx}px 0` }, { translate: '0 0' }], { duration: t.base, easing: t.out });
    }
  }

  // An element whose box changes size (the task card) grows or shrinks from
  // its anchored corner. Width/height never animate: the larger state is
  // clipped on the compositor (clip-path), so a blurred panel stays cheap.
  function resize(el, mutate) {
    if (!enabled() || !el?.isConnected || el.hidden) { mutate(); return; }
    const before = el.getBoundingClientRect();
    const shrinking = ghost(el, null, true);
    mutate();
    if (el.hidden) return;
    const after = el.getBoundingClientRect();
    if (Math.abs(before.width - after.width) < 1 && Math.abs(before.height - after.height) < 1) return;
    const t = token();
    const radius = getComputedStyle(el).borderRadius;
    const inset = (outer, inner) => `inset(${inner.top - outer.top}px ${outer.right - inner.right}px ${outer.bottom - inner.bottom}px ${inner.left - outer.left}px round ${radius})`;
    const growing = after.width * after.height >= before.width * before.height;
    if (growing) {
      play(el, [{ clipPath: inset(after, before) }, { clipPath: inset(after, after) }], { duration: t.base, easing: t.out }, 'size');
      return;
    }
    // Shrinking: the previous, larger card is clipped down onto the new one.
    shrinking.style.position = 'fixed';
    shrinking.style.inset = 'auto';
    shrinking.style.left = before.left + 'px';
    shrinking.style.top = before.top + 'px';
    shrinking.style.width = before.width + 'px';
    shrinking.style.height = before.height + 'px';
    shrinking.style.margin = '0';
    el.after(shrinking);
    const animation = shrinking.animate([{ clipPath: inset(before, before), opacity: 1 }, { clipPath: inset(before, after), opacity: 1 }],
      { duration: t.exit, easing: t.in, fill: 'forwards' });
    removeWhenDone(shrinking, animation);
  }

  // Disclosures open and close in place. The observer reacts to the hidden
  // attribute, so call sites only change state.
  const DISCLOSURES = '#oa-password-panel,#oa-compression-advanced,#oa-task-content,.oa-password-choices,#oa-password-unsupported,#oa-compression-preview';
  // Elements that only fade: in-flow chrome whose layout change is not the point.
  const FADES = '#oa-task-output,#oa-task-dismiss,#oa-task-cancel,#oa-toolbar,.oa-tree-section,#oa-archive-tabs';
  const POPOVERS = '#oa-task-panel';
  const collapsing = new WeakMap();
  const sizeProps = ['height', 'paddingTop', 'paddingBottom', 'marginTop', 'marginBottom'];
  function measured(el) {
    const style = getComputedStyle(el);
    return Object.fromEntries(sizeProps.map(prop => [prop, style[prop]]));
  }
  const closed = { height: '0px', paddingTop: '0px', paddingBottom: '0px', marginTop: '0px', marginBottom: '0px' };
  function reveal(el) {
    const t = token();
    const copy = collapsing.get(el);
    let from = { ...closed, opacity: 0 };
    if (copy) {
      // Reopened mid-close: continue from the copy's current size.
      from = { ...measured(copy), opacity: getComputedStyle(copy).opacity };
      collapsing.delete(el);
      copy.remove();
    }
    const to = { ...measured(el), opacity: 1 };
    el.style.overflow = 'hidden';
    const animation = play(el, [from, to], { duration: t.base, easing: t.out }, 'disclosure');
    const release = () => { el.style.overflow = ''; };
    animation ? animation.finished.then(release, release) : release();
  }
  function conceal(el) {
    const t = token();
    const opening = running.get(el)?.get('disclosure');
    const copy = ghost(el);
    copy.style.overflow = 'hidden';
    el.after(copy);
    let from = { ...measured(copy), opacity: 1 };
    if (opening) {
      // Closed mid-open: start from where the opening had reached.
      const progress = opening.effect.getComputedTiming().progress ?? 1;
      from = { ...from, height: parseFloat(from.height) * progress + 'px', opacity: progress };
      opening.cancel();
    }
    collapsing.set(el, copy);
    const animation = copy.animate([from, { ...closed, opacity: 0 }], { duration: t.exit, easing: t.in, fill: 'forwards' });
    animation.finished.then(() => { if (collapsing.get(el) === copy) collapsing.delete(el); copy.remove(); }, () => {});
  }
  function hiddenChanged(el) {
    if (!enabled() || insideEntrance(el)) return;
    const t = token();
    if (el.matches(POPOVERS)) {
      if (!el.hidden) enter(el, [{ opacity: 0, translate: '0 10px', scale: '.98' }, { opacity: 1, translate: '0 0', scale: '1' }], { duration: t.base, easing: t.out });
      return;
    }
    if (el.matches(FADES)) {
      if (!el.hidden) play(el, [{ opacity: 0 }, { opacity: 1 }], { duration: t.base, easing: t.out });
      return;
    }
    if (el.hidden) conceal(el); else reveal(el);
  }

  // A segmented selection indicator that slides to the chosen option.
  const GROUPS = [['.qa-theme-switch', 'button[aria-pressed=true]'], ['.oa-nav', 'button[aria-current=true]']];
  const thumbs = new WeakMap();
  function placeThumb(group, selector, animate) {
    if (forcedQuery.matches) return;
    let thumb = thumbs.get(group);
    if (!thumb) {
      thumb = document.createElement('span');
      thumb.className = 'qa-motion-thumb';
      thumb.setAttribute('aria-hidden', 'true');
      group.prepend(thumb);
      group.classList.add('qa-has-thumb');
      thumbs.set(group, thumb);
      animate = false;
    }
    const selected = group.querySelector(selector);
    if (!selected || !selected.offsetWidth) { thumb.style.opacity = '0'; return; }
    const wasVisible = thumb.style.opacity !== '0';
    if (!animate || !wasVisible || !enabled()) thumb.style.transition = 'none';
    thumb.style.transform = `translate(${selected.offsetLeft}px, ${selected.offsetTop}px)`;
    thumb.style.width = selected.offsetWidth + 'px';
    thumb.style.height = selected.offsetHeight + 'px';
    thumb.style.borderRadius = getComputedStyle(selected).borderRadius;
    thumb.style.opacity = '1';
    if (thumb.style.transition === 'none') { void thumb.offsetWidth; thumb.style.transition = ''; }
  }
  function placeThumbs(animate) {
    for (const [groupSelector, selector] of GROUPS) {
      for (const group of document.querySelectorAll(groupSelector)) placeThumb(group, selector, animate);
    }
  }

  // Colour scheme changes cross-fade instead of flashing.
  function crossfade(change) {
    if (!enabled() || typeof document.startViewTransition !== 'function') { change(); return; }
    document.startViewTransition(change);
  }

  function observe(root) {
    const watched = DISCLOSURES + ',' + FADES + ',' + POPOVERS;
    new MutationObserver(records => {
      let thumbsChanged = false;
      // Re-setting an unchanged attribute still produces a record; compare the
      // first old value with the final state and act once per element.
      const wasHidden = new Map();
      for (const record of records) {
        const el = record.target;
        if (record.attributeName === 'hidden') {
          if (!wasHidden.has(el) && el.matches?.(watched)) wasHidden.set(el, record.oldValue !== null);
        } else if (record.oldValue !== el.getAttribute(record.attributeName) && el.closest?.('.qa-theme-switch,.oa-nav')) {
          thumbsChanged = true;
        }
      }
      for (const [el, hidden] of wasHidden) if (hidden !== el.hidden) hiddenChanged(el);
      if (thumbsChanged) placeThumbs(true);
    }).observe(root, { subtree: true, attributes: true, attributeOldValue: true, attributeFilter: ['hidden', 'aria-pressed', 'aria-current'] });
    placeThumbs(false);
    new ResizeObserver(() => placeThumbs(false)).observe(root);
    forcedQuery.addEventListener('change', () => {
      if (!forcedQuery.matches) { placeThumbs(false); return; }
      for (const group of document.querySelectorAll('.qa-has-thumb')) {
        thumbs.get(group)?.remove();
        thumbs.delete(group);
        group.classList.remove('qa-has-thumb');
      }
    });
  }

  const host = document.getElementById('qing-archive');
  if (host) observe(host);

  window.QingMotion = {
    enabled, dialogShown, dialogClosing, menuShown, menuClosing, toastShown, toastHide,
    pageShown, text, attention, fadeIn, flip, resize, crossfade, observe,
  };
})();
