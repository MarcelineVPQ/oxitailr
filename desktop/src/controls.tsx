// The settings controls, carried over from the DS5 configurator's controls.tsx
// (Row, Toggle, Segmented): one look for every setting, label on the left,
// control on the right, an optional short hint under the label.
import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { Info } from "lucide-react";
import { useTopLayer } from "./topLayer";

interface RowProps {
  label: string;
  /** A short hint under the label (keep it to one line). */
  sub?: string;
  /** The longer explanation, behind the ⓘ beside the label (InfoTip). */
  info?: string;
  /** The control takes only its own width (a switch, a button). */
  wide?: boolean;
  children?: ReactNode;
}

export function Row({ label, sub, info, wide, children }: RowProps) {
  return (
    <div className={`dk-row${wide ? " wide" : ""}${sub ? " has-sub" : ""}`}>
      <span className="dk-label">
        {label}
        {info && <InfoTip text={info} />}
        {sub && <span className="dk-sub">{sub}</span>}
      </span>
      <div className="dk-ctl">{children}</div>
    </div>
  );
}

interface ToggleProps {
  label: string;
  sub?: string;
  info?: string;
  value: boolean;
  disabled?: boolean;
  onChange: (value: boolean) => void;
}

export function Toggle({ label, sub, info, value, disabled, onChange }: ToggleProps) {
  return (
    <Row label={label} sub={sub} info={info} wide>
      <button
        type="button"
        role="switch"
        aria-checked={value}
        aria-label={label}
        className={`dk-toggle${value ? " on" : ""}`}
        disabled={disabled}
        onClick={() => onChange(!value)}
      />
    </Row>
  );
}

interface SegmentedProps<T extends string> {
  label: string;
  sub?: string;
  info?: string;
  value: T;
  options: ReadonlyArray<{ value: T; label: string; title?: string }>;
  disabled?: boolean;
  onChange: (value: T) => void;
}

export function Segmented<T extends string>({ label, sub, info, value, options, disabled, onChange }: SegmentedProps<T>) {
  return (
    <Row label={label} sub={sub} info={info}>
      <div className="dk-seg" role="group" aria-label={label}>
        {/* loop: HOT-PATH-OK — N ≤ 6 segment options */}
        {options.map((o) => (
          <button
            type="button"
            key={o.value}
            title={o.title}
            className={o.value === value ? "on" : ""}
            aria-pressed={o.value === value}
            disabled={disabled}
            onClick={() => onChange(o.value)}
          >
            {o.label}
          </button>
        ))}
      </div>
    </Row>
  );
}

// The ⓘ beside a label (copied from the DS5 configurator): its text shows in
// a tooltip in the top layer on hover, keyboard focus or a click, and hides
// on leave, blur, Escape, scroll or resize. Screen readers get the text as the
// button's description at once.
export function InfoTip({ text }: { text: string }) {
  const id = useId();
  const btn = useRef<HTMLButtonElement>(null);
  const tip = useRef<HTMLSpanElement>(null);
  const [open, setOpen] = useState(false);
  const show = () => setOpen(true);
  const hide = () => setOpen(false);
  useTopLayer(tip, open, { anchor: btn, dx: -10 });
  // lifetime: OK — the effect returns the scroll/resize listeners' removal (closed / unmount)
  useEffect(() => {
    if (!open) return;
    // lifetime: OK — removed in the cleanup (closed / unmount)
    window.addEventListener("scroll", hide, true);
    // lifetime: OK — removed in the cleanup (closed / unmount)
    window.addEventListener("resize", hide);
    return () => {
      window.removeEventListener("scroll", hide, true);
      window.removeEventListener("resize", hide);
    };
  }, [open]);
  return (
    <>
      <button
        ref={btn}
        type="button"
        className={`dk-info${open ? " open" : ""}`}
        aria-label="More information"
        aria-describedby={id}
        onMouseEnter={show}
        onMouseLeave={hide}
        onFocus={show}
        onBlur={hide}
        onClick={(e) => {
          e.preventDefault(); // inside a label: not its own click
          e.stopPropagation();
          setOpen((o) => !o);
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape" && open) {
            e.stopPropagation(); // closes the tooltip, not the drawer around it
            hide();
          }
        }}
      >
        <Info size={13} aria-hidden="true" />
      </button>
      <span id={id} role="tooltip" hidden>
        {text}
      </span>
      {open && (
        <span ref={tip} className="dk-info-tip top-layer" aria-hidden="true">
          {text}
        </span>
      )}
    </>
  );
}
