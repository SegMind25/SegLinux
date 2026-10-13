// Shared appearance preferences for the website and documentation.
(() => {
  const root = document.documentElement;
  const systemMotion = matchMedia('(prefers-reduced-motion: reduce)');
  let userReduced = false;
  try {
    const theme = localStorage.getItem('seglinux-theme');
    if (theme === 'dark' || theme === 'light') root.dataset.theme = theme;
    userReduced = localStorage.getItem('seglinux-reduced-motion') === 'true';
  } catch {}
  function motion() {
    root.dataset.motion = systemMotion.matches || userReduced ? 'reduced' : 'full';
    window.dispatchEvent(new CustomEvent('seglinux-motion-change'));
    const button = document.getElementById('motion-toggle');
    if (button) {
      button.setAttribute('aria-pressed', String(root.dataset.motion === 'reduced'));
      button.disabled = systemMotion.matches;
      button.title = systemMotion.matches ? 'Reduced motion is enabled in your system settings' : 'Reduce website animations';
    }
  }
  motion();
  systemMotion.addEventListener('change', motion);
  document.addEventListener('DOMContentLoaded', () => {
    const right = document.querySelector('.nav-right');
    if (!right) return;
    let theme = document.getElementById('theme-toggle');
    if (!theme) {
      theme = document.createElement('button'); theme.id = 'theme-toggle'; theme.className = 'nav-icon';
      theme.type = 'button'; theme.textContent = '◐'; theme.setAttribute('aria-label', 'Switch between dark and light'); right.prepend(theme);
    }
    const syncTheme = () => {
      theme.setAttribute('aria-pressed', String(root.dataset.theme === 'light'));
      const meta = document.querySelector('meta[name="theme-color"]');
      if (meta) meta.content = root.dataset.theme === 'light' ? '#f3f2ee' : '#161719';
    };
    syncTheme();
    theme.addEventListener('click', () => {
      root.dataset.theme = root.dataset.theme === 'light' ? 'dark' : 'light'; syncTheme();
      try { localStorage.setItem('seglinux-theme', root.dataset.theme); } catch {}
    });
    const button = document.createElement('button'); button.id = 'motion-toggle'; button.type = 'button';
    button.className = 'nav-icon'; button.textContent = 'Ⅱ'; button.setAttribute('aria-label', 'Reduce website animations');
    theme.after(button);
    button.addEventListener('click', () => {
      userReduced = !userReduced;
      try { localStorage.setItem('seglinux-reduced-motion', String(userReduced)); } catch {}
      motion();
    });
    motion();
  });
})();
