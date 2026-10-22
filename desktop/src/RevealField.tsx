// A password field with an eye button. The rule for every secret field: a
// saved value shows only as "saved" (the placeholder), leaving the field blank
// keeps it, and the eye fetches the REAL saved value on click (never before).
// The value is in the DOM only while revealed; hiding drops it from state.
import "./dialog.css";
import { useEffect, useRef, useState } from "react";
import { Eye, EyeOff } from "lucide-react";
import { api } from "./api";

interface RevealFieldProps {
  value: string;
  onChange: (value: string) => void;
  /** The saved secret's id: with an empty value, the eye reveals it. */
  savedId?: string | null;
  placeholder?: string;
  "aria-label": string;
  disabled?: boolean;
  /** A reveal failure in plain words (null clears it). Without it, shown under the field. */
  onError?: (message: string | null) => void;
}

export default function RevealField({ value, onChange, savedId, placeholder, "aria-label": ariaLabel, disabled, onError }: RevealFieldProps) {
  // Showing what was typed (no fetch involved).
  const [showTyped, setShowTyped] = useState(false);
  // The saved value, fetched by an eye click; null when hidden.
  const [revealed, setRevealed] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Bumped on every hide / change, so a reveal that resolves late is dropped.
  const ticket = useRef(0);

  const report = (msg: string | null) => {
    if (onError) onError(msg);
    else setError(msg);
  };

  const hideSaved = () => {
    ticket.current += 1;
    setRevealed(null);
    setBusy(false);
  };

  // A different saved secret, or typing: whatever was revealed goes away.
  useEffect(() => {
    hideSaved();
  }, [savedId]);
  useEffect(() => {
    if (value !== "") hideSaved();
  }, [value]);

  const usesSaved = !!savedId && value === "";
  const shown = usesSaved ? revealed !== null : showTyped;

  const toggle = async () => {
    report(null);
    if (!usesSaved) {
      setShowTyped((s) => !s);
      return;
    }
    if (revealed !== null) {
      hideSaved();
      return;
    }
    const mine = ++ticket.current;
    setBusy(true);
    try {
      const v = await api.revealSecret(savedId!);
      if (mine !== ticket.current) return;
      if (v === null) report("The saved password could not be found.");
      else setRevealed(v);
    } catch {
      if (mine === ticket.current) report("The saved password could not be read.");
    } finally {
      if (mine === ticket.current) setBusy(false);
    }
  };

  const label = shown ? "Hide password" : usesSaved ? "Show saved password" : "Show password";

  return (
    <div className="dk-reveal">
      <div className="dk-reveal-field">
        <input
          className={`dk-input${revealed !== null ? " mono" : ""}`}
          type={shown ? "text" : "password"}
          aria-label={ariaLabel}
          value={revealed ?? value}
          readOnly={revealed !== null}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          autoComplete="off"
          spellCheck={false}
          disabled={disabled}
        />
        <button
          type="button"
          className="dk-btn dk-icon-btn"
          aria-label={label}
          title={label}
          aria-pressed={shown}
          aria-busy={busy}
          disabled={disabled || busy}
          onClick={toggle}
        >
          {shown ? <EyeOff size={14} aria-hidden="true" /> : <Eye size={14} aria-hidden="true" />}
        </button>
      </div>
      {!onError && error && (
        <span className="dk-inline-error" role="alert">
          {error}
        </span>
      )}
    </div>
  );
}
