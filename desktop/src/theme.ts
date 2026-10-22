// The window's theme: light, dark, or following the system. index.html sets
// data-theme before the first paint from the same key; from then on this owns
// it. tokens.css and shell.css switch on html[data-theme].
import { useEffect } from "react";
import { usePref } from "./storage";

export type Theme = "light" | "dark" | "system";
export const THEME_KEY = "oxi.theme";
export const THEMES: readonly Theme[] = ["light", "dark", "system"];

export function useTheme(): [Theme, (theme: Theme) => void] {
  const [theme, setTheme] = usePref<Theme>(THEME_KEY, "system", THEMES);
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);
  return [theme, setTheme];
}
