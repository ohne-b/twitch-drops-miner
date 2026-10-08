(async () => {
  const invoke = window.__TAURI_INTERNALS__.invoke;
  const wait = async (check, label) => {
    const end = Date.now() + 20000;
    while (!check()) {
      if (Date.now() > end) throw new Error(label);
      await new Promise((resolve) => setTimeout(resolve, 50));
    }
  };
  try {
    await wait(() => document.querySelector('[aria-label="Pause mining"]'), 'snapshot not rendered');
    await wait(() => document.title.startsWith('70% Rust'), 'confirmed title missing');
    document.querySelector('[aria-label="Pause mining"]').click();
    await wait(() => document.title.startsWith('Paused'), 'pause not published');
    document.querySelector('[aria-label="Resume mining"]').click();
    await wait(() => document.title.startsWith('70% Rust'), 'resume not published');
    document.querySelector('a[href="/settings"]').click();
    await wait(() => document.querySelector('a[href="/settings#desktop"]'), 'desktop settings missing');
    if (document.querySelector('a[href="/settings#access"]')) throw new Error('server auth exposed');
    document.querySelector('a[href="/settings#desktop"]').click();
    await wait(() => document.body.textContent.includes('Start when I sign in'), 'preferences not rendered');
    window.dispatchEvent(new Event('desktop-update-open'));
    await wait(() => document.querySelector('dialog[open]'), 'updater dialog not opened');
    await wait(() => document.querySelector('dialog[open]').textContent.includes('You are up to date'), 'offline updater status not rendered');
    await invoke('smoke_result', { error: null });
  } catch (error) {
    await invoke('smoke_result', { error: String(error) });
  }
})();
