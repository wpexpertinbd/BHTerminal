import { useCallback, useEffect, useRef, useState } from "react";

/** Widths of the two side panes, in pixels, remembered per computer.
 *
 *  The defaults are what the app shipped with, so nothing moves until the
 *  user drags something. Browser storage can throw or come back empty, so a
 *  failed read just means "use the default" — never a broken layout. */
const DEFAULTS = { sidebar: 250, files: 320 };
const LIMITS = { sidebar: [170, 520], files: [220, 680] } as const;
const KEY = "bhterminal.paneWidths";
const COLLAPSED_KEY = "bhterminal.paneCollapsed";

export type PaneName = keyof typeof DEFAULTS;

function clamp(name: PaneName, value: number): number {
  const [min, max] = LIMITS[name];
  // Never let a pane eat the terminal: leave at least 320px for it whatever
  // the window size, so a drag cannot paint the user into a corner.
  const roomy = Math.min(max, Math.max(min, window.innerWidth - 320));
  return Math.round(Math.min(roomy, Math.max(min, value)));
}

function load(): typeof DEFAULTS {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return { ...DEFAULTS };
    const saved = JSON.parse(raw) as Partial<typeof DEFAULTS>;
    return {
      sidebar: clamp("sidebar", Number(saved.sidebar) || DEFAULTS.sidebar),
      files: clamp("files", Number(saved.files) || DEFAULTS.files),
    };
  } catch {
    return { ...DEFAULTS };
  }
}

function loadCollapsed(): { sidebar: boolean; files: boolean } {
  try {
    const raw = localStorage.getItem(COLLAPSED_KEY);
    if (!raw) return { sidebar: false, files: false };
    const saved = JSON.parse(raw) as Partial<{ sidebar: boolean; files: boolean }>;
    return { sidebar: Boolean(saved.sidebar), files: Boolean(saved.files) };
  } catch {
    return { sidebar: false, files: false };
  }
}

export function useResizablePanes() {
  const [widths, setWidths] = useState(load);
  /** Collapsed panes keep their WIDTH, so expanding puts them back exactly
   *  where they were rather than at the default. */
  const [collapsed, setCollapsed] = useState(loadCollapsed);
  const dragging = useRef<PaneName | null>(null);

  useEffect(() => {
    try {
      localStorage.setItem(KEY, JSON.stringify(widths));
    } catch {
      /* the size still applies to this run */
    }
  }, [widths]);

  useEffect(() => {
    try {
      localStorage.setItem(COLLAPSED_KEY, JSON.stringify(collapsed));
    } catch {
      /* the state still applies to this run */
    }
  }, [collapsed]);

  useEffect(() => {
    const move = (e: MouseEvent) => {
      const name = dragging.current;
      if (!name) return;
      // Measured from the window edge the pane is attached to, so the handle
      // stays under the pointer however far it is dragged.
      const next = name === "sidebar" ? e.clientX : window.innerWidth - e.clientX;
      setWidths((w) => ({ ...w, [name]: clamp(name, next) }));
      e.preventDefault();
    };
    const stop = () => {
      if (!dragging.current) return;
      dragging.current = null;
      document.body.classList.remove("resizing");
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", stop);
    // A drag that ends outside the window must not leave the layout stuck to
    // the pointer.
    window.addEventListener("blur", stop);
    return () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", stop);
      window.removeEventListener("blur", stop);
    };
  }, []);

  // A narrower window can leave a pane wider than the room available.
  useEffect(() => {
    const onResize = () =>
      setWidths((w) => ({ sidebar: clamp("sidebar", w.sidebar), files: clamp("files", w.files) }));
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  const startDrag = useCallback((name: PaneName) => {
    dragging.current = name;
    // While dragging, the cursor stays a resize arrow and nothing selects.
    document.body.classList.add("resizing");
  }, []);

  const reset = useCallback((name: PaneName) => {
    setWidths((w) => ({ ...w, [name]: DEFAULTS[name] }));
  }, []);

  const toggleCollapsed = useCallback((name: PaneName) => {
    setCollapsed((c) => ({ ...c, [name]: !c[name] }));
  }, []);

  return { widths, collapsed, startDrag, reset, toggleCollapsed };
}
