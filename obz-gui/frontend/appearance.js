(() => {
  'use strict';

  const modes = ['system', 'light', 'dark'];
  const scheme = window.matchMedia('(prefers-color-scheme: dark)');
  const contrast = window.matchMedia('(forced-colors: active)');

  function preferredMode(value) {
    return modes.includes(value) ? value : 'system';
  }

  function bind(readPreference, paint, onError) {
    let pending = Promise.resolve();
    let revision = 0;
    let lastFallback = '';

    function update() {
      const request = ++revision;
      const { theme, material } = readPreference();
      const requestedMaterial = contrast.matches ? 'solid' : material;
      pending = pending.then(async () => {
        if (request !== revision) return;
        try {
          const effective = await window.__TAURI__.core.invoke('set_appearance', {
            theme, material: requestedMaterial,
          });
          if (request !== revision) return;
          paint(effective.dark ? 'dark' : 'light', effective.material);
          const fallback = requestedMaterial !== effective.material ? requestedMaterial : '';
          if (fallback && fallback !== lastFallback) onError('当前 Windows 版本不支持此窗口材质，已使用纯色。');
          lastFallback = fallback;
        } catch (error) {
          if (request !== revision) return;
          onError('窗口外观更新失败：' + error);
        }
      });
    }

    const { theme, material } = readPreference();
    paint(theme === 'system' ? (scheme.matches ? 'dark' : 'light') : theme, contrast.matches ? 'solid' : material);
    contrast.addEventListener('change', update);
    window.__TAURI__.event.listen('tauri://theme-changed', () => {
      if (readPreference().theme === 'system') update();
    }).catch(onError);
    return update;
  }

  window.QingAppearance = { bind, preferredMode };
})();
