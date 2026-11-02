// A button that opens a menu below it; closes on a choice, a click outside or Escape.
import { html, useEffect, useRef, useState } from "../lib/html.js";

export function Menu({ label, title, cls = "", children, align = "left", id }) {
  const [open, setOpen] = useState(false);
  const ref = useRef(null);
  useEffect(() => {
    if (!open) return;
    const off = (e) => { if (!ref.current?.contains(e.target)) setOpen(false); };
    const esc = (e) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", off);
    document.addEventListener("keydown", esc);
    return () => { document.removeEventListener("mousedown", off); document.removeEventListener("keydown", esc); };
  }, [open]);
  return html`<span class="anchor" ref=${ref}>
    <button type="button" id=${id} class=${cls} title=${title} aria-haspopup="menu" aria-expanded=${open ? "true" : "false"} onClick=${() => setOpen(!open)}>${label}</button>
    ${open && html`<div class="menu" role="menu" style=${`top:calc(100% + 4px);${align === "right" ? "right:0" : "left:0"}`} onClick=${(e) => e.target.closest("button") && setOpen(false)}>${children}</div>`}
  </span>`;
}
