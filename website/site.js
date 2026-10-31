const motion = !matchMedia('(prefers-reduced-motion: reduce)').matches;
if (motion) {
  const targets = document.querySelectorAll('.brand, nav a, .wordmark, h1, .intro p, .action, .section-head h2, .section-head p, .workflow h2, .workflow p, .workflow dl > div, .development h2, .development-copy p, .resource-links a, footer > *');
  const observer = new IntersectionObserver(entries => {
    entries.forEach(entry => {
      if (!entry.isIntersecting) return;
      entry.target.classList.add('reveal');
      observer.unobserve(entry.target);
    });
  }, { threshold: 0.1 });
  targets.forEach((target, i) => {
    target.style.setProperty('--delay', `${(i % 4) * 70}ms`);
    observer.observe(target);
  });
}
