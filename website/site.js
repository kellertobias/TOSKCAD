const controls = document.querySelector('.gallery-controls');
const shots = [...document.querySelectorAll('[data-shot]')];
const buttons = [...controls.querySelectorAll('button')];
function show(view) {
  for (const shot of shots) {
    shot.hidden = shot.dataset.shot !== view;
    shot.classList.toggle('shot-enter', !shot.hidden);
  }
  for (const button of buttons) button.setAttribute('aria-pressed', String(button.dataset.view === view));
}
controls.hidden = false;
show('model');
for (const button of buttons) button.addEventListener('click', () => show(button.dataset.view));
const motion = !matchMedia('(prefers-reduced-motion: reduce)').matches;
if (motion) {
  const targets = document.querySelectorAll('.brand, nav a, .wordmark, h1, .intro p, .action, .section-head h2, .section-head p, .gallery-controls, .development h2, .development-copy p, .resource-links a, footer > *');
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
