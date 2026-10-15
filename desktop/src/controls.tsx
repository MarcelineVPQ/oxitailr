// The settings controls, carried over from the DS5 configurator's controls.tsx
// (Row, Toggle, Segmented): one look for every setting, label on the left,
// control on the right, an optional short hint under the label.
import type { ReactNode } from "react";

interface RowProps {
  label: string;
  /** A short hint under the label (keep it to one line). */
  sub?: string;
  /** The longer explanation, as the row's tooltip. */
  title?: string;
  /** The control takes only its own width (a switch, a button). */
  wide?: boolean;
  children?: ReactNode;
}

export function Row({ label, sub, title, wide, children }: RowProps) {
  return (
    <div className={`dk-row${wide ? " wide" : ""}${sub ? " has-sub" : ""}`} title={title}>
      <span className="dk-label">
        {label}
        {sub && <span className="dk-sub">{sub}</span>}
      </span>
      <div className="dk-ctl">{children}</div>
    </div>
  );
}

interface ToggleProps {
  label: string;
  sub?: string;
  title?: string;
  value: boolean;
  disabled?: boolean;
  onChange: (value: boolean) => void;
}

export function Toggle({ label, sub, title, value, disabled, onChange }: ToggleProps) {
  return (
    <Row label={label} sub={sub} title={title} wide>
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
  title?: string;
  value: T;
  options: ReadonlyArray<{ value: T; label: string; title?: string }>;
  disabled?: boolean;
  onChange: (value: T) => void;
}

export function Segmented<T extends string>({ label, sub, title, value, options, disabled, onChange }: SegmentedProps<T>) {
  return (
    <Row label={label} sub={sub} title={title}>
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
