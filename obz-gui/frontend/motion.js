// Motion for state changes. CSS handles colour and transform transitions on
// controls (motion.css); this module choreographs what CSS cannot: things that
// appear, disappear, move or change size. Everything here is purely visual —
// application state (hidden, content, focus) always changes synchronously, and
// exit effects use empty, inert decoration. Durations and curves come from the CSS tokens in ui.css.
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
  const enabled = () => !reduceQuery.matches && !forcedQuery.matches &&
    typeof Element.prototype.animate === 'function';

  const running = new WeakMap();
  const active = new Set();
  const entering = new Map();
  function stop(el, channel = 'main') {
    running.get(el)?.get(channel)?.finish(true);
  }
  function play(el, frames, options, channel = 'main', onFinish) {
    stop(el, channel);
    if (!enabled() || !el?.isConnected) return null;
    let channels = running.get(el);
    if (!channels) running.set(el, channels = new Map());
    const animation = el.animate(frames, { fill: 'none', ...options });
    const entry = {
      finish(cancelled) {
        if (!active.delete(entry)) return;
        channels.delete(channel);
        if (entering.get(el) === entry) entering.delete(el);
        onFinish?.(cancelled);
        if (cancelled || options.fill === 'forwards') animation.cancel();
      },
    };
    channels.set(channel, entry);
    active.add(entry);
    animation.finished.then(() => entry.finish(false), () => entry.finish(true));
    return entry;
  }
  function settleMotion() {
    if (!enabled()) for (const entry of active) entry.finish(true);
  }
  reduceQuery.addEventListener('change', settleMotion);
  forcedQuery.addEventListener('change', settleMotion);

  function enter(el, frames, options) {
    const entry = play(el, frames, options);
    if (entry) entering.set(el, entry);
  }
  function insideEntrance(el) {
    for (let host = el.parentElement; host; host = host.parentElement) {
      if (entering.has(host)) return true;
    }
    return false;
  }

  const exits = new WeakMap();
  function clearExit(el) {
    const decoration = exits.get(el);
    if (decoration) stop(decoration);
  }
  function exit(el, className, prepare, duration) {
    clearExit(el);
    if (!enabled() || el.hidden || !el.isConnected) return;
    const opacity = getComputedStyle(el).opacity;
    const decoration = document.createElement('div');
    decoration.className = 'qa-motion-ghost ' + className;
    decoration.inert = true;
    decoration.setAttribute('aria-hidden', 'true');
    prepare?.(decoration);
    el.before(decoration);
    const remove = () => { decoration.remove(); exits.delete(el); };
    if (play(decoration, [{ opacity }, { opacity: 0 }], {
      duration, easing: token().in, fill: 'forwards',
    }, 'main', remove)) exits.set(el, decoration);
    else remove();
  }

  // Keep modal contents and menu actions out of exit decorations.
  function dialogShown(overlay, wasOpen) {
    clearExit(overlay);
    if (!enabled()) return;
    const sheet = overlay.firstElementChild;
    if (!sheet) return;
    const t = token();
    if (wasOpen) {
      play(sheet, [{ opacity: 0.8, translate: '0 4px' }, { opacity: 1, translate: '0 0' }], { duration: t.quick, easing: t.out });
      return;
    }
    enter(overlay, [{ opacity: 0.75 }, { opacity: 1 }], { duration: t.base, easing: t.out });
    play(sheet, [{ opacity: 0.8, translate: '0 8px', scale: '.99' }, { opacity: 1, translate: '0 0', scale: '1' }], { duration: t.sheet, easing: t.out });
  }
  function dialogClosing(overlay) {
    if (overlay.hidden || !overlay.firstElementChild) return;
    if (enabled()) exit(overlay, 'qa-overlay-ghost', null, token().exit);
    stop(overlay);
    stop(overlay.firstElementChild);
  }

  const menuDepth = new WeakMap();
  function menuPreparing(box) {
    clearExit(box);
    stop(box);
  }
  function menuShown(box, anchorRect, wasOpen) {
    if (!enabled()) return;
    const t = token();
    const depth = box.querySelector('.oa-submenu-head') ? 1 : 0;
    const previous = menuDepth.get(box) ?? depth;
    menuDepth.set(box, depth);
    if (wasOpen) {
      const direction = Math.sign(depth - previous);
      play(box, [{ opacity: 0.8, translate: `${direction * 8}px 0` }, { opacity: 1, translate: '0 0' }], { duration: t.quick, easing: t.out });
      return;
    }
    const bounds = box.getBoundingClientRect();
    const originX = anchorRect ? Math.max(0, Math.min(bounds.width, anchorRect.left + (anchorRect.width || 0) / 2 - bounds.left)) : 0;
    const originY = anchorRect && anchorRect.top > bounds.top ? bounds.height : 0;
    const transformOrigin = `${originX}px ${originY}px`;
    play(box, [{ opacity: 0.8, scale: '.98', transformOrigin }, { opacity: 1, scale: '1', transformOrigin }],
      { duration: t.quick, easing: t.out });
  }
  function menuClosing(box) {
    if (box.hidden) return;
    if (enabled()) exit(box, 'oa-menu-ghost', decoration => {
      const bounds = box.getBoundingClientRect();
      Object.assign(decoration.style, {
        left: bounds.left + 'px', top: bounds.top + 'px',
        width: bounds.width + 'px', height: bounds.height + 'px',
      });
    }, token().exit * 0.8);
    stop(box);
  }

  // A new message always interrupts a fade in progress.
  function toastCleared(el) {
    stop(el);
    el.hidden = true;
  }
  function toastShown(el, wasHidden) {
    stop(el);
    if (!wasHidden) return;
    const t = token();
    play(el, [{ opacity: 0, translate: '0 8px' }, { opacity: 1, translate: '0 0' }], { duration: t.base, easing: t.out });
  }
  function toastHide(el) {
    if (el.hidden) return;
    const t = token();
    const animation = play(el, [{ opacity: 1 }, { opacity: 0 }], {
      duration: t.exit, easing: t.in, fill: 'forwards',
    }, 'main', cancelled => { if (!cancelled || !enabled()) el.hidden = true; });
    if (!animation) el.hidden = true;
  }

  // Pages: a new destination settles in; switching archive tabs is lateral
  // and only cross-fades.
  function pageShown(main, lateral) {
    const t = token();
    if (lateral) play(main, [{ opacity: 0.85 }, { opacity: 1 }], { duration: t.quick, easing: t.out });
    else play(main, [{ opacity: 0.85, translate: '0 6px' }, { opacity: 1, translate: '0 0' }], { duration: t.base, easing: t.out });
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
    const items = Array.from(container.querySelectorAll(selector), item =>
      [item, before.get(item.getAttribute(key)), item.getBoundingClientRect()]);
    for (const [item, old, now] of items) {
      if (!old) {
        play(item, [{ opacity: 0.8, scale: '.97' }, { opacity: 1, scale: '1' }], { duration: t.base, easing: t.out });
        continue;
      }
      const dx = old.left - now.left;
      if (Math.abs(dx) >= 1) play(item, [{ translate: `${dx}px 0` }, { translate: '0 0' }], { duration: t.base, easing: t.out });
    }
  }

  // The task card settles into its new size with the live controls in place.
  function resize(el, mutate) {
    if (!enabled() || !el?.isConnected || el.hidden) { mutate(); return; }
    stop(el, 'size');
    const before = el.getBoundingClientRect();
    mutate();
    if (el.hidden) return;
    const after = el.getBoundingClientRect();
    if (Math.abs(before.width - after.width) < 1 && Math.abs(before.height - after.height) < 1) return;
    const t = token();
    play(el, [{ opacity: 0.85, translate: '0 3px' }, { opacity: 1, translate: '0 0' }],
      { duration: t.quick, easing: t.out }, 'size');
  }

  const DISCLOSURES = '#oa-password-panel,#oa-compression-advanced,#oa-task-content,.oa-password-choices,#oa-password-unsupported,#oa-compression-preview';
  const FADES = '#oa-task-output,#oa-task-dismiss,#oa-task-cancel,#oa-toolbar,.oa-tree-section,#oa-archive-tabs';
  const POPOVERS = '#oa-task-panel';
  function hiddenChanged(el) {
    if (el.hidden) {
      stop(el);
      stop(el, 'size');
      return;
    }
    if (!enabled() || insideEntrance(el)) return;
    const t = token();
    const options = { duration: t.base, easing: t.out };
    const still = { opacity: 1, translate: '0 0' };
    if (el.matches(POPOVERS)) {
      enter(el, [{ opacity: 0.8, translate: '0 6px' }, still], options);
    } else if (el.matches(FADES)) {
      play(el, [{ opacity: 0.8 }, { opacity: 1 }], options);
    } else {
      play(el, [{ opacity: 0.8, translate: '0 -3px' }, still], options);
    }
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

  function themeChanged(group) {
    if (!enabled()) return;
    const t = token();
    play(group, [{ opacity: 0.85 }, { opacity: 1 }], { duration: t.quick, easing: t.out });
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
      for (const [el, hidden] of wasHidden) if (el.isConnected && hidden !== el.hidden) hiddenChanged(el);
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
    enabled, dialogShown, dialogClosing, menuPreparing, menuShown, menuClosing, toastCleared, toastShown, toastHide,
    pageShown, text, attention, fadeIn, flip, resize, themeChanged,
  };
})();
