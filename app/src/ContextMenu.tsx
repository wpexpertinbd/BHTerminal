import { useEffect, useRef } from "react";

/** A separator, or an item. Keeping them as separate shapes means a
 *  divider cannot be given a label and an item cannot forget one. */
export type MenuAction =
  | { divider: true }
  | {
      divider?: false;
      label: string;
      run?: () => void;
      disabled?: boolean;
      /** Shown in red — deleting, mainly. */
      danger?: boolean;
    };

interface Props {
  x: number;
  y: number;
  items: MenuAction[];
  onClose: () => void;
}

/** A right-click menu for the app's own actions.
 *
 *  The webview's native menu is suppressed everywhere except real text
 *  fields (see main.tsx): "Ask Siri", "Translate" and "Search with Google"
 *  over a server's file listing are not this app's menu, and they are what
 *  appears if nothing takes the event. */
export function ContextMenu({ x, y, items, onClose }: Props) {
  const ref = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    // Keep the menu on screen: flip it up or left when it would overflow,
    // which is what a menu opened near the bottom-right corner always does.
    const el = ref.current;
    if (el) {
      const r = el.getBoundingClientRect();
      if (r.bottom > window.innerHeight) el.style.top = `${Math.max(4, y - r.height)}px`;
      if (r.right > window.innerWidth) el.style.left = `${Math.max(4, x - r.width)}px`;
    }
    const dismiss = (e: Event) => {
      if (e instanceof KeyboardEvent) {
        if (e.key === "Escape") onClose();
        return;
      }
      // ⚠️ A press INSIDE the menu belongs to the menu. Closing on it would
      // unmount the button before its click could fire, which makes every
      // item a no-op — mousedown always precedes click.
      if (e.target instanceof Node && ref.current?.contains(e.target)) return;
      onClose();
    };
    // Capture: a click anywhere else closes it before that click lands.
    window.addEventListener("mousedown", dismiss, true);
    window.addEventListener("keydown", dismiss, true);
    window.addEventListener("blur", dismiss);
    return () => {
      window.removeEventListener("mousedown", dismiss, true);
      window.removeEventListener("keydown", dismiss, true);
      window.removeEventListener("blur", dismiss);
    };
  }, [x, y, onClose]);

  return (
    <div className="ctx" style={{ left: x, top: y }} ref={ref} onContextMenu={(e) => e.preventDefault()}>
      {items.map((item, i) =>
        "divider" in item && item.divider ? (
          <hr key={i} />
        ) : (
          <button
            key={i}
            className={"danger" in item && item.danger ? "danger" : ""}
            disabled={"disabled" in item ? item.disabled : false}
            onClick={() => {
              onClose();
              if ("run" in item) item.run?.();
            }}
          >
            {"label" in item ? item.label : ""}
          </button>
        )
      )}
    </div>
  );
}
