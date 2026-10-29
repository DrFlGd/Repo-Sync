// Inline line icons (no icon font, no network), drawn for Drafting Sheet's fine square-ended lines.
import { html } from "../lib/html.js";

const svg = (d, size = 16) => html`<svg class="icon" width=${size} height=${size} viewBox="0 0 24 24" fill="none" stroke="currentColor" aria-hidden="true">${d}</svg>`;

const D = {
  library: html`<path d="M4 4h4v16H4zM10 4h4v16h-4z" /><path d="M15.2 5.2l3.8-1 3.4 14.6-3.8 1z" />`,
  activity: html`<path d="M2 12h4l3-7 4 14 3-7h6" />`,
  cog: html`<circle cx="12" cy="12" r="6.5" /><circle cx="12" cy="12" r="2.5" /><path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9 7 7M17 17l2.1 2.1M4.9 19.1 7 17M17 7l2.1-2.1" />`,
  info: html`<circle cx="12" cy="12" r="9" /><path d="M12 11v6M12 7v1.5" />`,
  plus: html`<path d="M12 4v16M4 12h16" />`,
  refresh: html`<path d="M20 12a8 8 0 1 1-2.3-5.7" /><path d="M20 4v5h-5" />`,
  play: html`<path d="M7 4.5v15l12-7.5z" />`,
  folder: html`<path d="M3 6h6l2 2h10v11H3z" /><path d="M3 11h18" stroke-dasharray="1.5 1.5" />`,
  external: html`<path d="M14 4h6v6M20 4l-9 9" /><path d="M18 14v6H4V6h6" />`,
  search: html`<circle cx="10.5" cy="10.5" r="6.5" /><path d="M15.2 15.2 21 21" />`,
  grid: html`<path d="M4 4h7v7H4zM13 4h7v7h-7zM4 13h7v7H4zM13 13h7v7h-7z" />`,
  list: html`<path d="M4 6h16M4 12h16M4 18h16" />`,
  trash: html`<path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13" />`,
  down: html`<path d="M6 9l6 6 6-6" />`,
  x: html`<path d="M6 6l12 12M18 6 6 18" />`,
  star: html`<path d="M12 3.5l2.6 5.4 5.9.8-4.3 4.1 1.1 5.8L12 16.8l-5.3 2.8 1.1-5.8-4.3-4.1 5.9-.8z" />`,
  tag: html`<path d="M3 3h8l10 10-8 8L3 11z" /><circle cx="7.5" cy="7.5" r="1.5" />`,
  branch: html`<circle cx="6" cy="5" r="2" /><circle cx="6" cy="19" r="2" /><circle cx="18" cy="7" r="2" /><path d="M6 7v10M18 9c0 5-6 4-11.5 8.5" />`,
  box: html`<path d="M12 3l8.5 4.5v9L12 21l-8.5-4.5v-9z" /><path d="M3.5 7.5 12 12l8.5-4.5M12 12v9" />`,
  code: html`<path d="M8 7l-5 5 5 5M16 7l5 5-5 5M14 4l-4 16" />`,
  back: html`<path d="M15 5l-7 7 7 7" />`,
  dots: html`<circle cx="5" cy="12" r="1.2" /><circle cx="12" cy="12" r="1.2" /><circle cx="19" cy="12" r="1.2" />`,
  check: html`<path d="M4 12.5l5 5L20 6.5" />`,
  alert: html`<path d="M12 3 2 20h20z" /><path d="M12 10v5M12 17v1.5" />`,
  clock: html`<circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" />`,
  sun: html`<circle cx="12" cy="12" r="4" /><path d="M12 2v3M12 19v3M2 12h3M19 12h3M5 5l2 2M17 17l2 2M5 19l2-2M17 7l2-2" />`,
  moon: html`<path d="M20 14.5A8 8 0 0 1 9.5 4 8 8 0 1 0 20 14.5z" />`,
  auto: html`<circle cx="12" cy="12" r="8" /><path d="M12 4v16" /><path d="M12 4a8 8 0 0 1 0 16z" fill="currentColor" stroke="none" />`,
};

export const Icon = Object.fromEntries(Object.entries(D).map(([k, d]) => [k, (size) => svg(d, size)]));

/** The app's mark: shelves of kept versions with a sync arrow. */
export const Mark = () => html`<svg viewBox="0 0 40 40" aria-hidden="true" fill="none">
  <rect x="1.5" y="1.5" width="37" height="37" stroke="var(--ink)" stroke-width="1.4" />
  <g stroke="var(--tool)" stroke-width="1.8"><rect x="8" y="9" width="15" height="5" /><rect x="8" y="17.5" width="15" height="5" /><rect x="8" y="26" width="15" height="5" /></g>
  <path d="M28 12.5a8 8 0 0 1 0 15" stroke="var(--mark)" stroke-width="2" /><path d="M24.5 26.5l4.2 2.8 1.3-4.6z" fill="var(--mark)" />
  <path d="M6 36h28M6 34.6v2.8M34 34.6v2.8" stroke="var(--ink)" stroke-width=".8" />
</svg>`;
