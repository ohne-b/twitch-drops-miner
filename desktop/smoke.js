(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const titleMatches = async (prefix) => {
    const status = await invoke('smoke_tray_status');
    return document.title.startsWith(prefix)
      && await invoke('plugin:window|title', { label: 'main' }) === document.title
      && (!status || (status[0] === document.title.replace(/ - Drops Miner$/, '') && status[1] === false));
  };
  const wait = async (check, label) => {
    const end = Date.now() + 20000;
    while (!(await check())) {
      if (Date.now() > end) throw new Error(label);
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
  };
  try {
    await wait(() => document.querySelector('[aria-label="Pause mining"]'), 'snapshot not rendered');
    if ((await invoke('desktop_settings')).tray_available && !await invoke('smoke_tray_status')) {
      throw new Error('tray status row missing');
    }
    const fonts = await document.fonts.load('15px "Manrope Variable"', '\u0462');
    if (!fonts.length) throw new Error('bundled Cyrillic-extended font missing');
    await wait(() => titleMatches('70% Rust'), 'confirmed native title missing');
    document.querySelector('[aria-label="Pause mining"]').click();
    await wait(() => titleMatches('Paused'), 'paused native title missing');
    document.querySelector('[aria-label="Resume mining"]').click();
    await wait(() => titleMatches('70% Rust'), 'resumed native title missing');
    document.querySelector('a[href="/settings"]').click();
    await wait(() => document.querySelector('a[href="/settings#desktop"]'), 'desktop settings missing');
    if (document.querySelector('a[href="/settings#access"]')) throw new Error('server auth exposed');
    document.querySelector('a[href="/settings#desktop"]').click();
    await wait(() => document.body.textContent.includes('Start when I sign in'), 'preferences not rendered');
    const keepAwake = [...document.querySelectorAll('label')]
      .find(label => label.textContent === 'Keep computer awake while mining')?.querySelector('input');
    if (!keepAwake || keepAwake.checked) throw new Error('keep-awake must default off');
    keepAwake.click();
    await wait(async () => (await invoke('desktop_settings')).keep_awake === true, 'keep-awake preference not saved');
    await wait(() => !keepAwake.disabled, 'preference action did not finish');
    keepAwake.click();
    await wait(async () => (await invoke('desktop_settings')).keep_awake === false, 'keep-awake preference not disabled');
    await invoke('plugin:event|emit', { event: 'desktop-keep-awake-failed', payload: true });
    await wait(() => document.body.textContent.includes('Could not prevent automatic sleep'), 'power failure missing');
    await invoke('plugin:event|emit', { event: 'desktop-keep-awake-failed', payload: false });
    await wait(() => !document.body.textContent.includes('Could not prevent automatic sleep'), 'power failure not cleared');
    const originalWidth = window.innerWidth;
    const windows = navigator.userAgent.includes('Windows');
    const mac = navigator.userAgent.includes('Macintosh');
    if (windows) {
      // Synthetic keys cannot trigger WebView2's native accelerators; check its zoom IPC here.
      await invoke('plugin:webview|set_webview_zoom', { value: 1.2 });
    } else {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: '+', ctrlKey: !mac, metaKey: mac }));
    }
    await wait(() => window.innerWidth < originalWidth, 'native zoom did not increase');
    if (windows) {
      await invoke('plugin:webview|set_webview_zoom', { value: 1 });
    } else {
      window.dispatchEvent(new KeyboardEvent('keydown', { key: '0', ctrlKey: !mac, metaKey: mac }));
    }
    await wait(() => Math.abs(window.innerWidth - originalWidth) <= 1, 'native zoom did not reset');
    if (!windows) {
      window.dispatchEvent(new WheelEvent('mousewheel', { ctrlKey: true, deltaY: 120 }));
      await wait(() => window.innerWidth > originalWidth, 'control-wheel zoom did not decrease');
      window.dispatchEvent(new KeyboardEvent('keydown', { key: '0', ctrlKey: !mac, metaKey: mac }));
      await wait(() => Math.abs(window.innerWidth - originalWidth) <= 1, 'wheel zoom did not reset');
    }
    await wait(async () => (await invoke('desktop_update')).phase === 'current', 'startup update check did not finish');
    const checked = await invoke('desktop_update');
    window.dispatchEvent(new Event('desktop-update-open'));
    await wait(() => document.querySelector('dialog[open]'), 'updater dialog not opened');
    await wait(() => document.querySelector('dialog[open]').textContent.includes('You are up to date'), 'offline updater status not rendered');
    await wait(async () => (await invoke('desktop_update')).revision > checked.revision, 'opening updates did not check again');
    const close = () => [...document.querySelectorAll('dialog[open] button')]
      .find(button => button.textContent === 'Close').click();
    close();
    const indicator = () => document.querySelector('aside button[aria-haspopup="dialog"]');
    if (indicator()) throw new Error('update indicator visible without an available update');
    let revision = (await invoke('desktop_update')).revision;
    const publish = (phase, version, error = null) => invoke('plugin:event|emit', {
      event: 'desktop-update',
      payload: { ...checked, revision: ++revision, phase, version, error },
    });
    for (const [phase, text, error] of [
      ['available', 'Version 99.0.0 is available.', null],
      ['downloading', 'Downloading 99.0.0', null],
      ['ready', 'The update is verified', null],
      ['failed', 'Try downloading again.', 'download_failed'],
    ]) {
      await publish(phase, '99.0.0', error);
      await wait(() => indicator()?.textContent === 'Update v99.0.0', 'sidebar update link missing');
      indicator().click();
      await wait(() => document.querySelector('dialog[open]')?.textContent.includes(text), 'update link lost the current phase');
      close();
    }
    await invoke('plugin:window|set_size', {
      label: 'main', value: { Logical: { width: 360, height: 480 } },
    });
    await wait(() => window.innerWidth < 400, 'compact window size not applied');
    document.querySelector('a[href="/activity"]').click();
    await wait(() => document.getElementById('activity-list'), 'activity not rendered');
    const list = document.getElementById('activity-list').getBoundingClientRect();
    const nav = document.querySelector('.primary-nav').getBoundingClientRect();
    if (list.bottom > nav.top) throw new Error('update link pushes activity behind navigation');
    const header = document.querySelector('.brand-row').getBoundingClientRect();
    await publish('available', '99.0.0-preview.with.a.long.version.label');
    await wait(() => indicator()?.textContent.includes('preview.with'), 'long update label missing');
    const link = indicator().getBoundingClientRect();
    if (link.left < header.right || link.right > window.innerWidth || link.bottom > header.bottom) {
      throw new Error('compact update link does not fit beside the app name');
    }
    await publish('current', null);
    await wait(() => !indicator(), 'obsolete update indicator remains visible');
    await invoke('smoke_result', { error: null });
  } catch (error) {
    await invoke('smoke_result', { error: String(error) });
  }
})();
