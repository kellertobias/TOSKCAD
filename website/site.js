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

const preview = document.querySelector('.image-preview');
const imageLinks = [...document.querySelectorAll('.application a[href]')].filter(link => link.querySelector('img'));
const previewImage = preview.querySelector('img');
const caption = preview.querySelector('#preview-caption');
const zoom = preview.querySelector('[data-preview="zoom"]');
let imageIndex = 0;
let opener;
function showImage(index) {
  imageIndex = (index + imageLinks.length) % imageLinks.length;
  const link = imageLinks[imageIndex];
  const image = link.querySelector('img');
  previewImage.src = link.href;
  previewImage.alt = image.alt;
  caption.textContent = (link.querySelector('span')?.textContent || link.closest('figure').querySelector('h3').textContent).replace(/ ↗$/, '');
  preview.classList.toggle('is-drawing', new URL(link.href).pathname.endsWith('.svg'));
  preview.classList.remove('is-zoomed');
  zoom.setAttribute('aria-pressed', 'false');
  preview.querySelector('.preview-image').scrollTo(0, 0);
}
for (const [index, link] of imageLinks.entries()) {
  link.addEventListener('click', event => {
    if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    opener = link;
    showImage(index);
    preview.showModal();
    document.documentElement.style.overflow = 'hidden';
  });
}
preview.querySelector('[data-preview="close"]').addEventListener('click', () => preview.close());
preview.querySelector('[data-preview="previous"]').addEventListener('click', () => showImage(imageIndex - 1));
preview.querySelector('[data-preview="next"]').addEventListener('click', () => showImage(imageIndex + 1));
zoom.addEventListener('click', () => {
  const enlarged = preview.classList.toggle('is-zoomed');
  zoom.setAttribute('aria-pressed', String(enlarged));
});
preview.addEventListener('click', event => {
  if (event.target === preview) preview.close();
});
preview.addEventListener('keydown', event => {
  if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
    event.preventDefault();
    showImage(imageIndex + (event.key === 'ArrowRight' ? 1 : -1));
  }
});
preview.addEventListener('close', () => {
  document.documentElement.style.overflow = '';
  opener?.focus({ preventScroll: true });
});
